//! Git commands exposed to the frontend.
//!
//! Reads go through libgit2 (`git2`). Anything that touches the network or must
//! honour the user's git config/hooks (clone, fetch, pull, push, commit, staging)
//! shells out to the user's `git` binary.

use crate::graph::{GraphBuilder, Segment, WIP_COLOR};
use git2::{
    BranchType, Delta, Diff, DiffFindOptions, DiffOptions, Oid, Patch, Repository, Signature,
    Sort, Status, StatusOptions,
};
use serde::Serialize;
use std::collections::{hash_map::DefaultHasher, HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, State};

pub type CmdResult<T> = Result<T, String>;

const MAX_COMMITS: usize = 20_000;
const MAX_BODY_CHARS: usize = 400;
const MAX_FILES: usize = 5_000;
const MAX_DIFF_LINES: usize = 20_000;
const MAX_CONTENT_BYTES: usize = 2 * 1024 * 1024;
const BACKGROUND_FETCH_TIMEOUT: Duration = Duration::from_secs(45);
const MANUAL_FETCH_TIMEOUT: Duration = Duration::from_secs(300);

fn e<E: std::fmt::Display>(err: E) -> String {
    err.to_string()
}

async fn blocking<T, F>(f: F) -> CmdResult<T>
where
    T: Send + 'static,
    F: FnOnce() -> CmdResult<T> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(f).await.map_err(e)?
}

fn open(path: &str) -> CmdResult<Repository> {
    Repository::open(path).map_err(e)
}

fn lossy(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn root_of(repo: &Repository) -> String {
    let p = repo.workdir().unwrap_or_else(|| repo.path());
    let s = p.to_string_lossy().to_string();
    let trimmed = s.trim_end_matches(['/', '\\']);
    if trimmed.is_empty() {
        s
    } else {
        trimmed.to_string()
    }
}

fn name_of(root: &str) -> String {
    Path::new(root)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| root.to_string())
}

pub fn gravatar(email: &str) -> String {
    format!("{:x}", md5::compute(email.trim().to_lowercase().as_bytes()))
}

fn split_message(msg: &str) -> (String, String) {
    let mut parts = msg.splitn(2, '\n');
    let summary = parts.next().unwrap_or("").trim().to_string();
    let body = parts.next().unwrap_or("").trim().to_string();
    (summary, body)
}

fn path_str(p: Option<&Path>) -> Option<String> {
    p.map(|p| p.to_string_lossy().replace('\\', "/"))
}

// ---------------------------------------------------------------------------
// Running the git CLI

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ProgressEvent {
    key: String,
    line: String,
}

/// Receives git's stderr progress lines as they arrive.
pub type Progress = Box<dyn Fn(&str) + Send>;

fn emitter(app: AppHandle, event: &'static str, key: String) -> Progress {
    Box::new(move |line| {
        let _ = app.emit(event, ProgressEvent { key: key.clone(), line: line.to_string() });
    })
}

#[derive(Default)]
pub struct GitRun {
    pub args: Vec<String>,
    pub stdin: Option<String>,
    pub timeout: Option<Duration>,
    pub progress: Option<Progress>,
}

fn git_command(cwd: &Path) -> Command {
    let mut cmd = Command::new("git");
    cmd.current_dir(cwd)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GCM_INTERACTIVE", "never");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    cmd
}

/// Turns the tail of git's stderr into a readable error.
fn git_error(stderr: &str, fallback: String) -> String {
    let lines: Vec<&str> = stderr
        .lines()
        .flat_map(|l| l.split('\r'))
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    if lines.is_empty() {
        return fallback;
    }
    let start = lines.len().saturating_sub(12);
    lines[start..].join("\n")
}

pub fn run_git(cwd: &Path, run: GitRun) -> CmdResult<()> {
    let mut child = git_command(cwd)
        .args(&run.args)
        .stdin(if run.stdin.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| format!("Could not run git: {err}. Is git installed and on your PATH?"))?;

    if let Some(input) = run.stdin {
        if let Some(mut sin) = child.stdin.take() {
            sin.write_all(input.as_bytes()).map_err(e)?;
        }
    }

    let mut stderr = child.stderr.take().unwrap();
    let progress = run.progress;
    let err_thread = std::thread::spawn(move || {
        let mut all = Vec::new();
        let mut line = Vec::new();
        let mut buf = [0u8; 4096];
        loop {
            match stderr.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    all.extend_from_slice(&buf[..n]);
                    for &b in &buf[..n] {
                        if b == b'\n' || b == b'\r' {
                            if let Some(report) = &progress {
                                if !line.is_empty() {
                                    report(&lossy(&line));
                                }
                            }
                            line.clear();
                        } else {
                            line.push(b);
                        }
                    }
                }
            }
        }
        lossy(&all)
    });

    let started = Instant::now();
    let status = loop {
        match child.try_wait().map_err(e)? {
            Some(status) => break status,
            None => {
                if let Some(limit) = run.timeout {
                    if started.elapsed() > limit {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err(format!(
                            "git {} timed out after {}s",
                            run.args.first().map(String::as_str).unwrap_or(""),
                            limit.as_secs()
                        ));
                    }
                }
                std::thread::sleep(Duration::from_millis(40));
            }
        }
    };

    let stderr = err_thread.join().unwrap_or_default();
    if status.success() {
        Ok(())
    } else {
        Err(git_error(
            &stderr,
            format!("git {} failed ({status})", run.args.first().map(String::as_str).unwrap_or("")),
        ))
    }
}

fn args<const N: usize>(a: [&str; N]) -> Vec<String> {
    a.iter().map(|s| s.to_string()).collect()
}

/// Network operations (fetch/pull/push) on one repo never overlap.
#[derive(Default)]
pub struct NetLocks(Mutex<HashSet<String>>);

pub struct NetGuard<'a> {
    locks: &'a NetLocks,
    key: String,
}

impl NetLocks {
    fn try_lock(&self, key: &str) -> Option<NetGuard<'_>> {
        let mut set = self.0.lock().unwrap();
        if set.insert(key.to_string()) {
            Some(NetGuard { locks: self, key: key.to_string() })
        } else {
            None
        }
    }
}

impl Drop for NetGuard<'_> {
    fn drop(&mut self) {
        self.locks.0.lock().unwrap().remove(&self.key);
    }
}

const BUSY: &str = "Another fetch, pull or push is already running for this repository.";

// ---------------------------------------------------------------------------
// Repository discovery

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoInfo {
    root: String,
    name: String,
}

fn info(repo: &Repository) -> RepoInfo {
    let root = root_of(repo);
    RepoInfo { name: name_of(&root), root }
}

#[tauri::command]
pub async fn open_repo(path: String) -> CmdResult<RepoInfo> {
    blocking(move || {
        let repo = Repository::discover(&path)
            .map_err(|_| format!("{path} is not inside a git repository"))?;
        if repo.is_bare() {
            return Err("Bare repositories are not supported".into());
        }
        Ok(info(&repo))
    })
    .await
}

#[tauri::command]
pub async fn init_repo(path: String) -> CmdResult<RepoInfo> {
    blocking(move || Repository::init(&path).map(|r| info(&r)).map_err(e)).await
}

#[tauri::command]
pub fn default_clone_dir() -> String {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".into())
}

/// "WahooFitness" from git@github.com:WahooFitness/repo.git or https://host/WahooFitness/repo
fn owner_from_url(url: &str) -> Option<String> {
    let trimmed = url.trim_end_matches('/').trim_end_matches(".git");
    let path = match trimmed.split_once("://") {
        Some((_, rest)) => rest.split_once('/').map(|(_, p)| p)?,
        None => trimmed.split_once(':').map(|(_, p)| p)?,
    };
    let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    (segs.len() >= 2).then(|| segs[segs.len() - 2].to_string())
}

fn head_branch(repo: &Repository) -> Option<String> {
    let head = repo.head().ok()?;
    head.is_branch().then(|| head.shorthand().ok().map(str::to_string)).flatten()
}

fn unborn_branch(repo: &Repository) -> Option<String> {
    // HEAD -> refs/heads/main even when main has no commits yet.
    let r = repo.find_reference("HEAD").ok()?;
    r.symbolic_target()
        .ok()
        .flatten()
        .and_then(|t| t.strip_prefix("refs/heads/"))
        .map(str::to_string)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoSummary {
    root: String,
    name: String,
    branch: Option<String>,
    ahead: usize,
    behind: usize,
    owner: Option<String>,
}

#[tauri::command]
pub async fn repo_summary(path: String) -> CmdResult<RepoSummary> {
    blocking(move || {
        let repo = open(&path)?;
        let RepoInfo { root, name } = info(&repo);
        let branch = head_branch(&repo).or_else(|| unborn_branch(&repo));
        let (mut ahead, mut behind) = (0, 0);
        if let Some(b) = branch.as_deref().and_then(|b| repo.find_branch(b, BranchType::Local).ok()) {
            if let (Ok(local), Ok(up)) = (b.get().peel_to_commit(), b.upstream()) {
                if let Ok(upc) = up.get().peel_to_commit() {
                    (ahead, behind) = repo.graph_ahead_behind(local.id(), upc.id()).unwrap_or((0, 0));
                }
            }
        }
        let owner = repo
            .find_remote("origin")
            .ok()
            .and_then(|r| r.url().ok().map(str::to_string))
            .and_then(|u| owner_from_url(&u));
        Ok(RepoSummary { root, name, branch, ahead, behind, owner })
    })
    .await
}

// ---------------------------------------------------------------------------
// Refs

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RefLabel {
    name: String,
    kind: &'static str,
    remote: Option<String>,
    is_head: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalBranch {
    name: String,
    target: String,
    upstream: Option<String>,
    ahead: usize,
    behind: usize,
    is_head: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteBranch {
    remote: String,
    name: String,
    full_name: String,
    target: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TagInfo {
    name: String,
    target: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteInfo {
    name: String,
    url: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HeadInfo {
    branch: Option<String>,
    oid: Option<String>,
    detached: bool,
    unborn: bool,
}

struct Refs {
    locals: Vec<LocalBranch>,
    remotes: Vec<RemoteBranch>,
    tags: Vec<TagInfo>,
    remote_infos: Vec<RemoteInfo>,
    labels: Vec<(Oid, RefLabel)>,
}

fn collect_refs(repo: &Repository) -> CmdResult<Refs> {
    let mut labels = Vec::new();

    let mut locals = Vec::new();
    for item in repo.branches(Some(BranchType::Local)).map_err(e)? {
        let Ok((b, _)) = item else { continue };
        let Some(name) = b.name().ok().flatten().map(str::to_string) else { continue };
        let Ok(commit) = b.get().peel_to_commit() else { continue };
        let target = commit.id();
        let is_head = b.is_head();
        let (mut upstream, mut ahead, mut behind) = (None, 0, 0);
        if let Ok(up) = b.upstream() {
            upstream = up.name().ok().flatten().map(str::to_string);
            if let Ok(upc) = up.get().peel_to_commit() {
                (ahead, behind) = repo.graph_ahead_behind(target, upc.id()).unwrap_or((0, 0));
            }
        }
        labels.push((target, RefLabel { name: name.clone(), kind: "local", remote: None, is_head }));
        locals.push(LocalBranch { name, target: target.to_string(), upstream, ahead, behind, is_head });
    }

    let remote_names: Vec<String> = repo
        .remotes()
        .map(|r| r.iter().flatten().flatten().map(str::to_string).collect())
        .unwrap_or_default();
    let remote_infos = remote_names
        .iter()
        .map(|n| RemoteInfo {
            name: n.clone(),
            url: repo.find_remote(n).ok().and_then(|r| r.url().ok().map(str::to_string)),
        })
        .collect();

    let mut remotes = Vec::new();
    for item in repo.branches(Some(BranchType::Remote)).map_err(e)? {
        let Ok((b, _)) = item else { continue };
        if matches!(b.get().symbolic_target(), Ok(Some(_))) {
            continue;
        }
        let Some(full) = b.name().ok().flatten().map(str::to_string) else { continue };
        if full.ends_with("/HEAD") {
            continue;
        }
        let Ok(commit) = b.get().peel_to_commit() else { continue };
        let remote = remote_names
            .iter()
            .filter(|r| full.starts_with(&format!("{r}/")))
            .max_by_key(|r| r.len())
            .cloned()
            .unwrap_or_else(|| full.split('/').next().unwrap_or_default().to_string());
        let name = full
            .strip_prefix(&format!("{remote}/"))
            .unwrap_or(&full)
            .to_string();
        labels.push((
            commit.id(),
            RefLabel { name: name.clone(), kind: "remote", remote: Some(remote.clone()), is_head: false },
        ));
        remotes.push(RemoteBranch { remote, name, full_name: full, target: commit.id().to_string() });
    }

    let mut tags = Vec::new();
    if let Ok(refs) = repo.references_glob("refs/tags/*") {
        for r in refs.flatten() {
            let Ok(name) = r.shorthand().map(str::to_string) else { continue };
            let Ok(commit) = r.peel_to_commit() else { continue };
            labels.push((commit.id(), RefLabel { name: name.clone(), kind: "tag", remote: None, is_head: false }));
            tags.push(TagInfo { name, target: commit.id().to_string() });
        }
    }

    Ok(Refs { locals, remotes, tags, remote_infos, labels })
}

fn head_info(repo: &Repository) -> HeadInfo {
    match repo.head() {
        Ok(head) => HeadInfo {
            branch: if head.is_branch() { head.shorthand().ok().map(str::to_string) } else { None },
            oid: head.target().map(|o| o.to_string()),
            detached: repo.head_detached().unwrap_or(false),
            unborn: false,
        },
        Err(_) => HeadInfo { branch: unborn_branch(repo), oid: None, detached: false, unborn: true },
    }
}

fn fingerprint_of(repo: &Repository, labels: &[(Oid, RefLabel)]) -> String {
    let mut items: Vec<(String, &str, &str, Oid)> = labels
        .iter()
        .map(|(o, l)| (l.name.clone(), l.kind, l.remote.as_deref().unwrap_or(""), *o))
        .collect();
    items.sort();
    let mut h = DefaultHasher::new();
    items.hash(&mut h);
    if let Ok(head) = repo.head() {
        head.name().ok().hash(&mut h);
        head.target().hash(&mut h);
    }
    format!("{:016x}", h.finish())
}

#[tauri::command]
pub async fn repo_fingerprint(path: String) -> CmdResult<String> {
    blocking(move || {
        let repo = open(&path)?;
        let refs = collect_refs(&repo)?;
        Ok(fingerprint_of(&repo, &refs.labels))
    })
    .await
}

// ---------------------------------------------------------------------------
// Commit graph

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Author {
    name: String,
    email: String,
    gravatar: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitRow {
    id: String,
    summary: String,
    body: String,
    author: u32,
    time: i64,
    merge: bool,
    lane: u16,
    color: u16,
    segs: Vec<Segment>,
    refs: Vec<RefLabel>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoData {
    root: String,
    name: String,
    fingerprint: String,
    truncated: bool,
    has_wip: bool,
    head: HeadInfo,
    authors: Vec<Author>,
    commits: Vec<CommitRow>,
    max_lanes: usize,
    locals: Vec<LocalBranch>,
    remotes: Vec<RemoteBranch>,
    remote_infos: Vec<RemoteInfo>,
    tags: Vec<TagInfo>,
}

fn label_rank(l: &RefLabel) -> u8 {
    match (l.kind, l.is_head) {
        ("local", true) => 0,
        ("local", false) => 1,
        ("remote", _) => 2,
        _ => 3,
    }
}

fn is_dirty(repo: &Repository) -> bool {
    let mut opts = StatusOptions::new();
    opts.include_untracked(true).recurse_untracked_dirs(false).exclude_submodules(true);
    repo.statuses(Some(&mut opts)).map(|s| !s.is_empty()).unwrap_or(false)
}

fn now_secs() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

#[tauri::command]
pub async fn load_repo(path: String) -> CmdResult<RepoData> {
    blocking(move || build_repo_data(&path)).await
}

fn build_repo_data(path: &str) -> CmdResult<RepoData> {
    let repo = open(path)?;
    let RepoInfo { root, name } = info(&repo);
    let refs = collect_refs(&repo)?;
    let head = head_info(&repo);
    let head_oid = head.oid.as_deref().and_then(|o| Oid::from_str(o).ok());
    let has_wip = is_dirty(&repo);
    let fingerprint = fingerprint_of(&repo, &refs.labels);

    let mut label_map: HashMap<Oid, Vec<RefLabel>> = HashMap::new();
    for (oid, l) in &refs.labels {
        label_map.entry(*oid).or_default().push(l.clone());
    }
    for v in label_map.values_mut() {
        v.sort_by(|a, b| label_rank(a).cmp(&label_rank(b)).then_with(|| a.name.cmp(&b.name)));
    }

    let mut graph = GraphBuilder::default();
    let mut commits = Vec::new();
    let mut authors: Vec<Author> = Vec::new();
    let mut author_ids: HashMap<(String, String), u32> = HashMap::new();

    if has_wip {
        let row = graph.push_wip(head_oid);
        commits.push(CommitRow {
            id: "WIP".into(),
            summary: String::new(),
            body: String::new(),
            author: u32::MAX,
            time: now_secs(),
            merge: false,
            lane: row.lane,
            color: WIP_COLOR,
            segs: row.segments,
            refs: Vec::new(),
        });
    }

    let mut truncated = false;
    if !refs.labels.is_empty() || head_oid.is_some() {
        let mut walk = repo.revwalk().map_err(e)?;
        walk.set_sorting(Sort::TOPOLOGICAL | Sort::TIME).map_err(e)?;
        let mut pushed = HashSet::new();
        for oid in refs.labels.iter().map(|(o, _)| *o).chain(head_oid) {
            if pushed.insert(oid) {
                let _ = walk.push(oid);
            }
        }
        for (i, oid) in walk.enumerate() {
            if i >= MAX_COMMITS {
                truncated = true;
                break;
            }
            let Ok(oid) = oid else { continue };
            let Ok(commit) = repo.find_commit(oid) else { continue };
            let parents: Vec<Oid> = commit.parent_ids().collect();
            let row = graph.push(oid, &parents);

            let sig = commit.author();
            let key = (lossy(sig.name_bytes()), lossy(sig.email_bytes()));
            let author = *author_ids.entry(key.clone()).or_insert_with(|| {
                authors.push(Author { gravatar: gravatar(&key.1), name: key.0, email: key.1 });
                (authors.len() - 1) as u32
            });

            let message = lossy(commit.message_bytes());
            let (summary, body) = split_message(&message);
            let body: String = body.chars().take(MAX_BODY_CHARS).collect();

            commits.push(CommitRow {
                id: oid.to_string(),
                summary,
                body,
                author,
                time: sig.when().seconds(),
                merge: parents.len() > 1,
                lane: row.lane,
                color: row.color,
                segs: row.segments,
                refs: label_map.remove(&oid).unwrap_or_default(),
            });
        }
    }

    Ok(RepoData {
        root,
        name,
        fingerprint,
        truncated,
        has_wip,
        head,
        authors,
        commits,
        max_lanes: graph.max_lanes.max(1),
        locals: refs.locals,
        remotes: refs.remotes,
        remote_infos: refs.remote_infos,
        tags: refs.tags,
    })
}

// ---------------------------------------------------------------------------
// Commit details and diffs

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Person {
    name: String,
    email: String,
    gravatar: String,
    time: i64,
}

fn person(sig: &Signature) -> Person {
    let email = lossy(sig.email_bytes());
    Person { name: lossy(sig.name_bytes()), gravatar: gravatar(&email), email, time: sig.when().seconds() }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CoAuthor {
    name: String,
    email: String,
    gravatar: String,
}

fn parse_co_authors(message: &str) -> Vec<CoAuthor> {
    let mut out: Vec<CoAuthor> = Vec::new();
    for line in message.lines() {
        let l = line.trim();
        if l.len() < 15 || !l[..15].eq_ignore_ascii_case("co-authored-by:") {
            continue;
        }
        let rest = l[15..].trim();
        let (name, email) = match (rest.find('<'), rest.rfind('>')) {
            (Some(a), Some(b)) if b > a => (rest[..a].trim().to_string(), rest[a + 1..b].trim().to_string()),
            _ => (rest.to_string(), String::new()),
        };
        if !out.iter().any(|c| c.email == email && c.name == name) {
            out.push(CoAuthor { gravatar: gravatar(&email), name, email });
        }
    }
    out
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileChange {
    path: String,
    old_path: Option<String>,
    status: String,
    additions: usize,
    deletions: usize,
    binary: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitDetail {
    id: String,
    summary: String,
    body: String,
    author: Person,
    committer: Person,
    parents: Vec<String>,
    co_authors: Vec<CoAuthor>,
    files: Vec<FileChange>,
    files_truncated: bool,
}

fn delta_status(d: Delta) -> &'static str {
    match d {
        Delta::Added | Delta::Untracked => "A",
        Delta::Deleted => "D",
        Delta::Renamed => "R",
        Delta::Copied => "C",
        Delta::Typechange => "T",
        Delta::Conflicted => "U",
        _ => "M",
    }
}

fn find_renames(diff: &mut Diff) -> CmdResult<()> {
    let mut find = DiffFindOptions::new();
    find.renames(true);
    diff.find_similar(Some(&mut find)).map_err(e)
}

#[tauri::command]
pub async fn commit_detail(path: String, oid: String) -> CmdResult<CommitDetail> {
    blocking(move || {
        let repo = open(&path)?;
        let commit = repo.find_commit(Oid::from_str(&oid).map_err(e)?).map_err(e)?;
        let message = lossy(commit.message_bytes());
        let (summary, body) = split_message(&message);
        let (author, committer) = (person(&commit.author()), person(&commit.committer()));

        let tree = commit.tree().map_err(e)?;
        let parent_tree = match commit.parent(0) {
            Ok(p) => Some(p.tree().map_err(e)?),
            Err(_) => None,
        };
        let mut diff = repo
            .diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), Some(&mut DiffOptions::new()))
            .map_err(e)?;
        find_renames(&mut diff)?;

        let total = diff.deltas().len();
        let mut files = Vec::new();
        for (idx, delta) in diff.deltas().enumerate().take(MAX_FILES) {
            let (additions, deletions, binary) = match Patch::from_diff(&diff, idx) {
                Ok(Some(p)) => {
                    let (_, a, d) = p.line_stats().unwrap_or((0, 0, 0));
                    (a, d, delta.flags().is_binary())
                }
                _ => (0, 0, true),
            };
            let new_path = path_str(delta.new_file().path());
            let old_path = path_str(delta.old_file().path());
            let status = delta_status(delta.status());
            let path = if status == "D" { old_path.clone() } else { new_path.clone() };
            files.push(FileChange {
                path: path.unwrap_or_default(),
                old_path: if matches!(status, "R" | "C") { old_path } else { None },
                status: status.to_string(),
                additions,
                deletions,
                binary,
            });
        }

        let parents = commit.parent_ids().map(|p| p.to_string()).collect();
        let detail = CommitDetail {
            id: commit.id().to_string(),
            summary,
            body,
            author,
            committer,
            parents,
            co_authors: parse_co_authors(&message),
            files,
            files_truncated: total > MAX_FILES,
        };
        Ok(detail)
    })
    .await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffLineOut {
    /// '+', '-', ' ', or '\\' for "No newline at end of file"
    o: char,
    a: Option<u32>,
    b: Option<u32>,
    t: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HunkOut {
    header: String,
    lines: Vec<DiffLineOut>,
}

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct FileDiff {
    binary: bool,
    truncated: bool,
    hunks: Vec<HunkOut>,
}

fn diff_for_file(diff: &Diff, file: &str) -> CmdResult<FileDiff> {
    let idx = diff
        .deltas()
        .position(|d| path_str(d.new_file().path()).as_deref() == Some(file))
        .or_else(|| diff.deltas().position(|d| path_str(d.old_file().path()).as_deref() == Some(file)));
    let Some(idx) = idx else { return Ok(FileDiff::default()) };
    let delta = diff.get_delta(idx).unwrap();
    let Some(patch) = Patch::from_diff(diff, idx).map_err(e)? else {
        return Ok(FileDiff { binary: true, ..Default::default() });
    };
    if delta.flags().is_binary() {
        return Ok(FileDiff { binary: true, ..Default::default() });
    }

    let mut out = FileDiff::default();
    let mut count = 0usize;
    'hunks: for h in 0..patch.num_hunks() {
        let (hunk, n) = patch.hunk(h).map_err(e)?;
        let mut lines = Vec::with_capacity(n);
        for l in 0..n {
            if count >= MAX_DIFF_LINES {
                out.truncated = true;
                out.hunks.push(HunkOut { header: lossy(hunk.header()).trim_end().to_string(), lines });
                break 'hunks;
            }
            let line = patch.line_in_hunk(h, l).map_err(e)?;
            let (o, t) = match line.origin() {
                c @ ('+' | '-' | ' ') => (c, lossy(line.content()).trim_end_matches(['\n', '\r']).to_string()),
                _ => ('\\', "No newline at end of file".to_string()),
            };
            lines.push(DiffLineOut { o, a: line.old_lineno(), b: line.new_lineno(), t });
            count += 1;
        }
        out.hunks.push(HunkOut { header: lossy(hunk.header()).trim_end().to_string(), lines });
    }
    Ok(out)
}

fn pathspec_opts(files: &[&str]) -> DiffOptions {
    let mut opts = DiffOptions::new();
    opts.disable_pathspec_match(true);
    for f in files {
        opts.pathspec(f);
    }
    opts
}

#[tauri::command]
pub async fn file_diff(path: String, oid: String, file: String, old_file: Option<String>) -> CmdResult<FileDiff> {
    blocking(move || {
        let repo = open(&path)?;
        let commit = repo.find_commit(Oid::from_str(&oid).map_err(e)?).map_err(e)?;
        let tree = commit.tree().map_err(e)?;
        let parent_tree = match commit.parent(0) {
            Ok(p) => Some(p.tree().map_err(e)?),
            Err(_) => None,
        };
        let mut specs = vec![file.as_str()];
        if let Some(o) = old_file.as_deref() {
            specs.push(o);
        }
        let mut opts = pathspec_opts(&specs);
        let mut diff = repo.diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), Some(&mut opts)).map_err(e)?;
        find_renames(&mut diff)?;
        diff_for_file(&diff, &file)
    })
    .await
}

#[tauri::command]
pub async fn wip_file_diff(path: String, file: String, old_file: Option<String>, staged: bool) -> CmdResult<FileDiff> {
    blocking(move || {
        let repo = open(&path)?;
        let mut specs = vec![file.as_str()];
        if let Some(o) = old_file.as_deref() {
            specs.push(o);
        }
        let mut opts = pathspec_opts(&specs);
        let mut diff = if staged {
            let head_tree = repo.head().ok().and_then(|h| h.peel_to_tree().ok());
            repo.diff_tree_to_index(head_tree.as_ref(), None, Some(&mut opts)).map_err(e)?
        } else {
            opts.include_untracked(true).recurse_untracked_dirs(true).show_untracked_content(true);
            repo.diff_index_to_workdir(None, Some(&mut opts)).map_err(e)?
        };
        find_renames(&mut diff)?;
        diff_for_file(&diff, &file)
    })
    .await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileContent {
    binary: bool,
    truncated: bool,
    content: String,
}

fn to_content(bytes: &[u8]) -> FileContent {
    let binary = bytes.iter().take(8000).any(|&b| b == 0);
    if binary {
        return FileContent { binary, truncated: false, content: String::new() };
    }
    let truncated = bytes.len() > MAX_CONTENT_BYTES;
    let slice = &bytes[..bytes.len().min(MAX_CONTENT_BYTES)];
    FileContent { binary, truncated, content: lossy(slice) }
}

/// `oid` = None reads the working-tree file.
#[tauri::command]
pub async fn file_content(path: String, oid: Option<String>, file: String) -> CmdResult<FileContent> {
    blocking(move || {
        let repo = open(&path)?;
        let Some(oid) = oid else {
            let full = PathBuf::from(root_of(&repo)).join(&file);
            let bytes = std::fs::read(&full).map_err(|err| format!("{file}: {err}"))?;
            return Ok(to_content(&bytes));
        };
        let commit = repo.find_commit(Oid::from_str(&oid).map_err(e)?).map_err(e)?;
        let mut candidates = vec![commit.clone()];
        if let Ok(p) = commit.parent(0) {
            candidates.push(p);
        }
        for c in candidates {
            let tree = c.tree().map_err(e)?;
            if let Ok(entry) = tree.get_path(Path::new(&file)) {
                let obj = entry.to_object(&repo).map_err(e)?;
                if let Some(blob) = obj.as_blob() {
                    return Ok(to_content(blob.content()));
                }
            }
        }
        Err(format!("{file} not found in commit"))
    })
    .await
}

// ---------------------------------------------------------------------------
// Working tree (WIP)

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WipStatus {
    unstaged: Vec<FileChange>,
    staged: Vec<FileChange>,
}

fn change(path: String, old_path: Option<String>, status: &str) -> FileChange {
    FileChange { path, old_path, status: status.into(), additions: 0, deletions: 0, binary: false }
}

#[tauri::command]
pub async fn wip_status(path: String) -> CmdResult<WipStatus> {
    blocking(move || {
        let repo = open(&path)?;
        let mut opts = StatusOptions::new();
        opts.include_untracked(true)
            .recurse_untracked_dirs(true)
            .exclude_submodules(true)
            .renames_head_to_index(true);
        let statuses = repo.statuses(Some(&mut opts)).map_err(e)?;
        let (mut staged, mut unstaged) = (Vec::new(), Vec::new());
        for entry in statuses.iter() {
            let s = entry.status();
            let path = lossy(entry.path_bytes()).replace('\\', "/");
            if s.contains(Status::CONFLICTED) {
                unstaged.push(change(path, None, "U"));
                continue;
            }
            let staged_status = if s.contains(Status::INDEX_NEW) {
                Some("A")
            } else if s.contains(Status::INDEX_DELETED) {
                Some("D")
            } else if s.contains(Status::INDEX_RENAMED) {
                Some("R")
            } else if s.contains(Status::INDEX_TYPECHANGE) {
                Some("T")
            } else if s.contains(Status::INDEX_MODIFIED) {
                Some("M")
            } else {
                None
            };
            if let Some(st) = staged_status {
                let (new_path, old_path) = match entry.head_to_index() {
                    Some(d) => (path_str(d.new_file().path()), path_str(d.old_file().path())),
                    None => (None, None),
                };
                staged.push(change(
                    new_path.unwrap_or_else(|| path.clone()),
                    if st == "R" { old_path } else { None },
                    st,
                ));
            }
            let wt_status = if s.contains(Status::WT_NEW) {
                Some("A")
            } else if s.contains(Status::WT_DELETED) {
                Some("D")
            } else if s.contains(Status::WT_TYPECHANGE) {
                Some("T")
            } else if s.contains(Status::WT_MODIFIED) || s.contains(Status::WT_RENAMED) {
                Some("M")
            } else {
                None
            };
            if let Some(st) = wt_status {
                unstaged.push(change(path, None, st));
            }
        }
        staged.sort_by(|a, b| a.path.cmp(&b.path));
        unstaged.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(WipStatus { unstaged, staged })
    })
    .await
}

fn with_paths(mut base: Vec<String>, files: &[String]) -> Vec<String> {
    base.push("--".into());
    base.extend(files.iter().cloned());
    base
}

#[tauri::command]
pub async fn stage_files(path: String, files: Vec<String>) -> CmdResult<()> {
    blocking(move || {
        let cwd = PathBuf::from(&path);
        if files.is_empty() {
            run_git(&cwd, GitRun { args: args(["add", "-A"]), ..Default::default() })?;
        } else {
            for chunk in files.chunks(200) {
                run_git(&cwd, GitRun { args: with_paths(args(["add", "-A"]), chunk), ..Default::default() })?;
            }
        }
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn unstage_files(path: String, files: Vec<String>) -> CmdResult<()> {
    blocking(move || {
        let cwd = PathBuf::from(&path);
        let has_head = open(&path)?.head().is_ok();
        let base = if has_head {
            args(["restore", "--staged"])
        } else {
            args(["rm", "--cached", "-r", "-q"])
        };
        if files.is_empty() {
            run_git(&cwd, GitRun { args: with_paths(base, &[".".into()]), ..Default::default() })?;
        } else {
            for chunk in files.chunks(200) {
                run_git(&cwd, GitRun { args: with_paths(base.clone(), chunk), ..Default::default() })?;
            }
        }
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn head_message(path: String) -> CmdResult<String> {
    blocking(move || {
        let repo = open(&path)?;
        let commit = repo.head().map_err(e)?.peel_to_commit().map_err(e)?;
        Ok(lossy(commit.message_bytes()))
    })
    .await
}

#[tauri::command]
pub async fn commit(path: String, message: String, amend: bool) -> CmdResult<String> {
    blocking(move || {
        let mut a = args(["commit", "-F", "-"]);
        if amend {
            a.push("--amend".into());
        }
        run_git(Path::new(&path), GitRun { args: a, stdin: Some(message), ..Default::default() })?;
        let repo = open(&path)?;
        let head = repo.head().map_err(e)?.target().ok_or("HEAD has no target")?;
        Ok(head.to_string())
    })
    .await
}

// ---------------------------------------------------------------------------
// Network operations

#[tauri::command]
pub async fn clone_repo(app: AppHandle, url: String, parent_dir: String, name: String) -> CmdResult<RepoInfo> {
    let dest = Path::new(&parent_dir).join(name.trim()).to_string_lossy().to_string();
    clone_impl(url, parent_dir, name, Some(emitter(app, "clone-progress", dest))).await
}

pub async fn clone_impl(url: String, parent_dir: String, name: String, progress: Option<Progress>) -> CmdResult<RepoInfo> {
    blocking(move || {
        let name = name.trim();
        if name.is_empty() || name.contains(['/', '\\']) {
            return Err("Enter a valid folder name".into());
        }
        let dest = Path::new(&parent_dir).join(name);
        if dest.exists() && std::fs::read_dir(&dest).map(|mut d| d.next().is_some()).unwrap_or(true) {
            return Err(format!("{} already exists and is not empty", dest.display()));
        }
        std::fs::create_dir_all(&parent_dir).map_err(e)?;
        let dest_s = dest.to_string_lossy().to_string();
        run_git(
            Path::new(&parent_dir),
            GitRun {
                args: vec!["clone".into(), "--progress".into(), "--".into(), url.trim().into(), dest_s.clone()],
                progress,
                ..Default::default()
            },
        )?;
        Ok(info(&open(&dest_s)?))
    })
    .await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FetchResult {
    skipped: bool,
}

#[tauri::command]
pub async fn fetch_repo(locks: State<'_, NetLocks>, path: String, quiet: bool) -> CmdResult<FetchResult> {
    fetch_impl(&locks, path, quiet).await
}

pub async fn fetch_impl(locks: &NetLocks, path: String, quiet: bool) -> CmdResult<FetchResult> {
    let Some(_guard) = locks.try_lock(&path) else {
        return if quiet { Ok(FetchResult { skipped: true }) } else { Err(BUSY.into()) };
    };
    let p = path.clone();
    blocking(move || {
        let repo = open(&p)?;
        if repo.remotes().map(|r| r.is_empty()).unwrap_or(true) {
            return Ok(FetchResult { skipped: true });
        }
        let mut a = args(["fetch", "--all", "--prune"]);
        a.push(if quiet { "--quiet" } else { "--progress" }.into());
        run_git(
            Path::new(&p),
            GitRun {
                args: a,
                timeout: Some(if quiet { BACKGROUND_FETCH_TIMEOUT } else { MANUAL_FETCH_TIMEOUT }),
                ..Default::default()
            },
        )?;
        Ok(FetchResult { skipped: false })
    })
    .await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PullResult {
    count: usize,
    upstream: Option<String>,
}

fn head_oid(repo: &Repository) -> Option<Oid> {
    repo.head().ok().and_then(|h| h.target())
}

fn upstream_name(repo: &Repository) -> Option<String> {
    let branch = head_branch(repo)?;
    let b = repo.find_branch(&branch, BranchType::Local).ok()?;
    let up = b.upstream().ok()?;
    up.name().ok().flatten().map(str::to_string)
}

#[tauri::command]
pub async fn pull_repo(app: AppHandle, locks: State<'_, NetLocks>, path: String) -> CmdResult<PullResult> {
    let progress = emitter(app, "pull-progress", path.clone());
    pull_impl(&locks, path, Some(progress)).await
}

pub async fn pull_impl(locks: &NetLocks, path: String, progress: Option<Progress>) -> CmdResult<PullResult> {
    let Some(_guard) = locks.try_lock(&path) else { return Err(BUSY.into()) };
    let p = path.clone();
    blocking(move || {
        let repo = open(&p)?;
        let before = head_oid(&repo);
        let upstream = upstream_name(&repo);
        if upstream.is_none() {
            return Err("The current branch has no upstream branch to pull from.".into());
        }
        let result = run_git(
            Path::new(&p),
            GitRun {
                args: args(["pull", "--ff-only", "--progress"]),
                progress,
                ..Default::default()
            },
        );
        if let Err(msg) = result {
            let lower = msg.to_lowercase();
            if lower.contains("diverg") || lower.contains("not possible to fast-forward") {
                return Err(format!(
                    "Your branch and {} have diverged, so it can't be fast-forwarded. \
                     Merge or rebase in the terminal.\n\n{msg}",
                    upstream.unwrap_or_default()
                ));
            }
            return Err(msg);
        }
        let repo = open(&p)?;
        let after = head_oid(&repo);
        let count = match (before, after) {
            (Some(b), Some(a)) if a != b => repo.graph_ahead_behind(a, b).map(|(n, _)| n).unwrap_or(0),
            _ => 0,
        };
        Ok(PullResult { count, upstream })
    })
    .await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PushResult {
    upstream: Option<String>,
}

#[tauri::command]
pub async fn push_repo(
    app: AppHandle,
    locks: State<'_, NetLocks>,
    path: String,
    remote: Option<String>,
    branch: Option<String>,
) -> CmdResult<PushResult> {
    let progress = emitter(app, "push-progress", path.clone());
    push_impl(&locks, path, remote, branch, Some(progress)).await
}

pub async fn push_impl(
    locks: &NetLocks,
    path: String,
    remote: Option<String>,
    branch: Option<String>,
    progress: Option<Progress>,
) -> CmdResult<PushResult> {
    let Some(_guard) = locks.try_lock(&path) else { return Err(BUSY.into()) };
    let p = path.clone();
    blocking(move || {
        let mut a = args(["push", "--progress"]);
        if let (Some(remote), Some(branch)) = (remote, branch) {
            a.extend(["-u".into(), remote, format!("HEAD:{branch}")]);
        }
        run_git(
            Path::new(&p),
            GitRun { args: a, progress, ..Default::default() },
        )?;
        Ok(PushResult { upstream: upstream_name(&open(&p)?) })
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owner_parsing() {
        assert_eq!(owner_from_url("git@github.com:WahooFitness/lobby-mfe.git").as_deref(), Some("WahooFitness"));
        assert_eq!(owner_from_url("https://github.com/rust-lang/rust").as_deref(), Some("rust-lang"));
        assert_eq!(owner_from_url("/tmp/remote.git"), None);
    }

    #[test]
    fn co_author_trailers() {
        let msg = "Fix\n\nCo-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>\nco-authored-by: A B <a@b.c>\n";
        let c = parse_co_authors(msg);
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].name, "Claude Opus 5.5");
        assert_eq!(c[1].email, "a@b.c");
    }

    /// `GDT_BENCH_REPO=~/some/repo cargo test --release bench_load -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn bench_load() {
        let path = std::env::var("GDT_BENCH_REPO").expect("set GDT_BENCH_REPO");
        let started = Instant::now();
        let data = build_repo_data(&path).unwrap();
        let built = started.elapsed();
        let json = serde_json::to_string(&data).unwrap();
        println!(
            "commits={} lanes={} refs={} wip={} build={:?} json={}KB",
            data.commits.len(),
            data.max_lanes,
            data.locals.len() + data.remotes.len() + data.tags.len(),
            data.has_wip,
            built,
            json.len() / 1024
        );
    }

    #[test]
    fn git_error_keeps_tail() {
        let err = git_error("Counting\rDone\nfatal: bad\n", "x".into());
        assert_eq!(err, "Counting\nDone\nfatal: bad");
    }
}
