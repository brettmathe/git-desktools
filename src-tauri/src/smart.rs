//! Smart Environments and Smart Features.
//!
//! A Smart Environment is a user-defined set of local repos (anywhere on disk).
//! Smart Features (later phases) switch every repo of an environment between
//! feature branches in place, parking uncommitted work in pinned stashes.
//!
//! Storage, one file per environment so a bad write can only hurt one of them:
//!   <app_config_dir>/smart-features/environments.json   order of environments
//!   <app_config_dir>/smart-features/env-<id>.json       one environment
//! Git stays the source of truth for real state (branches, ahead/behind, stashes);
//! nothing here caches it. Which environment a repo belongs to is derived from the
//! environment files rather than stored twice.

use crate::git::{blocking, e, head_branch, name_of, root_of, unborn_branch, CmdResult};
use git2::{BranchType, Oid, Repository, RepositoryState, Status, StatusOptions};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::State;

const VERSION: u32 = 1;
const INDEX_FILE: &str = "environments.json";
const SCAN_DEPTH: usize = 3;
const SCAN_SKIP: &[&str] = &["node_modules", "target", "build", "dist", "Pods", "DerivedData"];
const BASE_CANDIDATES: &[&str] = &["develop", "main", "master"];
const STATUS_THREADS: usize = 8;

// ---------------------------------------------------------------------------
// Schema

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SmartRepo {
    #[serde(default)]
    pub id: String,
    pub path: String,
    #[serde(default)]
    pub label: String,
    /// None = resolve from origin/HEAD (then develop/main/master).
    #[serde(default)]
    pub base_branch: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct EnvSettings {
    /// One base (e.g. release/2.4) for every repo that has it.
    #[serde(default)]
    pub shared_base: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ParkedStash {
    #[serde(rename = "ref")]
    pub ref_name: String,
    pub sha: String,
    pub parked_at: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct FeatureRepo {
    #[serde(default)]
    pub parked_stash: Option<ParkedStash>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Feature {
    pub id: String,
    pub name: String,
    pub branch: String,
    pub created_at: i64,
    /// Repo id -> state, for the repos where the feature branch exists.
    #[serde(default)]
    pub repos: BTreeMap<String, FeatureRepo>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SmartEnv {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub settings: EnvSettings,
    #[serde(default)]
    pub repos: Vec<SmartRepo>,
    #[serde(default)]
    pub active_feature_id: Option<String>,
    #[serde(default)]
    pub features: Vec<Feature>,
    /// An unfinished feature switch (later phase); None when idle.
    #[serde(default)]
    pub journal: Option<serde_json::Value>,
}

impl SmartEnv {
    fn has_parked_work(&self) -> bool {
        self.features.iter().any(|f| f.repos.values().any(|r| r.parked_stash.is_some()))
    }
    fn repo_in_features(&self, repo_id: &str) -> bool {
        self.features.iter().any(|f| f.repos.contains_key(repo_id))
    }
}

#[derive(Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct Index {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    order: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvList {
    envs: Vec<SmartEnv>,
    warnings: Vec<String>,
}

// ---------------------------------------------------------------------------
// Store

pub struct SmartStore {
    dir: PathBuf,
    lock: Mutex<()>,
}

static ID_COUNTER: AtomicU64 = AtomicU64::new(0);

fn new_id(prefix: &str) -> String {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let n = ID_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{:x}{:x}", nanos & 0xffff_ffff_ffff, n)
}

/// Real path, so a repo reached through a symlink is still one repo.
fn canonical(path: &str) -> String {
    std::fs::canonicalize(path)
        .map(|p| p.to_string_lossy().trim_end_matches(['/', '\\']).to_string())
        .unwrap_or_else(|_| path.trim_end_matches(['/', '\\']).to_string())
}

fn write_atomic(path: &Path, value: &impl Serialize) -> CmdResult<()> {
    let text = serde_json::to_string_pretty(value).map_err(e)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text).map_err(e)?;
    if path.exists() {
        let _ = std::fs::copy(path, path.with_extension("json.bak"));
    }
    std::fs::rename(&tmp, path).map_err(e)
}

impl SmartStore {
    pub fn new(dir: PathBuf) -> Self {
        SmartStore { dir, lock: Mutex::new(()) }
    }

    fn env_path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("env-{id}.json"))
    }

    fn read_index(&self) -> Index {
        std::fs::read_to_string(self.dir.join(INDEX_FILE))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    fn read_env(&self, id: &str, warnings: &mut Vec<String>) -> Option<SmartEnv> {
        let path = self.env_path(id);
        let parse = |p: &Path| -> Option<SmartEnv> {
            serde_json::from_str(&std::fs::read_to_string(p).ok()?).ok()
        };
        let env = match parse(&path) {
            Some(env) => env,
            None => {
                let backup = parse(&path.with_extension("json.bak"));
                if path.exists() {
                    warnings.push(match &backup {
                        Some(b) => format!("{} was unreadable; loaded its backup for \"{}\".", path.display(), b.name),
                        None => format!("{} is unreadable and has no backup.", path.display()),
                    });
                }
                backup?
            }
        };
        if env.version > VERSION {
            warnings.push(format!(
                "Environment \"{}\" was saved by a newer git-desktools and is shown read-only.",
                env.name
            ));
        }
        Some(env)
    }

    /// Every environment, in display order. Env files missing from the index are
    /// adopted at the end so nothing on disk is ever silently hidden.
    pub fn load_all(&self) -> EnvList {
        let _guard = self.lock.lock().unwrap();
        self.load_all_locked()
    }

    fn load_all_locked(&self) -> EnvList {
        let mut warnings = Vec::new();
        let mut order = self.read_index().order;
        if let Ok(entries) = std::fs::read_dir(&self.dir) {
            let mut extra: Vec<String> = entries
                .flatten()
                .filter_map(|d| {
                    let name = d.file_name().to_string_lossy().to_string();
                    name.strip_prefix("env-")?.strip_suffix(".json").map(str::to_string)
                })
                .filter(|id| !order.contains(id))
                .collect();
            extra.sort();
            order.extend(extra);
        }
        let envs = order.iter().filter_map(|id| self.read_env(id, &mut warnings)).collect();
        EnvList { envs, warnings }
    }

    /// Create or update an environment. Enforces: a name, no duplicate repos, and
    /// a repo belongs to only one environment.
    pub fn save(&self, mut env: SmartEnv) -> CmdResult<SmartEnv> {
        let _guard = self.lock.lock().unwrap();
        env.name = env.name.trim().to_string();
        if env.name.is_empty() {
            return Err("Give the environment a name.".into());
        }
        let others: Vec<SmartEnv> = self
            .load_all_locked()
            .envs
            .into_iter()
            .filter(|o| env.id.is_empty() || o.id != env.id)
            .collect();
        if others.iter().any(|o| o.name.eq_ignore_ascii_case(&env.name)) {
            return Err(format!("An environment named \"{}\" already exists.", env.name));
        }
        if let Some(previous) = self.load_all_locked().envs.into_iter().find(|o| !env.id.is_empty() && o.id == env.id) {
            if previous.version > VERSION {
                return Err(format!("\"{}\" was saved by a newer git-desktools; update the app to edit it.", previous.name));
            }
            // Features and their state are owned by the feature commands, never by an edit.
            env.features = previous.features.clone();
            env.active_feature_id = previous.active_feature_id.clone();
            env.journal = previous.journal.clone();
            for removed in previous.repos.iter().filter(|r| !env.repos.iter().any(|n| n.id == r.id)) {
                if previous.repo_in_features(&removed.id) {
                    return Err(format!(
                        "{} has feature branches or parked changes; finish those features before removing it.",
                        removed.label
                    ));
                }
            }
        }

        let taken: HashMap<String, &str> = others
            .iter()
            .flat_map(|o| o.repos.iter().map(move |r| (canonical(&r.path), o.name.as_str())))
            .collect();
        let mut seen = HashSet::new();
        for repo in &mut env.repos {
            repo.path = canonical(&repo.path);
            if repo.id.is_empty() {
                repo.id = new_id("r");
            }
            if repo.label.trim().is_empty() {
                repo.label = name_of(&repo.path);
            }
            repo.base_branch = repo.base_branch.take().map(|b| b.trim().to_string()).filter(|b| !b.is_empty());
            if !seen.insert(repo.path.clone()) {
                return Err(format!("{} is in the list twice.", repo.path));
            }
            if let Some(other) = taken.get(&repo.path) {
                return Err(format!("{} already belongs to the \"{other}\" environment.", repo.label));
            }
        }
        env.settings.shared_base =
            env.settings.shared_base.take().map(|b| b.trim().to_string()).filter(|b| !b.is_empty());
        if env.id.is_empty() {
            env.id = new_id("e");
        }
        env.version = VERSION;

        std::fs::create_dir_all(&self.dir).map_err(e)?;
        write_atomic(&self.env_path(&env.id), &env)?;
        let mut index = self.read_index();
        if !index.order.contains(&env.id) {
            index.order.push(env.id.clone());
        }
        index.version = VERSION;
        write_atomic(&self.dir.join(INDEX_FILE), &index)?;
        Ok(env)
    }

    /// Forget an environment. Never touches the repos themselves.
    pub fn delete(&self, id: &str) -> CmdResult<()> {
        let _guard = self.lock.lock().unwrap();
        let mut warnings = Vec::new();
        if let Some(env) = self.read_env(id, &mut warnings) {
            if env.has_parked_work() || env.active_feature_id.is_some() || env.journal.is_some() {
                return Err(format!(
                    "\"{}\" has an active feature or parked changes; finish or go back to base first.",
                    env.name
                ));
            }
        }
        let path = self.env_path(id);
        if path.exists() {
            std::fs::remove_file(&path).map_err(e)?;
        }
        let _ = std::fs::remove_file(path.with_extension("json.bak"));
        let mut index = self.read_index();
        index.order.retain(|x| x != id);
        if self.dir.exists() {
            write_atomic(&self.dir.join(INDEX_FILE), &index)?;
        }
        Ok(())
    }

    /// Name of the environment that already holds `path`, ignoring `except_env`.
    fn owner_of(&self, path: &str, except_env: Option<&str>) -> Option<String> {
        let target = canonical(path);
        self.load_all()
            .envs
            .into_iter()
            .filter(|env| Some(env.id.as_str()) != except_env)
            .find(|env| env.repos.iter().any(|r| canonical(&r.path) == target))
            .map(|env| env.name)
    }
}

// ---------------------------------------------------------------------------
// Base branch

fn branch_exists(repo: &Repository, name: &str) -> bool {
    repo.find_branch(name, BranchType::Local).is_ok()
        || repo.find_reference(&format!("refs/remotes/origin/{name}")).is_ok()
}

/// Override, then the shared base when this repo has it, then origin/HEAD, then
/// the first of develop/main/master that exists.
pub fn resolve_base(repo: &Repository, override_: Option<&str>, shared: Option<&str>) -> Option<String> {
    if let Some(b) = override_ {
        return Some(b.to_string());
    }
    if let Some(b) = shared.filter(|b| branch_exists(repo, b)) {
        return Some(b.to_string());
    }
    let from_origin_head = repo
        .find_reference("refs/remotes/origin/HEAD")
        .ok()
        .and_then(|r| r.symbolic_target().ok().flatten().map(str::to_string))
        .and_then(|t| t.strip_prefix("refs/remotes/origin/").map(str::to_string));
    if from_origin_head.is_some() {
        return from_origin_head;
    }
    BASE_CANDIDATES.iter().find(|b| branch_exists(repo, b)).map(|b| b.to_string())
}

/// Tip to measure "ahead of base" against: origin/<base>, else the local base.
fn base_tip(repo: &Repository, base: &str) -> Option<Oid> {
    repo.find_reference(&format!("refs/remotes/origin/{base}"))
        .ok()
        .and_then(|r| r.target())
        .or_else(|| repo.find_branch(base, BranchType::Local).ok().and_then(|b| b.get().target()))
}

// ---------------------------------------------------------------------------
// Adding repos

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoCheck {
    root: String,
    name: String,
    branch: Option<String>,
    base_branch: Option<String>,
    /// Name of another environment that already holds this repo.
    in_env: Option<String>,
}

fn check_repo(store: &SmartStore, path: &str, env_id: Option<&str>) -> CmdResult<RepoCheck> {
    let repo = Repository::discover(path).map_err(|_| format!("{path} is not inside a git repository"))?;
    if repo.is_bare() {
        return Err("Bare repositories have no working files, so they can't be in an environment.".into());
    }
    let root = canonical(&root_of(&repo));
    Ok(RepoCheck {
        name: name_of(&root),
        branch: head_branch(&repo).or_else(|| unborn_branch(&repo)),
        base_branch: resolve_base(&repo, None, None),
        in_env: store.owner_of(&root, env_id),
        root,
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanHit {
    root: String,
    name: String,
    in_env: Option<String>,
}

fn is_repo_dir(dir: &Path) -> bool {
    dir.join(".git").exists()
}

/// Git repos at or under `root`, up to `depth` folders down. Doesn't descend
/// into a repo (nested repos and submodules belong to it), hidden folders,
/// common build/dependency folders, or symlinks (their target is found directly).
fn scan(root: &Path, depth: usize, out: &mut Vec<PathBuf>, seen: &mut HashSet<PathBuf>) {
    let Ok(real) = std::fs::canonicalize(root) else { return };
    if !seen.insert(real.clone()) {
        return;
    }
    if is_repo_dir(&real) {
        out.push(real);
        return;
    }
    if depth == 0 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(&real) else { return };
    let mut dirs: Vec<PathBuf> = entries
        .flatten()
        .filter(|d| d.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .filter(|d| {
            let name = d.file_name().to_string_lossy().to_string();
            !name.starts_with('.') && !SCAN_SKIP.contains(&name.as_str())
        })
        .map(|d| d.path())
        .collect();
    dirs.sort();
    for d in dirs {
        scan(&d, depth - 1, out, seen);
    }
}

// ---------------------------------------------------------------------------
// Status

#[derive(Serialize, Default, Debug)]
#[serde(rename_all = "camelCase")]
pub struct RepoStatus {
    repo_id: String,
    path: String,
    label: String,
    missing: bool,
    branch: Option<String>,
    detached: bool,
    base_branch: Option<String>,
    /// The branch this repo should be on: the active feature's if it has one, else base.
    expected_branch: Option<String>,
    staged: usize,
    unstaged: usize,
    untracked: usize,
    conflicted: usize,
    has_upstream: bool,
    ahead: usize,
    behind: usize,
    ahead_of_base: usize,
    in_progress: Option<&'static str>,
    /// "red" | "yellow" | "green" | "none"
    dot: &'static str,
    warning: Option<String>,
}

fn in_progress(state: RepositoryState) -> Option<&'static str> {
    match state {
        RepositoryState::Clean => None,
        RepositoryState::Merge => Some("merge"),
        RepositoryState::Revert | RepositoryState::RevertSequence => Some("revert"),
        RepositoryState::CherryPick | RepositoryState::CherryPickSequence => Some("cherry-pick"),
        RepositoryState::Bisect => Some("bisect"),
        RepositoryState::Rebase | RepositoryState::RebaseInteractive | RepositoryState::RebaseMerge => {
            Some("rebase")
        }
        RepositoryState::ApplyMailbox | RepositoryState::ApplyMailboxOrRebase => Some("am"),
    }
}

fn repo_status(env: &SmartEnv, r: &SmartRepo) -> RepoStatus {
    let mut s = RepoStatus { repo_id: r.id.clone(), path: r.path.clone(), label: r.label.clone(), dot: "none", ..Default::default() };
    if !Path::new(&r.path).exists() {
        s.missing = true;
        s.warning = Some("Folder not found. It may have been moved or deleted.".into());
        return s;
    }
    let repo = match Repository::open(&r.path) {
        Ok(repo) => repo,
        Err(err) => {
            s.warning = Some(format!("Can't open repository: {}", err.message()));
            return s;
        }
    };

    s.base_branch = resolve_base(&repo, r.base_branch.as_deref(), env.settings.shared_base.as_deref());
    let feature_branch = env
        .active_feature_id
        .as_ref()
        .and_then(|id| env.features.iter().find(|f| &f.id == id))
        .filter(|f| f.repos.contains_key(&r.id))
        .map(|f| f.branch.clone());
    s.expected_branch = feature_branch.or_else(|| s.base_branch.clone());
    s.detached = repo.head_detached().unwrap_or(false);
    s.branch = head_branch(&repo).or_else(|| unborn_branch(&repo));
    s.in_progress = in_progress(repo.state());

    let mut opts = StatusOptions::new();
    opts.include_untracked(true).recurse_untracked_dirs(true).exclude_submodules(true);
    if let Ok(statuses) = repo.statuses(Some(&mut opts)) {
        for entry in statuses.iter() {
            let st = entry.status();
            if st.contains(Status::CONFLICTED) {
                s.conflicted += 1;
                continue;
            }
            if st.intersects(
                Status::INDEX_NEW
                    | Status::INDEX_MODIFIED
                    | Status::INDEX_DELETED
                    | Status::INDEX_RENAMED
                    | Status::INDEX_TYPECHANGE,
            ) {
                s.staged += 1;
            }
            if st.intersects(Status::WT_MODIFIED | Status::WT_DELETED | Status::WT_TYPECHANGE | Status::WT_RENAMED) {
                s.unstaged += 1;
            }
            if st.contains(Status::WT_NEW) {
                s.untracked += 1;
            }
        }
    }

    let head = repo.head().ok().and_then(|h| h.target());
    if let (Some(head), Some(branch)) = (head, s.branch.as_deref().filter(|_| !s.detached)) {
        if let Ok(b) = repo.find_branch(branch, BranchType::Local) {
            if let Some(up) = b.upstream().ok().and_then(|u| u.get().target()) {
                s.has_upstream = true;
                (s.ahead, s.behind) = repo.graph_ahead_behind(head, up).unwrap_or((0, 0));
            }
        }
        let on_base = s.base_branch.as_deref() == Some(branch);
        if !on_base {
            if let Some(tip) = s.base_branch.as_deref().and_then(|b| base_tip(&repo, b)) {
                s.ahead_of_base = repo.graph_ahead_behind(head, tip).map(|(a, _)| a).unwrap_or(0);
            }
        }
    }

    let dirty = s.staged + s.unstaged + s.untracked + s.conflicted > 0;
    let unpushed = if s.has_upstream {
        s.ahead > 0
    } else {
        s.ahead_of_base > 0
    };
    s.dot = if dirty {
        "red"
    } else if unpushed {
        "yellow"
    } else {
        "green"
    };

    s.warning = if let Some(op) = s.in_progress {
        Some(format!("A {op} is in progress."))
    } else if s.detached {
        Some("Detached HEAD (not on a branch).".into())
    } else if s.base_branch.is_none() {
        Some("No base branch found. Set one in Edit environment.".into())
    } else if s.branch.is_some() && s.branch != s.expected_branch {
        Some(format!(
            "On {}, expected {}.",
            s.branch.as_deref().unwrap_or("?"),
            s.expected_branch.as_deref().unwrap_or("?")
        ))
    } else {
        None
    };
    s
}

/// Status of every repo in `env`, read in parallel.
pub fn env_status(env: &SmartEnv) -> Vec<RepoStatus> {
    let mut out = Vec::with_capacity(env.repos.len());
    for chunk in env.repos.chunks(STATUS_THREADS) {
        std::thread::scope(|scope| {
            let handles: Vec<_> = chunk.iter().map(|r| scope.spawn(|| repo_status(env, r))).collect();
            for h in handles {
                out.push(h.join().unwrap_or_default());
            }
        });
    }
    out
}

// ---------------------------------------------------------------------------
// Commands

#[tauri::command]
pub async fn sf_list_envs(store: State<'_, SmartStore>) -> CmdResult<EnvList> {
    Ok(store.load_all())
}

#[tauri::command]
pub async fn sf_save_env(store: State<'_, SmartStore>, env: SmartEnv) -> CmdResult<SmartEnv> {
    store.save(env)
}

#[tauri::command]
pub async fn sf_delete_env(store: State<'_, SmartStore>, id: String) -> CmdResult<()> {
    store.delete(&id)
}

#[tauri::command]
pub async fn sf_validate_repo(store: State<'_, SmartStore>, path: String, env_id: Option<String>) -> CmdResult<RepoCheck> {
    check_repo(&store, &path, env_id.as_deref())
}

#[tauri::command]
pub async fn sf_scan_folder(store: State<'_, SmartStore>, path: String, env_id: Option<String>) -> CmdResult<Vec<ScanHit>> {
    let roots = blocking(move || {
        let mut out = Vec::new();
        scan(Path::new(&path), SCAN_DEPTH, &mut out, &mut HashSet::new());
        Ok(out)
    })
    .await?;
    Ok(roots
        .into_iter()
        .map(|p| {
            let root = p.to_string_lossy().to_string();
            ScanHit { name: name_of(&root), in_env: store.owner_of(&root, env_id.as_deref()), root }
        })
        .collect())
}

#[tauri::command]
pub async fn sf_status(store: State<'_, SmartStore>, env_id: String) -> CmdResult<Vec<RepoStatus>> {
    let env = store
        .load_all()
        .envs
        .into_iter()
        .find(|env| env.id == env_id)
        .ok_or_else(|| "That environment no longer exists.".to_string())?;
    blocking(move || Ok(env_status(&env))).await
}

// Plain-function entry points for the dev bridge (no tauri::State there).
#[cfg(test)]
pub mod bridge {
    use super::*;

    pub fn list(store: &SmartStore) -> EnvList {
        store.load_all()
    }
    pub fn validate(store: &SmartStore, path: &str, env_id: Option<&str>) -> CmdResult<RepoCheck> {
        check_repo(store, path, env_id)
    }
    pub fn scan_folder(store: &SmartStore, path: &str, env_id: Option<&str>) -> Vec<ScanHit> {
        let mut out = Vec::new();
        scan(Path::new(path), SCAN_DEPTH, &mut out, &mut HashSet::new());
        out.into_iter()
            .map(|p| {
                let root = p.to_string_lossy().to_string();
                ScanHit { name: name_of(&root), in_env: store.owner_of(&root, env_id), root }
            })
            .collect()
    }
    pub fn status(store: &SmartStore, env_id: &str) -> CmdResult<Vec<RepoStatus>> {
        let env = store
            .load_all()
            .envs
            .into_iter()
            .find(|env| env.id == env_id)
            .ok_or_else(|| "That environment no longer exists.".to_string())?;
        Ok(env_status(&env))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    /// A fresh scratch directory under the system temp dir, removed on drop.
    struct Scratch(PathBuf);
    impl Scratch {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("gdt-smart-{tag}-{}", new_id("t")));
            std::fs::create_dir_all(&dir).unwrap();
            Scratch(std::fs::canonicalize(dir).unwrap())
        }
        fn path(&self, rel: &str) -> PathBuf {
            self.0.join(rel)
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn git(dir: &Path, args: &[&str]) {
        let out = Command::new("git")
            .current_dir(dir)
            .args(["-c", "user.name=T", "-c", "user.email=t@t", "-c", "init.defaultBranch=main"])
            .args(args)
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    }

    /// A repo with one commit on `base`, cloned from a bare origin so it has
    /// origin/HEAD and an upstream.
    fn repo_with_origin(s: &Scratch, name: &str, base: &str) -> PathBuf {
        let bare = s.path(&format!("remotes/{name}.git"));
        let seed = s.path(&format!("seed/{name}"));
        std::fs::create_dir_all(&bare).unwrap();
        std::fs::create_dir_all(&seed).unwrap();
        git(&bare, &["init", "--bare", "-b", base]);
        git(&seed, &["init", "-b", base]);
        std::fs::write(seed.join("a.txt"), "a\n").unwrap();
        git(&seed, &["add", "."]);
        git(&seed, &["commit", "-m", "init"]);
        git(&seed, &["push", bare.to_str().unwrap(), base]);
        let work = s.path(&format!("work/{name}"));
        git(&s.0, &["clone", bare.to_str().unwrap(), work.to_str().unwrap()]);
        work
    }

    fn env_with(name: &str, paths: &[&Path]) -> SmartEnv {
        SmartEnv {
            version: 0,
            id: String::new(),
            name: name.into(),
            settings: EnvSettings::default(),
            repos: paths
                .iter()
                .map(|p| SmartRepo { id: String::new(), path: p.to_string_lossy().into(), label: String::new(), base_branch: None })
                .collect(),
            active_feature_id: None,
            features: vec![],
            journal: None,
        }
    }

    #[test]
    #[cfg(unix)]
    fn save_load_and_membership() {
        let s = Scratch::new("store");
        let a = repo_with_origin(&s, "a", "main");
        let b = repo_with_origin(&s, "b", "develop");
        let store = SmartStore::new(s.path("config/smart-features"));

        let saved = store.save(env_with("RGT", &[&a, &b])).unwrap();
        assert!(saved.id.starts_with("e_"));
        assert_eq!(saved.repos[0].label, "a");
        assert!(saved.repos.iter().all(|r| r.id.starts_with("r_")));

        // Survives a reload, in order.
        let list = store.load_all();
        assert_eq!(list.envs.len(), 1);
        assert_eq!(list.envs[0].repos.len(), 2);

        // A repo can only be in one environment, even via a symlinked path.
        let link = s.path("link-to-a");
        std::os::unix::fs::symlink(&a, &link).unwrap();
        let err = store.save(env_with("Other", &[&link])).unwrap_err();
        assert!(err.contains("already belongs to the \"RGT\""), "{err}");

        // Names are unique, case-insensitively.
        assert!(store.save(env_with("rgt", &[])).unwrap_err().contains("already exists"));

        // Editing keeps ids; deleting forgets the environment only.
        let mut edited = saved.clone();
        edited.repos.truncate(1);
        let edited = store.save(edited).unwrap();
        assert_eq!(edited.id, saved.id);
        assert_eq!(edited.repos[0].id, saved.repos[0].id);
        store.delete(&saved.id).unwrap();
        assert!(store.load_all().envs.is_empty());
        assert!(a.join(".git").exists());
    }

    #[test]
    fn edits_never_overwrite_feature_state() {
        let s = Scratch::new("features");
        let a = repo_with_origin(&s, "a", "main");
        let store = SmartStore::new(s.path("cfg"));
        let mut saved = store.save(env_with("E", &[&a])).unwrap();
        // Simulate state written by the feature commands.
        saved.features.push(Feature {
            id: "f1".into(),
            name: "x".into(),
            branch: "x".into(),
            created_at: 0,
            repos: BTreeMap::from([(saved.repos[0].id.clone(), FeatureRepo::default())]),
        });
        write_atomic(&store.env_path(&saved.id), &saved).unwrap();

        // An edit from the dialog arrives without features; they must survive.
        let mut edit = saved.clone();
        edit.features.clear();
        edit.name = "E2".into();
        let out = store.save(edit).unwrap();
        assert_eq!(out.features.len(), 1);

        // And a repo with a feature branch can't be removed.
        let mut remove = out.clone();
        remove.repos.clear();
        assert!(store.save(remove).unwrap_err().contains("finish those features"));
    }

    #[test]
    fn corrupt_file_falls_back_to_backup() {
        let s = Scratch::new("bak");
        let store = SmartStore::new(s.path("cfg"));
        let env = store.save(env_with("E", &[])).unwrap();
        store.save(SmartEnv { name: "E renamed".into(), ..env.clone() }).unwrap();
        std::fs::write(store.env_path(&env.id), "{not json").unwrap();
        let list = store.load_all();
        assert_eq!(list.envs[0].name, "E");
        assert_eq!(list.warnings.len(), 1);
    }

    #[test]
    fn base_resolution() {
        let s = Scratch::new("base");
        let a = repo_with_origin(&s, "a", "develop");
        let repo = Repository::open(&a).unwrap();
        assert_eq!(resolve_base(&repo, None, None).as_deref(), Some("develop"));
        assert_eq!(resolve_base(&repo, Some("trunk"), None).as_deref(), Some("trunk"));
        // A shared base applies only where it exists.
        assert_eq!(resolve_base(&repo, None, Some("release/9")).as_deref(), Some("develop"));
        git(&a, &["branch", "release/9"]);
        assert_eq!(resolve_base(&repo, None, Some("release/9")).as_deref(), Some("release/9"));

        // No remote: falls back to develop/main/master.
        let local = s.path("local");
        std::fs::create_dir_all(&local).unwrap();
        git(&local, &["init", "-b", "master"]);
        std::fs::write(local.join("f"), "x").unwrap();
        git(&local, &["add", "."]);
        git(&local, &["commit", "-m", "x"]);
        assert_eq!(resolve_base(&Repository::open(&local).unwrap(), None, None).as_deref(), Some("master"));
    }

    #[test]
    fn status_dots() {
        let s = Scratch::new("status");
        let clean = repo_with_origin(&s, "clean", "main");
        let dirty = repo_with_origin(&s, "dirty", "main");
        let ahead = repo_with_origin(&s, "ahead", "main");
        let branch = repo_with_origin(&s, "branch", "main");

        std::fs::write(dirty.join("a.txt"), "changed\n").unwrap();
        std::fs::write(dirty.join("new.txt"), "new\n").unwrap();
        std::fs::write(ahead.join("a.txt"), "local\n").unwrap();
        git(&ahead, &["commit", "-am", "local"]);
        // A feature-ish branch with a commit and no upstream yet.
        git(&branch, &["switch", "-c", "feature/x"]);
        std::fs::write(branch.join("b.txt"), "b\n").unwrap();
        git(&branch, &["add", "."]);
        git(&branch, &["commit", "-m", "b"]);

        let mut env = env_with("E", &[&clean, &dirty, &ahead, &branch]);
        for r in &mut env.repos {
            r.label = name_of(&r.path);
        }
        env.repos.push(SmartRepo {
            id: "gone".into(),
            path: s.path("nope").to_string_lossy().into(),
            label: "gone".into(),
            base_branch: None,
        });
        let st = env_status(&env);
        let by = |l: &str| st.iter().find(|x| x.label == l).unwrap();

        assert_eq!(by("clean").dot, "green");
        assert!(by("clean").warning.is_none());
        assert_eq!(by("dirty").dot, "red");
        assert_eq!((by("dirty").unstaged, by("dirty").untracked), (1, 1));
        assert_eq!(by("ahead").dot, "yellow");
        assert_eq!(by("ahead").ahead, 1);
        assert_eq!(by("branch").dot, "yellow");
        assert_eq!(by("branch").ahead_of_base, 1);
        assert!(by("branch").warning.as_deref().unwrap().contains("On feature/x, expected main"));
        assert!(by("gone").missing);
        assert_eq!(by("gone").dot, "none");
    }

    #[test]
    #[cfg(unix)]
    fn scan_finds_repos_once() {
        let s = Scratch::new("scan");
        let a = repo_with_origin(&s, "a", "main");
        let _b = repo_with_origin(&s, "b", "main");
        // Skipped: hidden folders, node_modules, and symlinks to repos already found.
        std::fs::create_dir_all(s.path("work/node_modules/pkg/.git")).unwrap();
        std::fs::create_dir_all(s.path("work/.hidden/repo/.git")).unwrap();
        std::os::unix::fs::symlink(&a, s.path("work/a-link")).unwrap();
        let mut out = Vec::new();
        scan(&s.path("work"), SCAN_DEPTH, &mut out, &mut HashSet::new());
        let names: Vec<String> = out.iter().map(|p| name_of(&p.to_string_lossy())).collect();
        assert_eq!(names, vec!["a", "b"]);
    }
}
