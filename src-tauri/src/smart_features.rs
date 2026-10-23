//! Smart Features: starting a feature, moving changed repos onto its branch
//! (auto-branch), the commit guard, Back to base, Push all and Merge base.
//!
//! The rule: a repo's feature branch is created only when a file in that repo
//! changes. Starting a feature creates no branches; the watcher (smart_watch.rs)
//! and a periodic sweep call `auto_branch` for repos that are dirty on base.
//! Creating a branch at the current commit never touches the working tree, so
//! it's safe while an editor is still writing files.

use crate::git::{
    blocking, head_branch, is_dirty, now_secs, run_git, CmdResult, GitRun, NetLocks, BUSY, MANUAL_FETCH_TIMEOUT,
};
use crate::smart_switch::switch_locked;
use crate::smart::{base_tip, new_id, resolve_base, Feature, FeatureRepo, SmartEnv, SmartRepo, SmartStore};
use git2::{BranchType, Repository, RepositoryState};
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::Path;
use tauri::{AppHandle, Manager, State};

const PUSH_THREADS: usize = 4;

pub(crate) fn git(cwd: &Path, args: &[&str]) -> CmdResult<()> {
    run_git(cwd, GitRun { args: args.iter().map(|s| s.to_string()).collect(), ..Default::default() })
}

fn open_repo(r: &SmartRepo) -> CmdResult<Repository> {
    Repository::open(&r.path).map_err(|_| format!("{}: can't open the repository at {}", r.label, r.path))
}

pub(crate) fn base_of(env: &SmartEnv, r: &SmartRepo, repo: &Repository) -> Option<String> {
    resolve_base(repo, r.base_branch.as_deref(), env.settings.shared_base.as_deref())
}

pub(crate) fn local_branch_exists(repo: &Repository, name: &str) -> bool {
    repo.find_branch(name, BranchType::Local).is_ok()
}

// ---------------------------------------------------------------------------
// Branch name checks

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct BranchRepo {
    repo_id: String,
    label: String,
    local: bool,
    /// origin/<branch> exists.
    remote: bool,
    /// Uncommitted changes on base: these move to the branch as soon as the feature starts.
    dirty_on_base: bool,
    /// Why the branch can't be created here (a/b vs a).
    conflict: Option<String>,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct BranchCheck {
    /// Blocks starting the feature.
    error: Option<String>,
    repos: Vec<BranchRepo>,
}

/// `git check-ref-format --branch` is the authority on legal names; the
/// frontend's regex is only the first gate.
fn ref_format_ok(cwd: &Path, branch: &str) -> bool {
    git(cwd, &["check-ref-format", "--branch", branch]).is_ok()
}

/// A branch `a/b` can't coexist with a branch `a`, and vice versa, because refs are files.
fn ref_conflict(repo: &Repository, branch: &str) -> Option<String> {
    let branches = repo.branches(Some(BranchType::Local)).ok()?;
    for (b, _) in branches.flatten() {
        let Ok(Some(name)) = b.name() else { continue };
        if name.starts_with(&format!("{branch}/")) {
            return Some(format!("branch \"{name}\" exists, so \"{branch}\" can't be created"));
        }
        if branch.starts_with(&format!("{name}/")) {
            return Some(format!("branch \"{name}\" exists, so \"{branch}\" can't be created"));
        }
    }
    None
}

pub fn check_branch(env: &SmartEnv, branch: &str) -> CmdResult<BranchCheck> {
    let branch = branch.trim();
    let mut error = None;
    let cwd = env.repos.iter().map(|r| Path::new(&r.path)).find(|p| p.exists()).map(Path::to_path_buf);
    let cwd = cwd.unwrap_or_else(std::env::temp_dir);
    if branch.is_empty() {
        error = Some("Type a branch name.".to_string());
    } else if branch == "@" || !ref_format_ok(&cwd, branch) {
        error = Some(format!("git doesn't accept \"{branch}\" as a branch name."));
    } else if let Some(f) = env.features.iter().find(|f| f.branch == branch) {
        error = Some(format!("The feature \"{}\" already uses {branch}.", f.name));
    }

    let mut repos = Vec::new();
    for r in &env.repos {
        let Ok(repo) = Repository::open(&r.path) else { continue };
        let base = base_of(env, r, &repo);
        if error.is_none() && base.as_deref() == Some(branch) {
            error = Some(format!("{branch} is the base branch of {}.", r.label));
        }
        let local = local_branch_exists(&repo, branch);
        let remote = repo.find_reference(&format!("refs/remotes/origin/{branch}")).is_ok();
        let on_base = base.is_some() && head_branch(&repo) == base;
        repos.push(BranchRepo {
            repo_id: r.id.clone(),
            label: r.label.clone(),
            local,
            remote,
            dirty_on_base: on_base && is_dirty(&repo),
            conflict: if local { None } else { ref_conflict(&repo, branch) },
        });
    }
    if error.is_none() {
        let blocked: Vec<String> =
            repos.iter().filter_map(|r| r.conflict.as_ref().map(|c| format!("{}: {c}", r.label))).collect();
        if !blocked.is_empty() {
            error = Some(blocked.join("\n"));
        }
    }
    Ok(BranchCheck { error, repos })
}

// ---------------------------------------------------------------------------
// Auto-branch

#[derive(Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AutoBranched {
    pub env_id: String,
    pub repo_id: String,
    pub label: String,
    pub branch: String,
    /// false: an existing local branch was checked out (or the repo was already on it).
    pub created: bool,
}

/// Move one repo onto the active feature's branch if it has uncommitted changes
/// on its base branch (`force`: even if it's clean, for the commit guard).
/// Returns what happened, or None when there was nothing to do.
pub fn auto_branch(store: &SmartStore, env_id: &str, repo_id: &str, force: bool) -> CmdResult<Option<AutoBranched>> {
    let _op = store.op_lock();
    auto_branch_locked(store, env_id, repo_id, force)
}

/// `auto_branch` for callers that already hold `op_lock`.
pub(crate) fn auto_branch_locked(
    store: &SmartStore,
    env_id: &str,
    repo_id: &str,
    force: bool,
) -> CmdResult<Option<AutoBranched>> {
    let env = store.get(env_id)?;
    if env.journal.is_some() {
        return Ok(None); // a switch is running or needs recovery
    }
    let Some(feature) = env.active_feature().cloned() else { return Ok(None) };
    let r = env.repo(repo_id)?.clone();
    let Ok(repo) = Repository::open(&r.path) else { return Ok(None) };
    if repo.state() != RepositoryState::Clean || repo.head_detached().unwrap_or(false) {
        return Ok(None);
    }
    let current = head_branch(&repo);
    let done = |created: bool| AutoBranched {
        env_id: env.id.clone(),
        repo_id: r.id.clone(),
        label: r.label.clone(),
        branch: feature.branch.clone(),
        created,
    };

    // Already on the feature branch (checked out by hand): just record it.
    if current.as_deref() == Some(feature.branch.as_str()) {
        if feature.repos.contains_key(&r.id) {
            return Ok(None);
        }
        record(store, env_id, &feature.id, &r.id)?;
        return Ok(Some(done(false)));
    }
    let base = base_of(&env, &r, &repo);
    if current.is_none() || current != base {
        return Ok(None);
    }
    if !force && !is_dirty(&repo) {
        return Ok(None);
    }

    let cwd = Path::new(&r.path);
    let created = if local_branch_exists(&repo, &feature.branch) {
        // `git switch` carries the changes over, and refuses if it would overwrite any.
        git(cwd, &["switch", &feature.branch]).map_err(|err| {
            format!("{}: couldn't move the changes onto {}.\n{err}", r.label, feature.branch)
        })?;
        false
    } else {
        // From the current commit, never from origin/<branch>: switching a dirty
        // tree onto someone else's commits could conflict.
        git(cwd, &["switch", "--no-track", "-c", &feature.branch])
            .map_err(|err| format!("{}: couldn't create {}.\n{err}", r.label, feature.branch))?;
        true
    };
    record(store, env_id, &feature.id, &r.id)?;
    Ok(Some(done(created)))
}

fn record(store: &SmartStore, env_id: &str, feature_id: &str, repo_id: &str) -> CmdResult<()> {
    store.update(env_id, |env| {
        if let Some(f) = env.features.iter_mut().find(|f| f.id == feature_id) {
            f.repos.entry(repo_id.to_string()).or_insert_with(FeatureRepo::default);
        }
        Ok(())
    })?;
    Ok(())
}

/// Auto-branch every repo of every environment that has an active feature.
/// The watcher's safety net for changes it missed.
/// Returns (env id, repo id, result) for each repo that was moved or failed.
pub fn sweep(store: &SmartStore) -> Vec<(String, String, CmdResult<AutoBranched>)> {
    let mut out = Vec::new();
    for env in store.load_all().envs.into_iter().filter(|e| e.active_feature_id.is_some() && e.journal.is_none()) {
        for r in &env.repos {
            match auto_branch(store, &env.id, &r.id, false) {
                Ok(Some(done)) => out.push((env.id.clone(), r.id.clone(), Ok(done))),
                Ok(None) => {}
                Err(err) => out.push((env.id.clone(), r.id.clone(), Err(err))),
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Commit guard

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CommitGuard {
    env_id: String,
    env_name: String,
    repo_id: String,
    feature_name: String,
    branch: String,
    base: String,
}

/// Some(..) when a commit in `path` would land on base while its environment
/// has an active feature.
pub fn commit_guard(store: &SmartStore, path: &str) -> Option<CommitGuard> {
    let (env, r) = store.find_repo(path)?;
    let feature = env.active_feature()?;
    let repo = Repository::open(&r.path).ok()?;
    let base = base_of(&env, &r, &repo)?;
    (head_branch(&repo).as_deref() == Some(base.as_str())).then(|| CommitGuard {
        env_id: env.id.clone(),
        env_name: env.name.clone(),
        repo_id: r.id.clone(),
        feature_name: feature.name.clone(),
        branch: feature.branch.clone(),
        base,
    })
}

/// Some(message) while the repo's environment is mid-switch (or needs recovery).
pub fn switch_running(store: &SmartStore, path: &str) -> Option<String> {
    let (env, _) = store.find_repo(path)?;
    env.journal.is_some().then(|| {
        format!("A feature switch in {} is running or didn't finish. Wait for it, or retry or roll it back first.", env.name)
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckoutWarning {
    env_name: String,
    feature_name: String,
    feature_branch: String,
    base: String,
    /// "base": edits there are moved back onto the feature branch.
    /// "other": switching features and Back to base refuse until the repo returns.
    kind: &'static str,
}

/// Some(..) when checking out `branch` in `path` takes the repo off its
/// environment's active feature.
pub fn checkout_warning(store: &SmartStore, path: &str, branch: &str) -> Option<CheckoutWarning> {
    let (env, r) = store.find_repo(path)?;
    let feature = env.active_feature()?;
    if feature.branch == branch {
        return None;
    }
    let repo = Repository::open(&r.path).ok()?;
    let base = base_of(&env, &r, &repo)?;
    Some(CheckoutWarning {
        env_name: env.name.clone(),
        feature_name: feature.name.clone(),
        feature_branch: feature.branch.clone(),
        kind: if base == branch { "base" } else { "other" },
        base,
    })
}

pub fn guard_message(g: &CommitGuard) -> String {
    format!(
        "\"{}\" is active in {}, so this commit belongs on {}, not {}. Move the changes to {} first.",
        g.feature_name, g.env_name, g.branch, g.base, g.branch
    )
}

// ---------------------------------------------------------------------------
// Start / Back to base

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct StartResult {
    env: SmartEnv,
    /// Repos moved onto the branch right away (they already had changes on base).
    moved: Vec<AutoBranched>,
    pub(crate) warnings: Vec<String>,
}

/// Start (or, with `adopt`, take over an existing branch as) a feature and make
/// it active. Creates no branches: repos join as their files change. Repos that
/// already have uncommitted changes on base move right away.
pub fn start_feature(
    store: &SmartStore,
    locks: &NetLocks,
    env_id: &str,
    name: &str,
    branch: &str,
    adopt: bool,
) -> CmdResult<StartResult> {
    let branch = branch.trim().to_string();
    let mut warnings = Vec::new();
    {
        let _op = store.op_lock();
        let env = store.get(env_id)?;
        if env.journal.is_some() {
            return Err(format!("A feature switch in {} didn't finish. Resolve it first.", env.name));
        }
        let check = check_branch(&env, &branch)?;
        if let Some(err) = check.error {
            return Err(err);
        }
        let existing: Vec<&BranchRepo> = check.repos.iter().filter(|r| r.local).collect();
        if !existing.is_empty() && !adopt {
            return Err(format!(
                "{branch} already exists in {}. Adopt it as the feature instead.",
                existing.iter().map(|r| r.label.as_str()).collect::<Vec<_>>().join(", ")
            ));
        }
        // Park the active feature first (its changes go into its pinned stashes).
        if env.active_feature_id.is_some() {
            warnings.extend(switch_locked(store, locks, env_id, None, &|_| {})?.warnings);
        }

        let feature = Feature {
            id: new_id("f"),
            name: Some(name.trim()).filter(|n| !n.is_empty()).unwrap_or(branch.as_str()).to_string(),
            branch: branch.clone(),
            created_at: now_secs(),
            repos: existing.iter().map(|r| (r.repo_id.clone(), FeatureRepo::default())).collect::<BTreeMap<_, _>>(),
        };
        let feature_id = feature.id.clone();
        store.update(env_id, |env| {
            env.features.push(feature);
            env.active_feature_id = Some(feature_id);
            Ok(())
        })?;

        // Adopted: check the branch out where it exists, if that's safe. A repo
        // with changes on base keeps them; `git switch` refuses to overwrite.
        for r in existing {
            let Ok(sr) = env.repo(&r.repo_id) else { continue };
            let Ok(repo) = open_repo(sr) else { continue };
            if head_branch(&repo).as_deref() == Some(branch.as_str()) {
                continue;
            }
            if let Err(err) = git(Path::new(&sr.path), &["switch", &branch]) {
                warnings.push(format!("{}: couldn't check out {branch}.\n{err}", sr.label));
            }
        }
    }

    let env = store.get(env_id)?;
    let mut moved = Vec::new();
    for r in &env.repos {
        match auto_branch(store, env_id, &r.id, false) {
            Ok(Some(done)) if done.created => moved.push(done),
            Ok(_) => {}
            Err(err) => warnings.push(err),
        }
    }
    Ok(StartResult { env: store.get(env_id)?, moved, warnings })
}

/// Check out a teammate's origin/<branch> as the local feature branch in one
/// repo. Only from a clean base, so nothing local is mixed into their commits.
pub fn checkout_remote(store: &SmartStore, env_id: &str, repo_id: &str) -> CmdResult<SmartEnv> {
    let _op = store.op_lock();
    let env = store.get(env_id)?;
    let feature = env.active_feature().cloned().ok_or("No feature is active.")?;
    let r = env.repo(repo_id)?.clone();
    let repo = open_repo(&r)?;
    if local_branch_exists(&repo, &feature.branch) {
        return Err(format!("{} already has a local {}.", r.label, feature.branch));
    }
    if is_dirty(&repo) {
        return Err(format!("{} has uncommitted changes. Commit or discard them first.", r.label));
    }
    let remote = format!("origin/{}", feature.branch);
    git(Path::new(&r.path), &["switch", "--track", "-c", &feature.branch, &remote])
        .map_err(|err| format!("{}: couldn't check out {remote}.\n{err}", r.label))?;
    record(store, env_id, &feature.id, &r.id)?;
    store.get(env_id)
}

pub fn rename_feature(store: &SmartStore, env_id: &str, feature_id: &str, name: &str) -> CmdResult<SmartEnv> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Give the feature a name.".into());
    }
    let (env, _) = store.update(env_id, |env| {
        let f = env.features.iter_mut().find(|f| f.id == feature_id).ok_or("That feature no longer exists.")?;
        f.name = name.to_string();
        Ok(())
    })?;
    Ok(env)
}

// ---------------------------------------------------------------------------
// Push all

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PushOutcome {
    repo_id: String,
    label: String,
    /// "pushed" | "skipped" | "failed"
    status: &'static str,
    detail: String,
}

fn push_one(locks: &NetLocks, env: &SmartEnv, r: &SmartRepo, branch: &str) -> PushOutcome {
    let out = |status, detail: String| PushOutcome { repo_id: r.id.clone(), label: r.label.clone(), status, detail };
    let repo = match open_repo(r) {
        Ok(repo) => repo,
        Err(err) => return out("failed", err),
    };
    let Ok(local) = repo.find_branch(branch, BranchType::Local) else {
        return out("skipped", "the branch no longer exists here".into());
    };
    if repo.find_remote("origin").is_err() {
        return out("skipped", "no origin remote".into());
    }
    let Some(tip) = local.get().target() else { return out("skipped", "the branch has no commits".into()) };
    let upstream = local.upstream().ok().and_then(|u| u.get().target());
    let pending = match upstream {
        Some(up) => repo.graph_ahead_behind(tip, up).map(|(a, _)| a).unwrap_or(0),
        None => base_of(env, r, &repo)
            .and_then(|b| base_tip(&repo, &b))
            .map(|t| repo.graph_ahead_behind(tip, t).map(|(a, _)| a).unwrap_or(0))
            .unwrap_or(1),
    };
    if pending == 0 {
        return out("skipped", if upstream.is_some() { "up to date" } else { "no commits yet" }.into());
    }
    let Some(_guard) = locks.try_lock(&r.path) else { return out("failed", BUSY.into()) };
    // Never forced. `-u` sets the upstream the first time.
    let refspec = format!("refs/heads/{branch}:refs/heads/{branch}");
    match git(Path::new(&r.path), &["push", "-u", "origin", &refspec]) {
        Ok(()) => out("pushed", format!("{pending} {}", if pending == 1 { "commit" } else { "commits" })),
        Err(err) => out("failed", err),
    }
}

/// Push a feature's branch in every repo where it has unpushed commits. Works
/// for inactive features too: pushing a branch needs no checkout.
pub fn push_all(store: &SmartStore, locks: &NetLocks, env_id: &str, feature_id: &str) -> CmdResult<Vec<PushOutcome>> {
    let env = store.get(env_id)?;
    let feature = env.features.iter().find(|f| f.id == feature_id).ok_or("That feature no longer exists.")?;
    let repos: Vec<&SmartRepo> = env.repos.iter().filter(|r| feature.repos.contains_key(&r.id)).collect();
    let mut out = Vec::with_capacity(repos.len());
    for chunk in repos.chunks(PUSH_THREADS) {
        std::thread::scope(|scope| {
            let handles: Vec<_> =
                chunk.iter().map(|r| scope.spawn(|| push_one(locks, &env, r, &feature.branch))).collect();
            for h in handles {
                if let Ok(o) = h.join() {
                    out.push(o);
                }
            }
        });
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Merge base

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MergeOutcome {
    repo_id: String,
    label: String,
    /// "merged" | "skipped" | "failed"
    status: &'static str,
    detail: String,
}

fn merge_one(locks: &NetLocks, env: &SmartEnv, r: &SmartRepo, branch: &str) -> MergeOutcome {
    let out = |status, detail: String| MergeOutcome { repo_id: r.id.clone(), label: r.label.clone(), status, detail };
    let repo = match open_repo(r) {
        Ok(repo) => repo,
        Err(err) => return out("failed", err),
    };
    if head_branch(&repo).as_deref() != Some(branch) {
        return out("skipped", format!("not on {branch}"));
    }
    if repo.state() != RepositoryState::Clean {
        return out("skipped", "a merge, rebase or similar is in progress".into());
    }
    // Aborting a conflicted merge could take uncommitted work with it.
    if is_dirty(&repo) {
        return out("skipped", "uncommitted changes; commit or stash them first".into());
    }
    let Some(base) = base_of(env, r, &repo) else { return out("skipped", "no base branch".into()) };
    let Some(_guard) = locks.try_lock(&r.path) else { return out("failed", BUSY.into()) };
    let cwd = Path::new(&r.path);
    let has_origin = repo.find_remote("origin").is_ok();
    if has_origin {
        let fetched = run_git(
            cwd,
            GitRun {
                args: vec!["fetch".into(), "--quiet".into(), "origin".into(), base.clone()],
                timeout: Some(MANUAL_FETCH_TIMEOUT),
                ..Default::default()
            },
        );
        if let Err(err) = fetched {
            return out("failed", err);
        }
    }
    // origin/<base>, so a stale local base doesn't matter; the local base without a remote.
    let remote = format!("origin/{base}");
    let from = if has_origin && repo.find_reference(&format!("refs/remotes/{remote}")).is_ok() { remote } else { base };
    let Some(theirs) = repo.revparse_single(&from).ok().map(|o| o.id()) else {
        return out("skipped", format!("{from} not found"));
    };
    let Some(ours) = repo.head().ok().and_then(|h| h.target()) else {
        return out("skipped", "the branch has no commits".into());
    };
    let behind = repo.graph_ahead_behind(ours, theirs).map(|(_, b)| b).unwrap_or(0);
    if behind == 0 {
        return out("skipped", format!("already has {from}"));
    }
    match git(cwd, &["merge", "--no-edit", &from]) {
        Ok(()) => out("merged", format!("{} from {from}", if behind == 1 { "1 commit".into() } else { format!("{behind} commits") })),
        Err(err) => {
            // Reopen: the handle's index predates the merge.
            let Ok(repo) = open_repo(r) else { return out("failed", err) };
            if repo.state() != RepositoryState::Merge {
                return out("failed", err);
            }
            let conflicted = conflicted_paths(&repo);
            // Leave the repo as it was: a half-merged repo blocks switching features.
            let _ = git(cwd, &["merge", "--abort"]);
            out(
                "failed",
                format!(
                    "conflicts in {}; nothing was merged. Run `git merge {from}` in the terminal to resolve them.",
                    conflicted.join(", ")
                ),
            )
        }
    }
}

fn conflicted_paths(repo: &Repository) -> Vec<String> {
    let Ok(index) = repo.index() else { return Vec::new() };
    let Ok(conflicts) = index.conflicts() else { return Vec::new() };
    conflicts
        .filter_map(|c| c.ok())
        .filter_map(|c| c.our.or(c.their).or(c.ancestor))
        .map(|e| String::from_utf8_lossy(&e.path).to_string())
        .collect()
}

/// Merge each repo's base (origin/<base> after fetching it) into the active
/// feature's branch, wherever the feature has the branch checked out. Repos
/// with uncommitted changes are skipped; a merge that conflicts is aborted.
pub fn merge_base(store: &SmartStore, locks: &NetLocks, env_id: &str) -> CmdResult<Vec<MergeOutcome>> {
    // Holds off auto-branch and feature switches while branches move.
    let _op = store.op_lock();
    let env = store.get(env_id)?;
    if env.journal.is_some() {
        return Err(format!("A feature switch in {} is running or didn't finish. Retry or roll it back first.", env.name));
    }
    let feature = env.active_feature().ok_or("No feature is active.")?;
    let repos: Vec<&SmartRepo> = env.repos.iter().filter(|r| feature.repos.contains_key(&r.id)).collect();
    let mut out = Vec::with_capacity(repos.len());
    for chunk in repos.chunks(PUSH_THREADS) {
        std::thread::scope(|scope| {
            let handles: Vec<_> =
                chunk.iter().map(|r| scope.spawn(|| merge_one(locks, &env, r, &feature.branch))).collect();
            for h in handles {
                if let Ok(o) = h.join() {
                    out.push(o);
                }
            }
        });
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Commands

#[tauri::command]
pub async fn sf_check_branch(store: State<'_, SmartStore>, env_id: String, branch: String) -> CmdResult<BranchCheck> {
    let store = store.inner().clone();
    blocking(move || check_branch(&store.get(&env_id)?, &branch)).await
}

#[tauri::command]
pub async fn sf_start_feature(
    app: AppHandle,
    store: State<'_, SmartStore>,
    env_id: String,
    name: String,
    branch: String,
    adopt: bool,
) -> CmdResult<StartResult> {
    let store = store.inner().clone();
    blocking(move || start_feature(&store, &app.state::<NetLocks>(), &env_id, &name, &branch, adopt)).await
}

/// The commit guard's "Move to feature branch": branch even if the tree is clean.
#[tauri::command]
pub async fn sf_move_to_feature(store: State<'_, SmartStore>, env_id: String, repo_id: String) -> CmdResult<Option<AutoBranched>> {
    let store = store.inner().clone();
    blocking(move || auto_branch(&store, &env_id, &repo_id, true)).await
}

#[tauri::command]
pub async fn sf_commit_guard(store: State<'_, SmartStore>, path: String) -> CmdResult<Option<CommitGuard>> {
    let store = store.inner().clone();
    blocking(move || Ok(commit_guard(&store, &path))).await
}

#[tauri::command]
pub async fn sf_checkout_warning(store: State<'_, SmartStore>, path: String, branch: String) -> CmdResult<Option<CheckoutWarning>> {
    let store = store.inner().clone();
    blocking(move || Ok(checkout_warning(&store, &path, &branch))).await
}

#[tauri::command]
pub async fn sf_checkout_remote(store: State<'_, SmartStore>, env_id: String, repo_id: String) -> CmdResult<SmartEnv> {
    let store = store.inner().clone();
    blocking(move || checkout_remote(&store, &env_id, &repo_id)).await
}

#[tauri::command]
pub async fn sf_rename_feature(
    store: State<'_, SmartStore>,
    env_id: String,
    feature_id: String,
    name: String,
) -> CmdResult<SmartEnv> {
    rename_feature(&store, &env_id, &feature_id, &name)
}

#[tauri::command]
pub async fn sf_push_all(
    app: AppHandle,
    store: State<'_, SmartStore>,
    env_id: String,
    feature_id: String,
) -> CmdResult<Vec<PushOutcome>> {
    let store = store.inner().clone();
    blocking(move || push_all(&store, &app.state::<NetLocks>(), &env_id, &feature_id)).await
}

#[tauri::command]
pub async fn sf_merge_base(app: AppHandle, store: State<'_, SmartStore>, env_id: String) -> CmdResult<Vec<MergeOutcome>> {
    let store = store.inner().clone();
    blocking(move || merge_base(&store, &app.state::<NetLocks>(), &env_id)).await
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::smart::tests::{env_with, git as sh, repo_with_origin, Scratch};
    use std::path::PathBuf;

    pub(crate) fn head(p: &Path) -> Option<String> {
        head_branch(&Repository::open(p).unwrap())
    }

    pub(crate) fn setup(tag: &str, names: &[&str]) -> (Scratch, SmartStore, String, Vec<PathBuf>) {
        let s = Scratch::new(tag);
        let paths: Vec<PathBuf> = names.iter().map(|n| repo_with_origin(&s, n, "main")).collect();
        let store = SmartStore::new(s.path("cfg"));
        let refs: Vec<&Path> = paths.iter().map(PathBuf::as_path).collect();
        let env = store.save(env_with("E", &refs)).unwrap();
        (s, store, env.id, paths)
    }

    pub(crate) fn repo_id(store: &SmartStore, env_id: &str, i: usize) -> String {
        store.get(env_id).unwrap().repos[i].id.clone()
    }

    #[test]
    fn start_creates_branches_only_where_files_changed() {
        let (_s, store, env_id, p) = setup("start", &["a", "b", "c"]);
        // a already has edits on base: staged, unstaged and untracked.
        std::fs::write(p[0].join("a.txt"), "staged\n").unwrap();
        sh(&p[0], &["add", "a.txt"]);
        std::fs::write(p[0].join("a.txt"), "unstaged\n").unwrap();
        std::fs::write(p[0].join("new.txt"), "untracked\n").unwrap();

        let out = start_feature(&store, &NetLocks::default(), &env_id, "Matchmaking", "feature/mm", false).unwrap();
        assert_eq!(out.moved.len(), 1);
        assert_eq!(head(&p[0]).as_deref(), Some("feature/mm"));
        // Nothing was lost or re-staged in the move.
        assert_eq!(std::fs::read_to_string(p[0].join("a.txt")).unwrap(), "unstaged\n");
        assert!(p[0].join("new.txt").exists());
        let repo = Repository::open(&p[0]).unwrap();
        let st = repo.status_file(Path::new("a.txt")).unwrap();
        assert!(st.contains(git2::Status::INDEX_MODIFIED) && st.contains(git2::Status::WT_MODIFIED));
        // Clean repos stay on base with no branch at all.
        for clean in &p[1..] {
            assert_eq!(head(clean).as_deref(), Some("main"));
            assert!(!local_branch_exists(&Repository::open(clean).unwrap(), "feature/mm"));
        }
        let env = store.get(&env_id).unwrap();
        let f = env.active_feature().unwrap();
        assert_eq!((f.name.as_str(), f.repos.len()), ("Matchmaking", 1));

        // A later edit in b moves b; a clean repo is left alone.
        let b = repo_id(&store, &env_id, 1);
        let c = repo_id(&store, &env_id, 2);
        assert!(auto_branch(&store, &env_id, &c, false).unwrap().is_none());
        std::fs::write(p[1].join("a.txt"), "b edit\n").unwrap();
        let done = auto_branch(&store, &env_id, &b, false).unwrap().unwrap();
        assert!(done.created);
        assert_eq!(head(&p[1]).as_deref(), Some("feature/mm"));
        assert!(auto_branch(&store, &env_id, &b, false).unwrap().is_none(), "second call is a no-op");
        assert_eq!(store.get(&env_id).unwrap().active_feature().unwrap().repos.len(), 2);

        // A display name is optional.
        sh(&p[0], &["commit", "-qam", "wip"]);
        sh(&p[0], &["add", "."]);
        sh(&p[0], &["commit", "-qm", "wip2"]);
        sh(&p[1], &["commit", "-qam", "wip"]);
        let out = start_feature(&store, &NetLocks::default(), &env_id, " ", "fix/other", false).unwrap();
        assert_eq!(out.env.active_feature().unwrap().name, "fix/other");
    }

    #[test]
    fn branch_name_checks() {
        let (_s, store, env_id, p) = setup("names", &["a", "b"]);
        let env = store.get(&env_id).unwrap();
        let err = |b: &str| check_branch(&env, b).unwrap().error;
        assert!(err("").is_some());
        assert!(err("has space").unwrap().contains("doesn't accept"));
        assert!(err("x.lock").is_some());
        assert!(err("@").is_some());
        assert!(err("main").unwrap().contains("base branch"));
        assert!(err("feature/ok").is_none());

        // a/b can't be created where a branch "team" exists, and vice versa.
        sh(&p[1], &["branch", "team"]);
        assert!(err("team/x").unwrap().contains("b: branch \"team\" exists"));
        sh(&p[0], &["branch", "solo/one"]);
        assert!(err("solo").unwrap().contains("a: branch \"solo/one\" exists"));

        // An existing branch is reported, and starting needs `adopt`.
        sh(&p[1], &["branch", "shared"]);
        let check = check_branch(&env, "shared").unwrap();
        assert!(check.error.is_none());
        assert_eq!(check.repos.iter().filter(|r| r.local).count(), 1);
        assert!(start_feature(&store, &NetLocks::default(), &env_id, "", "shared", false).unwrap_err().contains("Adopt"));
        let out = start_feature(&store, &NetLocks::default(), &env_id, "", "shared", true).unwrap();
        assert!(out.warnings.is_empty(), "{:?}", out.warnings);
        assert_eq!(head(&p[1]).as_deref(), Some("shared"), "adopt checks the branch out where it exists");
        assert_eq!(head(&p[0]).as_deref(), Some("main"));
        assert_eq!(out.env.active_feature().unwrap().repos.len(), 1);

        // Another feature can't reuse the branch.
        let env = store.get(&env_id).unwrap();
        assert!(check_branch(&env, "shared").unwrap().error.unwrap().contains("already uses"));
    }

    #[test]
    fn commit_guard_and_move() {
        let (_s, store, env_id, p) = setup("guard", &["a"]);
        let path = p[0].to_string_lossy().to_string();
        assert!(commit_guard(&store, &path).is_none(), "no active feature, no guard");
        start_feature(&store, &NetLocks::default(), &env_id, "", "feat/g", false).unwrap();
        let g = commit_guard(&store, &path).unwrap();
        assert_eq!((g.branch.as_str(), g.base.as_str()), ("feat/g", "main"));

        // The commit command refuses on base...
        std::fs::write(p[0].join("a.txt"), "x\n").unwrap();
        sh(&p[0], &["add", "."]);
        let block = |m: &str| {
            tauri::async_runtime::block_on(crate::git::commit_impl(Some(store.clone()), path.clone(), m.into(), false))
        };
        assert!(block("on base").unwrap_err().contains("belongs on feat/g"));
        // ...and works once the changes are moved.
        move_and_commit(&store, &env_id, &path);
        assert_eq!(head(&p[0]).as_deref(), Some("feat/g"));
        assert!(commit_guard(&store, &path).is_none());
        assert!(block("x").is_err(), "nothing staged now");
    }

    fn move_and_commit(store: &SmartStore, env_id: &str, path: &str) {
        let repo_id = store.get(env_id).unwrap().repos[0].id.clone();
        auto_branch(store, env_id, &repo_id, true).unwrap().unwrap();
        tauri::async_runtime::block_on(crate::git::commit_impl(Some(store.clone()), path.into(), "c".into(), false))
            .unwrap();
    }

    #[test]
    fn push_all_pushes_only_what_is_unpushed() {
        let (_s, store, env_id, p) = setup("push", &["a", "b", "c"]);
        let locks = NetLocks::default();
        let started = start_feature(&store, &NetLocks::default(), &env_id, "", "feat/p", false).unwrap();
        let fid = started.env.active_feature_id.clone().unwrap();
        for (i, repo) in p.iter().take(2).enumerate() {
            std::fs::write(repo.join("a.txt"), format!("{i}\n")).unwrap();
            auto_branch(&store, &env_id, &repo_id(&store, &env_id, i), false).unwrap().unwrap();
        }
        sh(&p[0], &["commit", "-qam", "a1"]); // b is on the branch but has no commits

        let out = push_all(&store, &locks, &env_id, &fid).unwrap();
        let by = |l: &str| out.iter().find(|o| o.label == l).unwrap();
        assert_eq!(by("a").status, "pushed", "{:?}", out);
        assert_eq!(by("b").status, "skipped");
        assert!(out.iter().all(|o| o.label != "c"), "c never joined the feature");
        // The upstream is set, so the next run has nothing to do.
        let repo = Repository::open(&p[0]).unwrap();
        assert!(repo.find_branch("feat/p", BranchType::Local).unwrap().upstream().is_ok());
        let again = push_all(&store, &locks, &env_id, &fid).unwrap();
        assert!(again.iter().all(|o| o.status == "skipped"), "{again:?}");
    }

    #[test]
    fn checkout_remote_branch() {
        let (s, store, env_id, p) = setup("remote", &["a"]);
        // A teammate pushed feat/team from another clone.
        let other = s.path("other");
        let origin = s.path("remotes/a.git");
        sh(&s.0, &["clone", "-q", origin.to_str().unwrap(), other.to_str().unwrap()]);
        sh(&other, &["switch", "-qc", "feat/team"]);
        std::fs::write(other.join("t.txt"), "t\n").unwrap();
        sh(&other, &["add", "."]);
        sh(&other, &["commit", "-qm", "team"]);
        sh(&other, &["push", "-q", "origin", "feat/team"]);
        sh(&p[0], &["fetch", "-q"]);

        let out = start_feature(&store, &NetLocks::default(), &env_id, "", "feat/team", false).unwrap();
        assert_eq!(head(&p[0]).as_deref(), Some("main"), "remote branches are never checked out automatically");
        assert!(out.env.active_feature().unwrap().repos.is_empty());
        checkout_remote(&store, &env_id, &repo_id(&store, &env_id, 0)).unwrap();
        assert_eq!(head(&p[0]).as_deref(), Some("feat/team"));
        assert!(p[0].join("t.txt").exists());
    }

    #[test]
    fn merge_base_merges_skips_and_aborts() {
        let (s, store, env_id, p) = setup("merge", &["a", "b", "c", "d"]);
        let locks = NetLocks::default();
        start_feature(&store, &NetLocks::default(), &env_id, "", "feat/m", false).unwrap();
        for (i, repo) in p.iter().enumerate() {
            std::fs::write(repo.join("f.txt"), format!("feature {i}\n")).unwrap();
            auto_branch(&store, &env_id, &repo_id(&store, &env_id, i), false).unwrap().unwrap();
            sh(repo, &["add", "."]);
            sh(repo, &["commit", "-qm", "feature"]);
        }
        // A teammate moves main in a, b and c; b's change conflicts with the feature's.
        for (i, name) in ["a", "b", "c"].iter().enumerate() {
            let other = s.path(&format!("other-{name}"));
            let origin = s.path(&format!("remotes/{name}.git"));
            sh(&s.0, &["clone", "-q", origin.to_str().unwrap(), other.to_str().unwrap()]);
            let file = if i == 1 { "f.txt" } else { "m.txt" };
            std::fs::write(other.join(file), "from main\n").unwrap();
            sh(&other, &["add", "."]);
            sh(&other, &["commit", "-qm", "main moved"]);
            sh(&other, &["push", "-q", "origin", "main"]);
        }
        std::fs::write(p[2].join("f.txt"), "uncommitted\n").unwrap();
        let b_before = Repository::open(&p[1]).unwrap().head().unwrap().target();

        let out = merge_base(&store, &locks, &env_id).unwrap();
        let by = |l: &str| out.iter().find(|o| o.label == l).unwrap();
        assert_eq!(by("a").status, "merged", "{out:?}");
        assert!(p[0].join("m.txt").exists(), "fetched origin/main before merging");
        assert_eq!(head(&p[0]).as_deref(), Some("feat/m"));
        assert_eq!(by("b").status, "failed");
        assert!(by("b").detail.contains("f.txt"), "{}", by("b").detail);
        let b = Repository::open(&p[1]).unwrap();
        assert_eq!(b.state(), RepositoryState::Clean, "the conflicted merge was aborted");
        assert_eq!(b.head().unwrap().target(), b_before);
        assert_eq!(by("c").status, "skipped");
        assert!(by("c").detail.contains("uncommitted"));
        assert_eq!(std::fs::read_to_string(p[2].join("f.txt")).unwrap(), "uncommitted\n");
        assert_eq!(by("d").status, "skipped", "main didn't move in d");
        // a is up to date now.
        let again = merge_base(&store, &locks, &env_id).unwrap();
        assert_eq!(again.iter().find(|o| o.label == "a").unwrap().status, "skipped");
    }

    #[test]
    fn checkout_stays_at_the_local_commit_and_warns_off_feature() {
        let (s, store, env_id, p) = setup("checkout", &["a"]);
        let path = p[0].to_str().unwrap().to_string();
        let run = |b: &str, t: Option<&str>| {
            tauri::async_runtime::block_on(crate::git::checkout_impl(
                Some(store.clone()),
                path.clone(),
                b.into(),
                t.map(str::to_string),
            ))
        };
        // A teammate moves main and pushes feat/team; a fetch brings both in.
        let other = s.path("other");
        sh(&s.0, &["clone", "-q", s.path("remotes/a.git").to_str().unwrap(), other.to_str().unwrap()]);
        sh(&other, &["commit", "-q", "--allow-empty", "-m", "main moved"]);
        sh(&other, &["push", "-q", "origin", "main"]);
        sh(&other, &["switch", "-qc", "feat/team"]);
        sh(&other, &["push", "-q", "origin", "feat/team"]);
        sh(&p[0], &["switch", "-qc", "side"]);
        sh(&p[0], &["fetch", "-q"]);
        let tip = |r: &str| Repository::open(&p[0]).unwrap().revparse_single(r).unwrap().id();
        let main_before = tip("main");
        assert_ne!(main_before, tip("origin/main"));

        // Local main is behind origin/main: switching leaves it where it is.
        run("main", Some("origin/main")).unwrap();
        assert_eq!(head(&p[0]).as_deref(), Some("main"));
        assert_eq!(tip("HEAD"), main_before);
        // No local feat/team: created at origin/feat/team, tracking it.
        run("feat/team", Some("origin/feat/team")).unwrap();
        assert_eq!(head(&p[0]).as_deref(), Some("feat/team"));
        assert_eq!(tip("HEAD"), tip("origin/feat/team"));
        let repo = Repository::open(&p[0]).unwrap();
        assert!(repo.find_branch("feat/team", BranchType::Local).unwrap().upstream().is_ok());
        assert!(run("nope", None).is_err());
        assert!(run("-x", None).is_err());

        // Warnings only while a feature is active, and not for its own branch.
        assert!(checkout_warning(&store, &path, "main").is_none());
        start_feature(&store, &NetLocks::default(), &env_id, "Game", "feat/g", false).unwrap();
        let w = |b: &str| checkout_warning(&store, &path, b).map(|w| w.kind);
        assert_eq!(w("main"), Some("base"));
        assert_eq!(w("side"), Some("other"));
        assert_eq!(w("feat/g"), None);
    }
}
