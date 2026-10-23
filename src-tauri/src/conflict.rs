//! Merge conflicts: which operation left the repo conflicted (merge, cherry-pick,
//! revert, rebase, am, or a stash apply), the three versions of a conflicted
//! file, and resolving, aborting, skipping or continuing.
//!
//! Everything that changes the repo shells out to `git`, so hooks and the
//! user's config apply, as with staging and committing.

use crate::git::{
    args, blocking, change, commit_impl, e, lossy, open, root_of, run_git, to_content, with_paths, CmdResult,
    FileChange, FileContent, GitRun,
};
use crate::smart::SmartStore;
use git2::{BranchType, Oid, Repository, RepositoryState};
use serde::Serialize;
use std::path::{Component, Path, PathBuf};

#[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "kebab-case")]
pub enum OpKind {
    Merge,
    CherryPick,
    Revert,
    Rebase,
    Am,
    /// No operation in progress, but the index has conflicts: a stash
    /// apply/pop (or `checkout -m`) that didn't apply cleanly.
    Stash,
}

impl OpKind {
    fn command(self) -> &'static str {
        match self {
            OpKind::Merge => "merge",
            OpKind::CherryPick => "cherry-pick",
            OpKind::Revert => "revert",
            OpKind::Rebase => "rebase",
            OpKind::Am => "am",
            OpKind::Stash => "stash",
        }
    }
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Side {
    pub(crate) label: String,
    pub(crate) oid: Option<String>,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ConflictEntry {
    pub(crate) path: String,
    /// both-modified | both-added | both-deleted | deleted-by-us |
    /// deleted-by-them | added-by-us | added-by-them
    pub(crate) kind: &'static str,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct RepoOp {
    pub(crate) kind: OpKind,
    /// The branch being merged into, cherry-picked onto or rebased.
    pub(crate) branch: Option<String>,
    /// "A": HEAD's side. For a rebase that is the new base (git swaps the sides).
    pub(crate) ours: Side,
    /// "B": the commit being merged, picked, reverted or replayed.
    pub(crate) theirs: Side,
    /// The commit message git prepared, without its `#` comment lines.
    pub(crate) message: String,
    /// Whether Continue uses an edited message (am and apply-style rebases don't).
    pub(crate) message_editable: bool,
    pub(crate) step: Option<u32>,
    pub(crate) total: Option<u32>,
    pub(crate) can_skip: bool,
    pub(crate) conflicts: Vec<ConflictEntry>,
}

pub(crate) fn conflicts(repo: &Repository) -> Vec<ConflictEntry> {
    let Ok(index) = repo.index() else { return Vec::new() };
    let Ok(iter) = index.conflicts() else { return Vec::new() };
    let mut out: Vec<ConflictEntry> = iter
        .filter_map(Result::ok)
        .filter_map(|c| {
            let kind = match (c.ancestor.is_some(), c.our.is_some(), c.their.is_some()) {
                (true, true, true) => "both-modified",
                (false, true, true) => "both-added",
                (true, false, true) => "deleted-by-us",
                (true, true, false) => "deleted-by-them",
                (false, true, false) => "added-by-us",
                (false, false, true) => "added-by-them",
                _ => "both-deleted",
            };
            let entry = c.our.as_ref().or(c.their.as_ref()).or(c.ancestor.as_ref())?;
            Some(ConflictEntry { path: lossy(&entry.path).replace('\\', "/"), kind })
        })
        .collect();
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

pub(crate) fn conflicted_paths(repo: &Repository) -> Vec<String> {
    conflicts(repo).into_iter().map(|c| c.path).collect()
}

fn kind_of(repo: &Repository, has_conflicts: bool) -> Option<OpKind> {
    match repo.state() {
        RepositoryState::Merge => Some(OpKind::Merge),
        RepositoryState::CherryPick | RepositoryState::CherryPickSequence => Some(OpKind::CherryPick),
        RepositoryState::Revert | RepositoryState::RevertSequence => Some(OpKind::Revert),
        RepositoryState::Rebase | RepositoryState::RebaseInteractive | RepositoryState::RebaseMerge => {
            Some(OpKind::Rebase)
        }
        RepositoryState::ApplyMailbox | RepositoryState::ApplyMailboxOrRebase => Some(OpKind::Am),
        RepositoryState::Clean if has_conflicts => Some(OpKind::Stash),
        _ => None,
    }
}

/// A file in the git dir, trimmed.
fn read_git(repo: &Repository, rel: &str) -> Option<String> {
    std::fs::read_to_string(repo.path().join(rel)).ok().map(|s| s.trim().to_string())
}

fn read_oid(repo: &Repository, rel: &str) -> Option<Oid> {
    read_git(repo, rel).and_then(|s| Oid::from_str(s.lines().next()?.trim()).ok())
}

fn short(oid: Oid) -> String {
    oid.to_string()[..7].to_string()
}

/// "abc1234 Commit summary"
fn commit_label(repo: &Repository, oid: Oid) -> String {
    match repo.find_commit(oid) {
        Ok(c) => format!("{} {}", short(oid), lossy(c.summary_bytes().unwrap_or_default())),
        Err(_) => short(oid),
    }
}

/// A branch (local first, then remote) pointing at `oid`.
fn branch_at(repo: &Repository, oid: Oid) -> Option<String> {
    for kind in [BranchType::Local, BranchType::Remote] {
        for (b, _) in repo.branches(Some(kind)).ok()?.flatten() {
            if b.get().symbolic_target_bytes().is_none() && b.get().target() == Some(oid) {
                if let Ok(Some(name)) = b.name() {
                    return Some(name.to_string());
                }
            }
        }
    }
    None
}

/// The message without git's `#` comment lines ("# Conflicts: ...").
fn clean_message(text: &str) -> String {
    text.lines().filter(|l| !l.starts_with('#')).collect::<Vec<_>>().join("\n").trim().to_string()
}

/// "Merge branch 'feature/x'" → "feature/x".
fn merged_name(message: &str) -> Option<String> {
    let first = message.lines().next()?;
    if !first.starts_with("Merge ") {
        return None;
    }
    let mut parts = first.split('\'');
    parts.next()?;
    parts.next().filter(|s| !s.is_empty()).map(str::to_string)
}

fn num(repo: &Repository, rel: &str) -> Option<u32> {
    read_git(repo, rel)?.parse().ok()
}

/// What left the repo mid-operation, or None when nothing is in progress.
pub(crate) fn op_state(repo: &Repository) -> Option<RepoOp> {
    let conflicts = conflicts(repo);
    let kind = kind_of(repo, !conflicts.is_empty())?;
    let head = repo.head().ok().and_then(|h| h.target());
    let head_branch = crate::git::head_branch(repo);
    let head_side = || Side {
        label: head_branch.clone().or_else(|| head.map(short)).unwrap_or_else(|| "HEAD".into()),
        oid: head.map(|o| o.to_string()),
    };
    let commit_side = |oid: Option<Oid>| Side {
        label: oid.map(|o| commit_label(repo, o)).unwrap_or_else(|| "?".into()),
        oid: oid.map(|o| o.to_string()),
    };
    let merge_msg = read_git(repo, "MERGE_MSG").map(|m| clean_message(&m)).unwrap_or_default();
    let mut op = RepoOp {
        kind,
        branch: head_branch.clone(),
        ours: head_side(),
        theirs: Side { label: String::new(), oid: None },
        message: merge_msg.clone(),
        message_editable: true,
        step: None,
        total: None,
        can_skip: false,
        conflicts,
    };
    match kind {
        OpKind::Merge => {
            let oid = read_oid(repo, "MERGE_HEAD");
            let label = merged_name(&merge_msg)
                .or_else(|| oid.and_then(|o| branch_at(repo, o)))
                .or_else(|| oid.map(short))
                .unwrap_or_else(|| "?".into());
            op.theirs = Side { label, oid: oid.map(|o| o.to_string()) };
        }
        OpKind::CherryPick => {
            op.theirs = commit_side(read_oid(repo, "CHERRY_PICK_HEAD"));
            op.can_skip = true;
        }
        OpKind::Revert => {
            op.theirs = commit_side(read_oid(repo, "REVERT_HEAD"));
            op.can_skip = true;
        }
        OpKind::Rebase => {
            let merge_dir = repo.path().join("rebase-merge").is_dir();
            let dir = if merge_dir { "rebase-merge" } else { "rebase-apply" };
            op.branch = read_git(repo, &format!("{dir}/head-name"))
                .map(|h| h.strip_prefix("refs/heads/").unwrap_or(&h).to_string())
                .filter(|h| h != "detached HEAD");
            let onto = read_oid(repo, &format!("{dir}/onto"));
            op.ours = Side {
                label: onto.and_then(|o| branch_at(repo, o)).or_else(|| onto.map(short)).unwrap_or_else(|| "HEAD".into()),
                oid: head.map(|o| o.to_string()),
            };
            let replaying = read_oid(repo, "REBASE_HEAD").or_else(|| read_oid(repo, "rebase-merge/stopped-sha"));
            op.theirs = commit_side(replaying);
            let (step, total) = if merge_dir { ("msgnum", "end") } else { ("next", "last") };
            op.step = num(repo, &format!("{dir}/{step}"));
            op.total = num(repo, &format!("{dir}/{total}"));
            op.message_editable = merge_dir;
            op.message = if merge_dir {
                read_git(repo, "rebase-merge/message").map(|m| clean_message(&m)).unwrap_or_default()
            } else {
                replaying
                    .and_then(|o| repo.find_commit(o).ok())
                    .map(|c| clean_message(&lossy(c.message_bytes())))
                    .unwrap_or_default()
            };
            op.can_skip = true;
        }
        OpKind::Am => {
            let (next, last) = (num(repo, "rebase-apply/next"), num(repo, "rebase-apply/last"));
            op.theirs = Side {
                label: match (next, last) {
                    (Some(n), Some(l)) => format!("Patch {n} of {l}"),
                    _ => "Patch".into(),
                },
                oid: None,
            };
            op.step = next;
            op.total = last;
            op.message = read_git(repo, "rebase-apply/msg").map(|m| clean_message(&m)).unwrap_or_default();
            op.message_editable = false;
            op.can_skip = true;
        }
        OpKind::Stash => {
            op.theirs = Side { label: "Stash".into(), oid: None };
            op.message = String::new();
            op.message_editable = false;
        }
    }
    Some(op)
}

/// `file` must stay inside the work tree.
fn checked(file: &str) -> CmdResult<&str> {
    let p = Path::new(file);
    if file.is_empty() || p.is_absolute() || p.components().any(|c| !matches!(c, Component::Normal(_))) {
        return Err(format!("Invalid file path: {file}"));
    }
    Ok(file)
}

fn git(path: &str, a: Vec<String>) -> CmdResult<()> {
    run_git(Path::new(path), GitRun { args: a, ..Default::default() })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConflictVersions {
    base: Option<FileContent>,
    ours: Option<FileContent>,
    theirs: Option<FileContent>,
}

/// The base / ours / theirs versions of a conflicted file (index stages 1–3).
#[tauri::command]
pub async fn conflict_versions(path: String, file: String) -> CmdResult<ConflictVersions> {
    blocking(move || {
        let repo = open(&path)?;
        let index = repo.index().map_err(e)?;
        let stage = |n: i32| {
            index
                .get_path(Path::new(&file), n)
                .and_then(|entry| repo.find_blob(entry.id).ok())
                .map(|blob| to_content(blob.content()))
        };
        let v = ConflictVersions { base: stage(1), ours: stage(2), theirs: stage(3) };
        if v.base.is_none() && v.ours.is_none() && v.theirs.is_none() {
            return Err(format!("{file} has no conflict"));
        }
        Ok(v)
    })
    .await
}

/// Save the merged text and mark the file resolved.
#[tauri::command]
pub async fn resolve_file(path: String, file: String, content: String) -> CmdResult<()> {
    blocking(move || {
        let full = PathBuf::from(&path).join(checked(&file)?);
        if let Some(dir) = full.parent() {
            std::fs::create_dir_all(dir).map_err(e)?;
        }
        std::fs::write(&full, content).map_err(|err| format!("{file}: {err}"))?;
        git(&path, with_paths(args(["add", "-A"]), &[file]))
    })
    .await
}

/// Resolve by taking one side whole ("ours" / "theirs"), or deleting the file.
#[tauri::command]
pub async fn resolve_take(path: String, file: String, side: String) -> CmdResult<()> {
    blocking(move || {
        let f = vec![checked(&file)?.to_string()];
        match side.as_str() {
            "ours" | "theirs" => {
                git(&path, with_paths(vec!["checkout".into(), format!("--{side}")], &f))?;
                git(&path, with_paths(args(["add", "-A"]), &f))
            }
            "delete" => git(&path, with_paths(args(["rm", "-q", "-f"]), &f)),
            other => Err(format!("Unknown side: {other}")),
        }
    })
    .await
}

/// Status of resolved files that have nothing staged (their resolution matches
/// HEAD): "D" when the file is gone, "M" otherwise. Shown in Resolved Files.
#[tauri::command]
pub async fn resolved_status(path: String, files: Vec<String>) -> CmdResult<Vec<FileChange>> {
    blocking(move || {
        let repo = open(&path)?;
        let root = PathBuf::from(root_of(&repo));
        let head = repo.head().ok().and_then(|h| h.peel_to_tree().ok());
        Ok(files
            .into_iter()
            .map(|f| {
                let exists = root.join(&f).exists();
                let in_head = head.as_ref().is_some_and(|t| t.get_path(Path::new(&f)).is_ok());
                let status = if !exists { "D" } else if in_head { "M" } else { "A" };
                change(f, None, status)
            })
            .collect())
    })
    .await
}

fn has_markers(text: &str) -> bool {
    let mut open = false;
    for line in text.lines() {
        if line.starts_with("<<<<<<<") {
            open = true;
        } else if open && line.starts_with(">>>>>>>") {
            return true;
        }
    }
    false
}

/// Which of `files` (empty = every conflicted file) still contain conflict markers.
#[tauri::command]
pub async fn conflict_markers(path: String, files: Vec<String>) -> CmdResult<Vec<String>> {
    blocking(move || {
        let repo = open(&path)?;
        let files = if files.is_empty() { conflicted_paths(&repo) } else { files };
        let root = PathBuf::from(root_of(&repo));
        Ok(files
            .into_iter()
            .filter(|f| std::fs::read(root.join(f)).is_ok_and(|b| has_markers(&String::from_utf8_lossy(&b))))
            .collect())
    })
    .await
}

/// Mark files resolved as they are in the work tree (empty = every conflicted file).
#[tauri::command]
pub async fn mark_resolved(path: String, files: Vec<String>) -> CmdResult<()> {
    blocking(move || {
        let files = if files.is_empty() { conflicted_paths(&open(&path)?) } else { files };
        for chunk in files.chunks(200) {
            git(&path, with_paths(args(["add", "-A"]), chunk))?;
        }
        Ok(())
    })
    .await
}

/// Put a resolved file back into conflict (rebuilt from the index's resolve-undo data).
#[tauri::command]
pub async fn mark_conflicted(path: String, file: String) -> CmdResult<()> {
    blocking(move || git(&path, with_paths(args(["checkout", "-m"]), &[checked(&file)?.to_string()]))).await
}

fn current_kind(path: &str) -> CmdResult<OpKind> {
    let repo = open(path)?;
    kind_of(&repo, !conflicts(&repo).is_empty()).ok_or_else(|| "Nothing is in progress in this repository.".into())
}

/// Stop the operation and put the branch back as it was before it started.
#[tauri::command]
pub async fn abort_operation(path: String) -> CmdResult<()> {
    blocking(move || {
        let a = match current_kind(&path)? {
            // Undo the half-applied stash; git keeps the stash entry when a pop conflicts.
            OpKind::Stash => args(["reset", "--merge"]),
            k => vec![k.command().into(), "--abort".into()],
        };
        git(&path, a)
    })
    .await
}

/// Runs `git <op> --continue/--skip` without an editor. A rebase that stops on
/// the next commit's conflicts isn't an error: the UI shows the new state.
fn sequence_step(path: &str, kind: OpKind, flag: &str) -> CmdResult<()> {
    let run = GitRun {
        args: vec![kind.command().into(), flag.into()],
        env: vec![("GIT_EDITOR", "true".into())],
        ..Default::default()
    };
    run_git(Path::new(path), run).or_else(|err| {
        let repo = open(path)?;
        match op_state(&repo) {
            Some(op) if !op.conflicts.is_empty() => Ok(()),
            _ => Err(err),
        }
    })
}

fn head_sha(path: &str) -> CmdResult<String> {
    let repo = open(path)?;
    let head = repo.head().map_err(e)?.target().ok_or("HEAD has no target")?;
    Ok(head.to_string())
}

#[tauri::command]
pub async fn continue_operation(
    store: tauri::State<'_, SmartStore>,
    path: String,
    message: String,
) -> CmdResult<String> {
    continue_impl(Some(store.inner().clone()), path, message).await
}

/// Commit the resolution and carry on. Returns the new HEAD.
pub async fn continue_impl(store: Option<SmartStore>, path: String, message: String) -> CmdResult<String> {
    let kind = current_kind(&path)?;
    if kind == OpKind::Merge {
        // A merge commit is an ordinary commit; it also gets the Smart Features checks.
        return commit_impl(store, path, message, false).await;
    }
    blocking(move || {
        if let Some(msg) = store.as_ref().and_then(|s| crate::smart_features::switch_running(s, &path)) {
            return Err(msg);
        }
        let repo = open(&path)?;
        let op = op_state(&repo).ok_or("Nothing is in progress in this repository.")?;
        if !op.conflicts.is_empty() {
            return Err("Resolve every conflicted file first.".into());
        }
        if kind == OpKind::Stash {
            return Err("A stash apply has nothing to continue.".into());
        }
        let msg_file = match kind {
            OpKind::CherryPick | OpKind::Revert => Some("MERGE_MSG"),
            OpKind::Rebase if op.message_editable => Some("rebase-merge/message"),
            _ => None,
        };
        if let Some(rel) = msg_file.filter(|_| !message.trim().is_empty()) {
            std::fs::write(repo.path().join(rel), format!("{}\n", message.trim_end())).map_err(e)?;
        }
        sequence_step(&path, kind, "--continue")?;
        head_sha(&path)
    })
    .await
}

/// Drop the commit being applied and go on to the next (rebase, cherry-pick, revert, am).
#[tauri::command]
pub async fn skip_operation(path: String) -> CmdResult<String> {
    blocking(move || {
        let kind = current_kind(&path)?;
        if matches!(kind, OpKind::Merge | OpKind::Stash) {
            return Err(format!("A {} can't be skipped.", kind.command()));
        }
        sequence_step(&path, kind, "--skip")?;
        head_sha(&path)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use tauri::async_runtime::block_on;

    struct Repo {
        dir: PathBuf,
    }

    impl Repo {
        fn new(name: &str) -> Repo {
            let dir = std::env::temp_dir().join(format!("gdt-conflict-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let r = Repo { dir };
            r.git(&["init", "-q", "-b", "main"]);
            r
        }
        fn path(&self) -> String {
            self.dir.to_string_lossy().to_string()
        }
        fn git(&self, a: &[&str]) -> bool {
            Command::new("git")
                .current_dir(&self.dir)
                .env("GIT_EDITOR", "true")
                .args(["-c", "user.name=T", "-c", "user.email=t@t", "-c", "core.autocrlf=false"])
                .args(a)
                .output()
                .unwrap()
                .status
                .success()
        }
        fn write(&self, f: &str, text: &str) {
            std::fs::write(self.dir.join(f), text).unwrap();
        }
        fn read(&self, f: &str) -> Option<String> {
            std::fs::read_to_string(self.dir.join(f)).ok()
        }
        fn commit(&self, f: &str, text: &str, msg: &str) {
            self.write(f, text);
            assert!(self.git(&["add", "-A"]));
            assert!(self.git(&["commit", "-qm", msg]));
        }
        fn op(&self) -> Option<RepoOp> {
            op_state(&open(&self.path()).unwrap())
        }
        fn head(&self) -> String {
            head_sha(&self.path()).unwrap()
        }
        fn state(&self) -> RepositoryState {
            open(&self.path()).unwrap().state()
        }
        /// main and `feat` both change line 2 of f.
        fn diverged(name: &str) -> Repo {
            let r = Repo::new(name);
            r.commit("f", "a\nb\nc\n", "init");
            assert!(r.git(&["checkout", "-qb", "feat"]));
            r.commit("f", "a\nB-feat\nc\n", "feat change");
            assert!(r.git(&["checkout", "-q", "main"]));
            r.commit("f", "a\nB-main\nc\n", "main change");
            r
        }
    }

    impl Drop for Repo {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn merge_state_versions_resolve_and_commit() {
        // f: both changed line 2. gone: deleted on main, edited on feat.
        let r = Repo::new("merge");
        r.write("gone", "x\n");
        r.commit("f", "a\nb\nc\n", "init");
        assert!(r.git(&["checkout", "-qb", "feat"]));
        r.write("gone", "edited\n");
        r.commit("f", "a\nB-feat\nc\n", "feat change");
        assert!(r.git(&["checkout", "-q", "main"]));
        assert!(r.git(&["rm", "-q", "gone"]));
        r.commit("f", "a\nB-main\nc\n", "main change");
        assert!(!r.git(&["merge", "feat"]));

        let op = r.op().expect("merge in progress");
        assert_eq!(op.kind, OpKind::Merge);
        assert_eq!(op.branch.as_deref(), Some("main"));
        assert_eq!(op.ours.label, "main");
        assert_eq!(op.theirs.label, "feat");
        assert_eq!(op.message, "Merge branch 'feat'");
        let kinds: Vec<_> = op.conflicts.iter().map(|c| (c.path.as_str(), c.kind)).collect();
        assert_eq!(kinds, [("f", "both-modified"), ("gone", "deleted-by-us")]);

        let v = block_on(conflict_versions(r.path(), "f".into())).unwrap();
        assert_eq!(v.base.unwrap().content, "a\nb\nc\n");
        assert_eq!(v.ours.unwrap().content, "a\nB-main\nc\n");
        assert_eq!(v.theirs.unwrap().content, "a\nB-feat\nc\n");

        // Markers are still in f; resolving saves it and stages it.
        assert_eq!(block_on(conflict_markers(r.path(), vec![])).unwrap(), ["f"]);
        block_on(resolve_file(r.path(), "f".into(), "a\nB-both\nc\n".into())).unwrap();
        assert_eq!(r.read("f").as_deref(), Some("a\nB-both\nc\n"));
        assert_eq!(r.op().unwrap().conflicts.len(), 1);

        // Mark conflicted brings the conflict back, with markers.
        block_on(mark_conflicted(r.path(), "f".into())).unwrap();
        assert_eq!(r.op().unwrap().conflicts.len(), 2);
        assert!(r.read("f").unwrap().contains("<<<<<<<"));
        block_on(resolve_take(r.path(), "f".into(), "theirs".into())).unwrap();
        assert_eq!(r.read("f").as_deref(), Some("a\nB-feat\nc\n"));
        block_on(resolve_take(r.path(), "gone".into(), "delete".into())).unwrap();
        assert!(r.read("gone").is_none());
        // Deleted, like on main: nothing staged, but it shows as deleted.
        let st = block_on(resolved_status(r.path(), vec!["gone".into(), "f".into()])).unwrap();
        assert_eq!(st.iter().map(|c| c.status.as_str()).collect::<Vec<_>>(), ["D", "M"]);
        assert!(r.op().unwrap().conflicts.is_empty());

        let sha = block_on(continue_impl(None, r.path(), "Merge feat into main".into())).unwrap();
        assert_eq!(r.state(), RepositoryState::Clean);
        let repo = open(&r.path()).unwrap();
        let c = repo.find_commit(Oid::from_str(&sha).unwrap()).unwrap();
        assert_eq!(c.parent_count(), 2);
        assert_eq!(c.summary().ok().flatten(), Some("Merge feat into main"));
        assert!(r.op().is_none());
    }

    #[test]
    fn abort_merge_restores_branch() {
        let r = Repo::diverged("abort");
        let before = r.head();
        assert!(!r.git(&["merge", "feat"]));
        block_on(mark_resolved(r.path(), vec![])).unwrap();
        block_on(abort_operation(r.path())).unwrap();
        assert_eq!(r.state(), RepositoryState::Clean);
        assert_eq!(r.head(), before);
        assert_eq!(r.read("f").as_deref(), Some("a\nB-main\nc\n"));
        assert!(r.op().is_none());
    }

    #[test]
    fn cherry_pick_and_revert() {
        let r = Repo::diverged("pick");
        assert!(!r.git(&["cherry-pick", "feat"]));
        let op = r.op().unwrap();
        assert_eq!(op.kind, OpKind::CherryPick);
        assert!(op.theirs.label.ends_with(" feat change"), "{}", op.theirs.label);
        assert_eq!(op.message, "feat change");
        assert!(block_on(continue_impl(None, r.path(), "x".into())).is_err(), "conflicts remain");
        block_on(resolve_file(r.path(), "f".into(), "a\npicked\nc\n".into())).unwrap();
        block_on(continue_impl(None, r.path(), "Picked feat change".into())).unwrap();
        assert!(r.op().is_none());
        let repo = open(&r.path()).unwrap();
        assert_eq!(repo.head().unwrap().peel_to_commit().unwrap().summary().ok().flatten(), Some("Picked feat change"));

        // Revert "main change" after f changed again: conflicts, then abort.
        r.commit("f", "a\nlater\nc\n", "later");
        let before = r.head();
        assert!(!r.git(&["revert", "HEAD~2"]));
        assert_eq!(r.op().unwrap().kind, OpKind::Revert);
        block_on(abort_operation(r.path())).unwrap();
        assert_eq!(r.head(), before);
        assert!(r.op().is_none());
    }

    #[test]
    fn rebase_stops_twice_then_finishes() {
        let r = Repo::diverged("rebase");
        assert!(r.git(&["checkout", "-q", "feat"]));
        r.commit("f", "a\nB-feat2\nc\n", "feat change 2");
        assert!(!r.git(&["rebase", "main"]));

        let op = r.op().unwrap();
        assert_eq!(op.kind, OpKind::Rebase);
        assert_eq!(op.branch.as_deref(), Some("feat"));
        assert_eq!(op.ours.label, "main");
        assert!(op.theirs.label.ends_with(" feat change"), "{}", op.theirs.label);
        assert_eq!((op.step, op.total), (Some(1), Some(2)));
        assert!(op.message_editable && op.can_skip);

        block_on(resolve_file(r.path(), "f".into(), "a\nstep1\nc\n".into())).unwrap();
        // Stops again on the second commit: not an error.
        block_on(continue_impl(None, r.path(), "Step one, edited".into())).unwrap();
        let op = r.op().unwrap();
        assert_eq!((op.step, op.conflicts.len()), (Some(2), 1));

        block_on(resolve_file(r.path(), "f".into(), "a\nstep2\nc\n".into())).unwrap();
        block_on(continue_impl(None, r.path(), "feat change 2".into())).unwrap();
        assert!(r.op().is_none());
        assert_eq!(crate::git::head_branch(&open(&r.path()).unwrap()).as_deref(), Some("feat"));
        let repo = open(&r.path()).unwrap();
        let first = repo.revparse_single("HEAD~1").unwrap().peel_to_commit().unwrap();
        assert_eq!(first.summary().ok().flatten(), Some("Step one, edited"));
        assert_eq!(first.author().name().ok(), Some("T"));
    }

    #[test]
    fn rebase_skip_and_abort() {
        let r = Repo::diverged("skip");
        assert!(r.git(&["checkout", "-q", "feat"]));
        let before = r.head();
        assert!(!r.git(&["rebase", "main"]));
        block_on(skip_operation(r.path())).unwrap();
        assert!(r.op().is_none());
        assert_eq!(r.read("f").as_deref(), Some("a\nB-main\nc\n"), "the only commit was skipped");

        assert!(r.git(&["reset", "-q", "--hard", &before]));
        assert!(!r.git(&["rebase", "main"]));
        block_on(abort_operation(r.path())).unwrap();
        assert_eq!(r.head(), before);
        assert!(r.op().is_none());
    }

    #[test]
    fn stash_pop_conflict() {
        let r = Repo::diverged("stash");
        r.write("f", "a\nstashed\nc\n");
        r.write("other", "kept\n");
        assert!(r.git(&["add", "other"]));
        assert!(r.git(&["stash", "-q"]));
        r.commit("f", "a\nmoved on\nc\n", "move on");
        let before = r.head();
        assert!(!r.git(&["stash", "pop"]));

        let op = r.op().unwrap();
        assert_eq!(op.kind, OpKind::Stash);
        assert_eq!(op.theirs.label, "Stash");
        assert!(!op.can_skip && !op.message_editable);
        assert!(block_on(continue_impl(None, r.path(), String::new())).is_err());

        block_on(abort_operation(r.path())).unwrap();
        assert!(r.op().is_none());
        assert_eq!(r.head(), before);
        assert_eq!(r.read("f").as_deref(), Some("a\nmoved on\nc\n"));
        assert!(r.git(&["rev-parse", "-q", "--verify", "refs/stash"]), "the stash entry is kept");
    }

    #[test]
    fn rejects_paths_outside_the_repo() {
        assert!(checked("../x").is_err());
        assert!(checked("/etc/passwd").is_err());
        assert!(checked("a/../../b").is_err());
        assert!(checked("dir/file.txt").is_ok());
    }

    #[test]
    fn markers_and_messages() {
        assert!(has_markers("x\n<<<<<<< HEAD\na\n=======\nb\n>>>>>>> feat\n"));
        assert!(!has_markers("x\n>>>>>>> not a conflict\n"));
        assert_eq!(clean_message("Merge branch 'a'\n\n# Conflicts:\n#\tf\n"), "Merge branch 'a'");
        assert_eq!(merged_name("Merge remote-tracking branch 'origin/x'").as_deref(), Some("origin/x"));
        assert_eq!(merged_name("Revert \"y\""), None);
    }
}
