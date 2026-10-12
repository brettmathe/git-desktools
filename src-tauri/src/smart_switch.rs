//! Switching features in place: no worktrees, no extra clones.
//!
//! A switch to feature T (or to base, T = none) converges every repo of the
//! environment from whatever state it's in:
//!   1. park: a repo on some feature F's branch stashes its uncommitted work
//!      (`git stash push --include-untracked`), pins the stash under
//!      `refs/smart-features/<F-id>/stash` (stash-list entries can expire and
//!      their indexes shift; the ref keeps it forever under a stable name), then
//!   2. base: switches to its base branch;
//!   3. switch: repos where T's branch exists check it out, and
//!   4. restore: re-apply T's pinned stash there (`git stash apply --index`),
//!      then drop the pin and its stash-list entry.
//!
//! Every step is checked against git before it runs, so running the same switch
//! again picks up where a failed or interrupted one stopped. The journal (in the
//! env file) records the plan and progress; Retry runs the switch to `to` again,
//! Roll back runs a switch to `from`. Pins are removed only after a successful
//! apply, so a rollback can always restore the parked work.

use crate::git::{blocking, git_command, head_branch, is_dirty, now_secs, CmdResult, NetLocks, BUSY};
use crate::smart::{base_tip, Feature, ParkedStash, SmartEnv, SmartRepo, SmartStore};
use crate::smart_features::{auto_branch_locked, base_of, git, local_branch_exists};
use git2::{BranchType, Repository, RepositoryState, Status, StatusOptions};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::Path;
use std::process::Stdio;
use tauri::{AppHandle, Emitter, Manager, State};

pub const PIN_NS: &str = "refs/smart-features";

pub fn pin_ref(feature_id: &str) -> String {
    format!("{PIN_NS}/{feature_id}/stash")
}

fn pin_exists(repo: &Repository, feature_id: &str) -> bool {
    repo.find_reference(&pin_ref(feature_id)).is_ok()
}

/// Run git and return stdout (on failure: stdout + stderr, since stash apply
/// reports conflicts on stdout).
fn git_out(cwd: &Path, args: &[&str]) -> CmdResult<String> {
    let out = git_command(cwd)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|err| format!("Could not run git: {err}. Is git installed and on your PATH?"))?;
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    if out.status.success() {
        Ok(stdout)
    } else {
        let stderr = String::from_utf8_lossy(&out.stderr);
        Err(format!("{}\n{}", stdout.trim(), stderr.trim()).trim().to_string())
    }
}

fn has_conflicts(repo: &Repository) -> bool {
    let mut opts = StatusOptions::new();
    opts.include_untracked(false).exclude_submodules(true);
    repo.statuses(Some(&mut opts))
        .map(|s| s.iter().any(|e| e.status().contains(Status::CONFLICTED)))
        .unwrap_or(false)
}

/// Marks each stash we create, so a stash made just before a crash (and not yet
/// pinned) is found again by its message.
fn stash_tag(feature_id: &str) -> String {
    format!("[sf {feature_id}]")
}

/// Newest stash-list entry whose message carries `tag`.
fn find_stash(cwd: &Path, tag: &str) -> CmdResult<Option<String>> {
    let list = git_out(cwd, &["stash", "list", "--format=%H%x09%gs"])?;
    Ok(list.lines().find_map(|l| {
        let (sha, subject) = l.split_once('\t')?;
        subject.ends_with(tag).then(|| sha.to_string())
    }))
}

/// Remove the stash-list entry for `sha`, if it's still listed.
fn drop_stash_entry(cwd: &Path, sha: &str) -> CmdResult<()> {
    let list = git_out(cwd, &["stash", "list", "--format=%H"])?;
    if let Some(i) = list.lines().position(|l| l.trim() == sha) {
        git(cwd, &["stash", "drop", "-q", &format!("stash@{{{i}}}")])?;
    }
    Ok(())
}

/// Delete a pin and its stash-list entry. The parked work is gone after this.
pub(crate) fn drop_pin(cwd: &Path, repo: &Repository, feature_id: &str) -> CmdResult<()> {
    let pin = pin_ref(feature_id);
    let Ok(r) = repo.find_reference(&pin) else { return Ok(()) };
    let sha = r.target().map(|o| o.to_string());
    git(cwd, &["update-ref", "-d", &pin])?;
    if let Some(sha) = sha {
        drop_stash_entry(cwd, &sha)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Journal and plan

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Journal {
    pub op: String,
    /// Feature ids; None = base.
    pub from: Option<String>,
    pub to: Option<String>,
    pub started_at: i64,
    pub steps: Vec<Step>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Step {
    pub repo_id: String,
    pub label: String,
    /// "park" | "base" | "switch" | "restore"
    pub action: String,
    /// The feature being parked or restored.
    pub feature_id: Option<String>,
    /// For "park": "changes" when there's uncommitted work to stash. For "base"/"switch": the branch.
    #[serde(default)]
    pub detail: Option<String>,
    /// "pending" | "done" | "failed" | "warning"
    pub status: String,
    #[serde(default)]
    pub message: Option<String>,
}

impl Step {
    fn new(r: &SmartRepo, action: &str, feature: Option<&Feature>, detail: Option<String>) -> Step {
        Step {
            repo_id: r.id.clone(),
            label: r.label.clone(),
            action: action.into(),
            feature_id: feature.map(|f| f.id.clone()),
            detail,
            status: "pending".into(),
            message: None,
        }
    }
}

fn journal_of(env: &SmartEnv) -> Option<Journal> {
    env.journal.clone().and_then(|j| serde_json::from_value(j).ok())
}

/// The steps that take every repo of `env` to `target`, or the problems that
/// stop it. Read-only.
fn plan(env: &SmartEnv, target: Option<&Feature>) -> (Vec<Step>, Vec<String>) {
    let mut steps = Vec::new();
    let mut problems = Vec::new();
    for r in &env.repos {
        let mut problem = |p: String| problems.push(format!("{}: {p}", r.label));
        if !Path::new(&r.path).exists() {
            problem("folder not found".into());
            continue;
        }
        let Ok(repo) = Repository::open(&r.path) else {
            problem("can't open the repository".into());
            continue;
        };
        if repo.state() != RepositoryState::Clean {
            problem("a merge, rebase or similar is in progress".into());
            continue;
        }
        if repo.path().join("index.lock").exists() {
            problem("git is busy here (index.lock exists)".into());
            continue;
        }
        if has_conflicts(&repo) {
            problem("has unresolved conflicts".into());
            continue;
        }
        if repo.head_detached().unwrap_or(false) {
            problem("detached HEAD (not on a branch)".into());
            continue;
        }
        let Some(base) = base_of(env, r, &repo) else {
            problem("no base branch; set one in Edit environment".into());
            continue;
        };
        let Some(cur) = head_branch(&repo) else {
            problem("the current branch has no commits".into());
            continue;
        };
        let dirty = is_dirty(&repo);
        let target_here = target.filter(|t| {
            (t.repos.contains_key(&r.id) || pin_exists(&repo, &t.id)) && local_branch_exists(&repo, &t.branch)
        });
        if let Some(t) = target.filter(|t| pin_exists(&repo, &t.id) && !local_branch_exists(&repo, &t.branch)) {
            problem(format!("{} was deleted, but parked changes for it are still pinned", t.branch));
            continue;
        }

        if cur == base {
            if target_here.is_some() && dirty {
                problem(format!(
                    "uncommitted changes on {base} that belong to no feature; commit or discard them first"
                ));
                continue;
            }
        } else if target.is_some_and(|t| t.branch == cur) {
            // Already on the target's branch.
        } else if let Some(f) = env.features.iter().find(|f| f.branch == cur) {
            if pin_exists(&repo, &f.id) {
                problem(format!(
                    "parked changes for \"{}\" weren't fully restored; resolve them, then choose Mark resolved",
                    f.name
                ));
                continue;
            }
            // Always: a clean repo may still hold a stash an interrupted run didn't pin.
            steps.push(Step::new(r, "park", Some(f), dirty.then(|| "changes".to_string())));
            steps.push(Step::new(r, "base", Some(f), Some(base.clone())));
        } else {
            problem(format!("on {cur}, which is neither {base} nor a feature branch; switch it back first"));
            continue;
        }

        if let Some(t) = target_here {
            if cur != t.branch {
                steps.push(Step::new(r, "switch", Some(t), Some(t.branch.clone())));
            }
            if pin_exists(&repo, &t.id) {
                if cur == t.branch && dirty {
                    problem(format!(
                        "parked changes for \"{}\" weren't fully restored; resolve them, then choose Mark resolved",
                        t.name
                    ));
                    continue;
                }
                steps.push(Step::new(r, "restore", Some(t), None));
            }
        }
    }
    (steps, problems)
}

// ---------------------------------------------------------------------------
// Running a switch

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Preflight {
    from: Option<String>,
    to: Option<String>,
    problems: Vec<String>,
    steps: Vec<Step>,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SwitchResult {
    pub env: SmartEnv,
    /// Repos that need attention afterwards (e.g. conflicted parked changes).
    pub warnings: Vec<String>,
}

pub type ProgressFn<'a> = &'a dyn Fn(&Step);

/// Move repos dirty on base onto the active feature first, so they're parked with it.
fn auto_branch_all(store: &SmartStore, env: &SmartEnv) {
    for r in &env.repos {
        let _ = auto_branch_locked(store, &env.id, &r.id, false);
    }
}

pub fn preflight(store: &SmartStore, env_id: &str, target: Option<&str>) -> CmdResult<Preflight> {
    let _op = store.op_lock();
    let env = store.get(env_id)?;
    auto_branch_all(store, &env);
    let env = store.get(env_id)?;
    let t = target.map(|id| env.features.iter().find(|f| f.id == id).ok_or("That feature no longer exists.")).transpose()?;
    let (steps, problems) = plan(&env, t);
    Ok(Preflight { from: env.active_feature_id.clone(), to: target.map(str::to_string), problems, steps })
}

/// Switch the environment to `target` (None = base). Caller holds `op_lock`.
pub(crate) fn switch_locked(
    store: &SmartStore,
    locks: &NetLocks,
    env_id: &str,
    target: Option<&str>,
    progress: ProgressFn,
) -> CmdResult<SwitchResult> {
    run(store, locks, env_id, target, None, progress)
}

/// `resume_from`: the unfinished journal being retried or rolled back (its
/// from/to are kept, so a second failure can still roll back to the original).
fn run(
    store: &SmartStore,
    locks: &NetLocks,
    env_id: &str,
    target: Option<&str>,
    resume_from: Option<&Journal>,
    progress: ProgressFn,
) -> CmdResult<SwitchResult> {
    let env = store.get(env_id)?;
    if env.journal.is_some() && resume_from.is_none() {
        return Err(format!("A feature switch in {} didn't finish. Retry or roll it back first.", env.name));
    }
    if resume_from.is_none() {
        auto_branch_all(store, &env);
    }
    let env = store.get(env_id)?;
    let t = target
        .map(|id| env.features.iter().find(|f| f.id == id).cloned().ok_or("That feature no longer exists."))
        .transpose()?;

    // Fetch/pull/push wait for the switch (and vice versa).
    let mut guards = Vec::new();
    for r in &env.repos {
        match locks.try_lock(&r.path) {
            Some(g) => guards.push(g),
            None => return Err(format!("{}: {BUSY}", r.label)),
        }
    }

    let (steps, problems) = plan(&env, t.as_ref());
    if !problems.is_empty() {
        return Err(format!("Can't switch yet:\n{}", problems.join("\n")));
    }
    let mut journal = Journal {
        op: "switch".into(),
        from: resume_from.map_or(env.active_feature_id.clone(), |j| j.from.clone()),
        to: resume_from.map_or(target.map(str::to_string), |j| j.to.clone()),
        started_at: resume_from.map_or(now_secs(), |j| j.started_at),
        steps,
    };
    store.update(env_id, |env| {
        env.journal = Some(json!(journal));
        Ok(())
    })?;

    let mut warnings = Vec::new();
    for i in 0..journal.steps.len() {
        progress(&journal.steps[i]);
        let step = journal.steps[i].clone();
        let r = env.repo(&step.repo_id)?.clone();
        let feature = step.feature_id.as_ref().and_then(|id| env.features.iter().find(|f| &f.id == id)).cloned();
        let result = run_step(store, env_id, &r, &step, feature.as_ref());
        let s = &mut journal.steps[i];
        match result {
            Ok(None) => s.status = "done".into(),
            Ok(Some(warning)) => {
                s.status = "warning".into();
                s.message = Some(warning.clone());
                warnings.push(format!("{}: {warning}", r.label));
            }
            Err(err) => {
                s.status = "failed".into();
                s.message = Some(err.clone());
            }
        }
        progress(&journal.steps[i]);
        let failed = journal.steps[i].status == "failed";
        store.update(env_id, |env| {
            env.journal = Some(json!(journal));
            Ok(())
        })?;
        if failed {
            return Err(format!(
                "{}: {}\n\nThe switch stopped here. Retry it, or roll back to where you started.",
                r.label,
                journal.steps[i].message.clone().unwrap_or_default()
            ));
        }
    }

    // Done: record the active feature and reconcile its repo list with git.
    let (env, _) = store.update(env_id, |env| {
        env.journal = None;
        env.active_feature_id = target.map(str::to_string);
        if let Some(t) = target.and_then(|id| env.features.iter_mut().find(|f| f.id == id)) {
            for r in &env.repos {
                let Ok(repo) = Repository::open(&r.path) else { continue };
                if local_branch_exists(&repo, &t.branch) {
                    if head_branch(&repo).as_deref() == Some(t.branch.as_str()) {
                        t.repos.entry(r.id.clone()).or_default();
                    }
                } else if !pin_exists(&repo, &t.id) {
                    t.repos.remove(&r.id); // deleted outside the app
                }
            }
        }
        Ok(())
    })?;
    drop(guards);
    Ok(SwitchResult { env, warnings })
}

/// One step, checked against git first so it can run again safely.
/// Ok(Some(..)) = done, but the repo needs attention.
fn run_step(store: &SmartStore, env_id: &str, r: &SmartRepo, step: &Step, f: Option<&Feature>) -> CmdResult<Option<String>> {
    let cwd = Path::new(&r.path);
    let repo = Repository::open(cwd).map_err(|_| "can't open the repository".to_string())?;
    let current = head_branch(&repo);
    match step.action.as_str() {
        "park" => {
            let f = f.ok_or("unknown feature")?;
            let tag = stash_tag(&f.id);
            if is_dirty(&repo) {
                let msg = format!("smart-features: {} ({}) {tag}", f.name, f.branch);
                git(cwd, &["stash", "push", "--include-untracked", "-m", &msg])?;
            }
            // Pin it (also a stash made just before an interrupted run).
            if !pin_exists(&repo, &f.id) {
                if let Some(sha) = find_stash(cwd, &tag)? {
                    git(cwd, &["update-ref", "-m", "smart-features: park", &pin_ref(&f.id), &sha])?;
                }
            }
            if let Some(sha) = repo.find_reference(&pin_ref(&f.id)).ok().and_then(|p| p.target()) {
                record_parked(store, env_id, &f.id, &r.id, Some(sha.to_string()))?;
            }
            Ok(None)
        }
        "base" | "switch" => {
            let branch = step.detail.as_deref().ok_or("missing branch")?;
            if current.as_deref() != Some(branch) {
                git(cwd, &["switch", branch])?;
            }
            Ok(None)
        }
        "restore" => {
            let f = f.ok_or("unknown feature")?;
            let Some(sha) = repo.find_reference(&pin_ref(&f.id)).ok().and_then(|p| p.target()) else {
                record_parked(store, env_id, &f.id, &r.id, None)?;
                return Ok(None);
            };
            let sha = sha.to_string();
            // A dirty tree here means an earlier run already applied it.
            if !is_dirty(&repo) {
                let applied = git_out(cwd, &["stash", "apply", "--index", &sha]).or_else(|err| {
                    // Staged changes that can't be restored as staged: apply them unstaged.
                    if err.contains("Conflicts in index") || err.contains("--index") {
                        git_out(cwd, &["stash", "apply", &sha])
                    } else {
                        Err(err)
                    }
                });
                if let Err(err) = applied {
                    let repo = Repository::open(cwd).map_err(|e| e.to_string())?;
                    return Ok(Some(if has_conflicts(&repo) {
                        format!(
                            "parked changes for \"{}\" conflicted. Resolve the conflicts, then choose Mark resolved.",
                            f.name
                        )
                    } else {
                        format!(
                            "couldn't restore the parked changes for \"{}\"; they're kept in {}.\n{err}",
                            f.name,
                            pin_ref(&f.id)
                        )
                    }));
                }
            }
            drop_pin(cwd, &repo, &f.id)?;
            record_parked(store, env_id, &f.id, &r.id, None)?;
            Ok(None)
        }
        other => Err(format!("unknown step {other}")),
    }
}

fn record_parked(store: &SmartStore, env_id: &str, feature_id: &str, repo_id: &str, sha: Option<String>) -> CmdResult<()> {
    store.update(env_id, |env| {
        if let Some(f) = env.features.iter_mut().find(|f| f.id == feature_id) {
            f.repos.entry(repo_id.to_string()).or_default().parked_stash =
                sha.map(|sha| ParkedStash { ref_name: pin_ref(feature_id), sha, parked_at: now_secs() });
        }
        Ok(())
    })?;
    Ok(())
}

pub fn switch(store: &SmartStore, locks: &NetLocks, env_id: &str, target: Option<&str>, progress: ProgressFn) -> CmdResult<SwitchResult> {
    let _op = store.op_lock();
    switch_locked(store, locks, env_id, target, progress)
}

/// Finish an interrupted switch ("retry": on to its target) or undo it
/// ("rollback": back to where it started).
pub fn resume(store: &SmartStore, locks: &NetLocks, env_id: &str, mode: &str, progress: ProgressFn) -> CmdResult<SwitchResult> {
    let _op = store.op_lock();
    let env = store.get(env_id)?;
    let journal = journal_of(&env).ok_or("There's no unfinished switch to resume.")?;
    let target = match mode {
        "retry" => journal.to.clone(),
        "rollback" => journal.from.clone(),
        other => return Err(format!("unknown mode {other}")),
    };
    // A feature that was finished meanwhile can't be a target; fall back to base.
    let target = target.filter(|id| env.features.iter().any(|f| &f.id == id));
    run(store, locks, env_id, target.as_deref(), Some(&journal), progress)
}

/// After resolving conflicted parked changes by hand: forget the pin.
pub fn mark_resolved(store: &SmartStore, env_id: &str, feature_id: &str, repo_id: &str) -> CmdResult<SmartEnv> {
    let _op = store.op_lock();
    let env = store.get(env_id)?;
    let r = env.repo(repo_id)?.clone();
    let repo = Repository::open(&r.path).map_err(|_| format!("{}: can't open the repository", r.label))?;
    if has_conflicts(&repo) {
        return Err(format!("{} still has unresolved conflicts. Resolve them (and stage the files) first.", r.label));
    }
    drop_pin(Path::new(&r.path), &repo, feature_id)?;
    record_parked(store, env_id, feature_id, repo_id, None)?;
    store.get(env_id)
}

// ---------------------------------------------------------------------------
// Finish

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FinishRepo {
    repo_id: String,
    label: String,
    /// The local branch exists.
    local: bool,
    has_upstream: bool,
    /// Commits not on the upstream (or, with no upstream, not on base).
    unpushed: usize,
    /// The branch tip is already in origin/<base>.
    merged: bool,
    /// Has parked changes (or uncommitted changes that will be parked first).
    parked: bool,
    /// Finishing loses work here unless the user ticks it.
    at_risk: bool,
}

fn finish_rows(env: &SmartEnv, f: &Feature) -> Vec<FinishRepo> {
    let active = env.active_feature_id.as_deref() == Some(f.id.as_str());
    let mut rows = Vec::new();
    for r in &env.repos {
        let Ok(repo) = Repository::open(&r.path) else { continue };
        let branch = repo.find_branch(&f.branch, BranchType::Local).ok();
        let on_it = head_branch(&repo).as_deref() == Some(f.branch.as_str());
        let parked = pin_exists(&repo, &f.id) || (active && on_it && is_dirty(&repo));
        if branch.is_none() && !parked && !f.repos.contains_key(&r.id) {
            continue;
        }
        let mut row = FinishRepo {
            repo_id: r.id.clone(),
            label: r.label.clone(),
            local: branch.is_some(),
            has_upstream: false,
            unpushed: 0,
            merged: false,
            parked,
            at_risk: parked,
        };
        if let Some(tip) = branch.as_ref().and_then(|b| b.get().target()) {
            let base = base_of(env, r, &repo).and_then(|b| base_tip(&repo, &b));
            row.merged = base.is_some_and(|b| b == tip || repo.graph_descendant_of(b, tip).unwrap_or(false));
            match branch.as_ref().and_then(|b| b.upstream().ok()).and_then(|u| u.get().target()) {
                Some(up) => {
                    row.has_upstream = true;
                    row.unpushed = repo.graph_ahead_behind(tip, up).map(|(a, _)| a).unwrap_or(0);
                }
                None => {
                    row.unpushed = base.map(|b| repo.graph_ahead_behind(tip, b).map(|(a, _)| a).unwrap_or(0)).unwrap_or(0)
                }
            }
            row.at_risk |= row.unpushed > 0 && !row.merged;
        }
        rows.push(row);
    }
    rows
}

pub fn finish_preview(store: &SmartStore, env_id: &str, feature_id: &str) -> CmdResult<Vec<FinishRepo>> {
    let env = store.get(env_id)?;
    let f = env.features.iter().find(|f| f.id == feature_id).ok_or("That feature no longer exists.")?;
    Ok(finish_rows(&env, f))
}

/// Delete a feature: its local branches and parked changes (never remote
/// branches). Repos where that would lose work must be listed in `discard`.
pub fn finish(
    store: &SmartStore,
    locks: &NetLocks,
    env_id: &str,
    feature_id: &str,
    discard: &[String],
    progress: ProgressFn,
) -> CmdResult<SwitchResult> {
    let _op = store.op_lock();
    let env = store.get(env_id)?;
    let f = env.features.iter().find(|f| f.id == feature_id).cloned().ok_or("That feature no longer exists.")?;
    let rows = finish_rows(&env, &f);
    let blocked: Vec<String> = rows
        .iter()
        .filter(|row| row.at_risk && !discard.contains(&row.repo_id))
        .map(|row| {
            let mut why = Vec::new();
            if row.parked {
                why.push("parked changes".to_string());
            }
            if row.unpushed > 0 && !row.merged {
                why.push(format!("{} unpushed commits", row.unpushed));
            }
            format!("{}: {}", row.label, why.join(", "))
        })
        .collect();
    if !blocked.is_empty() {
        return Err(format!("Finishing would lose work:\n{}", blocked.join("\n")));
    }

    let mut warnings = Vec::new();
    if env.active_feature_id.as_deref() == Some(feature_id) {
        warnings.extend(switch_locked(store, locks, env_id, None, progress)?.warnings);
    }
    let env = store.get(env_id)?;
    for row in &rows {
        let r = env.repo(&row.repo_id)?;
        let cwd = Path::new(&r.path);
        let Ok(repo) = Repository::open(cwd) else { continue };
        if let Err(err) = drop_pin(cwd, &repo, &f.id) {
            warnings.push(format!("{}: couldn't drop the parked changes.\n{err}", r.label));
        }
        if local_branch_exists(&repo, &f.branch) {
            if head_branch(&repo).as_deref() == Some(f.branch.as_str()) {
                warnings.push(format!("{}: still on {}, so the branch was kept.", r.label, f.branch));
                continue;
            }
            // Safe: anything unpushed and unmerged was ticked to discard above.
            if let Err(err) = git(cwd, &["branch", "-D", &f.branch]) {
                warnings.push(format!("{}: couldn't delete {}.\n{err}", r.label, f.branch));
            }
        }
    }
    let (env, _) = store.update(env_id, |env| {
        env.features.retain(|x| x.id != feature_id);
        if env.active_feature_id.as_deref() == Some(feature_id) {
            env.active_feature_id = None;
        }
        Ok(())
    })?;
    Ok(SwitchResult { env, warnings })
}

// ---------------------------------------------------------------------------
// Commands

fn emitter(app: &AppHandle, env_id: &str) -> impl Fn(&Step) {
    let app = app.clone();
    let env_id = env_id.to_string();
    move |step: &Step| {
        let _ = app.emit("sf-progress", json!({ "envId": env_id, "step": step }));
    }
}

#[tauri::command]
pub async fn sf_preflight(store: State<'_, SmartStore>, env_id: String, target_id: Option<String>) -> CmdResult<Preflight> {
    let store = store.inner().clone();
    blocking(move || preflight(&store, &env_id, target_id.as_deref())).await
}

#[tauri::command]
pub async fn sf_switch_feature(
    app: AppHandle,
    store: State<'_, SmartStore>,
    env_id: String,
    target_id: Option<String>,
) -> CmdResult<SwitchResult> {
    let store = store.inner().clone();
    blocking(move || {
        let progress = emitter(&app, &env_id);
        switch(&store, &app.state::<NetLocks>(), &env_id, target_id.as_deref(), &progress)
    })
    .await
}

#[tauri::command]
pub async fn sf_back_to_base(app: AppHandle, store: State<'_, SmartStore>, env_id: String) -> CmdResult<SwitchResult> {
    let store = store.inner().clone();
    blocking(move || {
        let progress = emitter(&app, &env_id);
        switch(&store, &app.state::<NetLocks>(), &env_id, None, &progress)
    })
    .await
}

#[tauri::command]
pub async fn sf_resume_switch(app: AppHandle, store: State<'_, SmartStore>, env_id: String, mode: String) -> CmdResult<SwitchResult> {
    let store = store.inner().clone();
    blocking(move || {
        let progress = emitter(&app, &env_id);
        resume(&store, &app.state::<NetLocks>(), &env_id, &mode, &progress)
    })
    .await
}

#[tauri::command]
pub async fn sf_mark_resolved(
    store: State<'_, SmartStore>,
    env_id: String,
    feature_id: String,
    repo_id: String,
) -> CmdResult<SmartEnv> {
    let store = store.inner().clone();
    blocking(move || mark_resolved(&store, &env_id, &feature_id, &repo_id)).await
}

#[tauri::command]
pub async fn sf_finish_preview(store: State<'_, SmartStore>, env_id: String, feature_id: String) -> CmdResult<Vec<FinishRepo>> {
    let store = store.inner().clone();
    blocking(move || finish_preview(&store, &env_id, &feature_id)).await
}

#[tauri::command]
pub async fn sf_finish_feature(
    app: AppHandle,
    store: State<'_, SmartStore>,
    env_id: String,
    feature_id: String,
    discard: Vec<String>,
) -> CmdResult<SwitchResult> {
    let store = store.inner().clone();
    blocking(move || {
        let progress = emitter(&app, &env_id);
        finish(&store, &app.state::<NetLocks>(), &env_id, &feature_id, &discard, &progress)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::smart::tests::git as sh;
    use crate::smart_features::tests::{head, repo_id, setup};
    use crate::smart_features::{auto_branch, start_feature};
    use std::path::PathBuf;

    fn locks() -> NetLocks {
        NetLocks::default()
    }

    fn read(p: &Path, f: &str) -> Option<String> {
        std::fs::read_to_string(p.join(f)).ok()
    }

    fn status(p: &Path, f: &str) -> Status {
        Repository::open(p).unwrap().status_file(Path::new(f)).unwrap()
    }

    fn active_id(store: &SmartStore, env_id: &str) -> String {
        store.get(env_id).unwrap().active_feature_id.unwrap()
    }

    fn pins(p: &Path) -> String {
        git_out(p, &["for-each-ref", "--format=%(refname)", PIN_NS]).unwrap()
    }

    /// Start feature `branch` and edit `repos` so they join it.
    fn start_with_edits(store: &SmartStore, env_id: &str, branch: &str, repos: &[(usize, &PathBuf)]) -> String {
        start_feature(store, &locks(), env_id, "", branch, false).unwrap();
        for (i, p) in repos {
            std::fs::write(p.join(format!("{branch}.txt").replace('/', "-")), "x\n").unwrap();
            auto_branch(store, env_id, &repo_id(store, env_id, *i), false).unwrap();
            sh(p, &["add", "."]);
            sh(p, &["commit", "-qm", branch]);
        }
        active_id(store, env_id)
    }

    #[test]
    fn switch_a_b_a_restores_everything() {
        let (_s, store, env_id, p) = setup("switch", &["a", "b", "c"]);
        let a_id = start_with_edits(&store, &env_id, "feat/a", &[(0, &p[0]), (1, &p[1])]);
        // Uncommitted work of every kind in a: staged, staged+unstaged, untracked.
        std::fs::write(p[0].join("a.txt"), "staged\n").unwrap();
        sh(&p[0], &["add", "a.txt"]);
        std::fs::write(p[0].join("a.txt"), "staged then edited\n").unwrap();
        std::fs::write(p[0].join("new.txt"), "untracked\n").unwrap();
        std::fs::create_dir_all(p[0].join("dir")).unwrap();
        std::fs::write(p[0].join("dir/deep.txt"), "untracked dir\n").unwrap();

        // Start B: A is parked, every repo is back on base and clean.
        let started = start_feature(&store, &locks(), &env_id, "", "feat/b", false).unwrap();
        assert!(started.warnings.is_empty(), "{:?}", started.warnings);
        for repo in &p {
            assert_eq!(head(repo).as_deref(), Some("main"));
            assert!(!is_dirty(&Repository::open(repo).unwrap()));
        }
        assert_eq!(pins(&p[0]).trim(), pin_ref(&a_id));
        assert!(pins(&p[1]).is_empty(), "b was clean: nothing parked");
        let env = store.get(&env_id).unwrap();
        let a = env.features.iter().find(|f| f.id == a_id).unwrap();
        assert!(a.repos[&env.repos[0].id].parked_stash.is_some());
        // The stash is listed too, with a readable message.
        assert!(git_out(&p[0], &["stash", "list"]).unwrap().contains("smart-features: feat/a (feat/a)"));

        // Work on B in a, then go back to A.
        let b_id = active_id(&store, &env_id);
        std::fs::write(p[0].join("b-work.txt"), "b\n").unwrap();
        auto_branch(&store, &env_id, &repo_id(&store, &env_id, 0), false).unwrap().unwrap();
        let out = switch(&store, &locks(), &env_id, Some(&a_id), &|_| {}).unwrap();
        assert!(out.warnings.is_empty(), "{:?}", out.warnings);
        assert_eq!(head(&p[0]).as_deref(), Some("feat/a"));
        assert_eq!(head(&p[1]).as_deref(), Some("feat/a"));
        assert_eq!(head(&p[2]).as_deref(), Some("main"), "c never joined A");
        assert_eq!(read(&p[0], "a.txt").as_deref(), Some("staged then edited\n"));
        let st = status(&p[0], "a.txt");
        assert!(st.contains(Status::INDEX_MODIFIED) && st.contains(Status::WT_MODIFIED), "{st:?}");
        assert_eq!(read(&p[0], "new.txt").as_deref(), Some("untracked\n"));
        assert_eq!(read(&p[0], "dir/deep.txt").as_deref(), Some("untracked dir\n"));
        assert!(read(&p[0], "b-work.txt").is_none(), "B's work is parked, not mixed in");
        assert_eq!(pins(&p[0]).trim(), pin_ref(&b_id), "A's pin is gone; B's is parked");
        assert!(!git_out(&p[0], &["stash", "list"]).unwrap().contains("feat/a"), "A's stash entry dropped");

        // And back to B.
        switch(&store, &locks(), &env_id, Some(&b_id), &|_| {}).unwrap();
        assert_eq!(read(&p[0], "b-work.txt").as_deref(), Some("b\n"));
        assert!(read(&p[0], "new.txt").is_none());
        // Back to base parks B.
        switch(&store, &locks(), &env_id, None, &|_| {}).unwrap();
        assert_eq!(head(&p[0]).as_deref(), Some("main"));
        assert!(store.get(&env_id).unwrap().active_feature_id.is_none());
    }

    #[test]
    fn pins_survive_gc() {
        let (_s, store, env_id, p) = setup("gc", &["a"]);
        let a_id = start_with_edits(&store, &env_id, "feat/a", &[(0, &p[0])]);
        std::fs::write(p[0].join("wip.txt"), "keep me\n").unwrap();
        switch(&store, &locks(), &env_id, None, &|_| {}).unwrap();
        // Lose the stash list entirely; the pin still holds the commit.
        sh(&p[0], &["stash", "clear"]);
        sh(&p[0], &["reflog", "expire", "--expire=now", "--all"]);
        sh(&p[0], &["gc", "-q", "--prune=now"]);
        switch(&store, &locks(), &env_id, Some(&a_id), &|_| {}).unwrap();
        assert_eq!(read(&p[0], "wip.txt").as_deref(), Some("keep me\n"));
    }

    #[test]
    fn interrupted_switch_retries_and_rolls_back() {
        let (_s, store, env_id, p) = setup("journal", &["a", "b"]);
        let a_id = start_with_edits(&store, &env_id, "feat/a", &[(0, &p[0]), (1, &p[1])]);
        std::fs::write(p[1].join("wip.txt"), "a's wip\n").unwrap();
        switch(&store, &locks(), &env_id, None, &|_| {}).unwrap();
        let b_id = start_with_edits(&store, &env_id, "feat/b", &[(0, &p[0]), (1, &p[1])]);
        std::fs::write(p[0].join("bwip.txt"), "b's wip\n").unwrap();

        // Make git fail when b is about to check out feat/a.
        let lock = p[1].join(".git/index.lock");
        let fail = |s: &Step| {
            if s.label == "b" && s.action == "switch" && s.status == "pending" {
                std::fs::write(&lock, "").unwrap();
            }
        };
        let err = switch(&store, &locks(), &env_id, Some(&a_id), &fail).unwrap_err();
        assert!(err.contains("The switch stopped here"), "{err}");
        let env = store.get(&env_id).unwrap();
        let j = journal_of(&env).unwrap();
        assert_eq!((j.from.as_deref(), j.to.as_deref()), (Some(b_id.as_str()), Some(a_id.as_str())));
        assert!(j.steps.iter().any(|s| s.status == "failed"));
        assert!(j.steps.iter().any(|s| s.status == "done"));
        // Nothing else may run on the environment meanwhile.
        assert!(switch(&store, &locks(), &env_id, None, &|_| {}).unwrap_err().contains("didn't finish"));
        assert!(crate::smart_features::switch_running(&store, &p[0].to_string_lossy()).is_some());

        // Roll back: B again, with its parked work restored.
        std::fs::remove_file(&lock).unwrap();
        resume(&store, &locks(), &env_id, "rollback", &|_| {}).unwrap();
        let env = store.get(&env_id).unwrap();
        assert!(env.journal.is_none());
        assert_eq!(env.active_feature_id.as_deref(), Some(b_id.as_str()));
        assert_eq!(head(&p[0]).as_deref(), Some("feat/b"));
        assert_eq!(head(&p[1]).as_deref(), Some("feat/b"));
        assert_eq!(read(&p[0], "bwip.txt").as_deref(), Some("b's wip\n"));

        // Fail again, then Retry: on to A.
        let err = switch(&store, &locks(), &env_id, Some(&a_id), &fail);
        assert!(err.is_err());
        std::fs::remove_file(&lock).unwrap();
        resume(&store, &locks(), &env_id, "retry", &|_| {}).unwrap();
        assert_eq!(head(&p[1]).as_deref(), Some("feat/a"));
        assert_eq!(read(&p[1], "wip.txt").as_deref(), Some("a's wip\n"));
        assert!(read(&p[0], "bwip.txt").is_none());
        assert!(store.get(&env_id).unwrap().journal.is_none());
    }

    #[test]
    fn stash_made_before_a_crash_is_still_pinned() {
        let (_s, store, env_id, p) = setup("crash", &["a"]);
        let a_id = start_with_edits(&store, &env_id, "feat/a", &[(0, &p[0])]);
        std::fs::write(p[0].join("wip.txt"), "w\n").unwrap();
        // The app stashed, then died before pinning.
        let msg = format!("smart-features: feat/a (feat/a) {}", stash_tag(&a_id));
        sh(&p[0], &["stash", "push", "-q", "--include-untracked", "-m", &msg]);
        switch(&store, &locks(), &env_id, None, &|_| {}).unwrap();
        assert_eq!(pins(&p[0]).trim(), pin_ref(&a_id));
        switch(&store, &locks(), &env_id, Some(&a_id), &|_| {}).unwrap();
        assert_eq!(read(&p[0], "wip.txt").as_deref(), Some("w\n"));
    }

    #[test]
    fn conflicting_restore_keeps_the_stash() {
        let (_s, store, env_id, p) = setup("conflict", &["a"]);
        let a_id = start_with_edits(&store, &env_id, "feat/a", &[(0, &p[0])]);
        std::fs::write(p[0].join("a.txt"), "parked version\n").unwrap();
        switch(&store, &locks(), &env_id, None, &|_| {}).unwrap();
        // While parked, feat/a moves on with a different a.txt.
        sh(&p[0], &["switch", "-q", "feat/a"]);
        std::fs::write(p[0].join("a.txt"), "teammate version\n").unwrap();
        sh(&p[0], &["commit", "-qam", "other"]);
        sh(&p[0], &["switch", "-q", "main"]);

        let out = switch(&store, &locks(), &env_id, Some(&a_id), &|_| {}).unwrap();
        assert_eq!(out.warnings.len(), 1, "{:?}", out.warnings);
        assert!(out.warnings[0].contains("conflicted"));
        assert!(has_conflicts(&Repository::open(&p[0]).unwrap()));
        assert_eq!(pins(&p[0]).trim(), pin_ref(&a_id), "the stash is kept");
        let env = store.get(&env_id).unwrap();
        let st = crate::smart::env_status(&env);
        assert!(st[0].warning.as_deref().unwrap_or("").contains("Mark resolved") || st[0].conflicted > 0);

        // Can't mark resolved with conflicts left; can after resolving.
        assert!(mark_resolved(&store, &env_id, &a_id, &env.repos[0].id).is_err());
        std::fs::write(p[0].join("a.txt"), "merged\n").unwrap();
        sh(&p[0], &["add", "a.txt"]);
        mark_resolved(&store, &env_id, &a_id, &env.repos[0].id).unwrap();
        assert!(pins(&p[0]).is_empty());
    }

    #[test]
    fn unknown_branch_blocks_the_switch() {
        let (_s, store, env_id, p) = setup("blocked", &["a", "b"]);
        start_feature(&store, &locks(), &env_id, "", "feat/a", false).unwrap();
        sh(&p[1], &["switch", "-qc", "random"]);
        let err = switch(&store, &locks(), &env_id, None, &|_| {}).unwrap_err();
        assert!(err.contains("b: on random"), "{err}");
        assert!(store.get(&env_id).unwrap().journal.is_none(), "nothing ran");
        let pre = preflight(&store, &env_id, None).unwrap();
        assert_eq!(pre.problems.len(), 1);
    }

    #[test]
    fn finish_blocks_on_lost_work_and_deletes_branches() {
        let (s, store, env_id, p) = setup("finish", &["a", "b"]);
        let a_id = start_with_edits(&store, &env_id, "feat/a", &[(0, &p[0]), (1, &p[1])]);
        // a: pushed. b: an unpushed commit plus uncommitted work (parked on finish).
        sh(&p[0], &["push", "-q", "-u", "origin", "feat/a"]);
        std::fs::write(p[1].join("wip.txt"), "w\n").unwrap();

        let rows = finish_preview(&store, &env_id, &a_id).unwrap();
        let by = |l: &str| rows.iter().find(|r| r.label == l).unwrap();
        assert!(!by("a").at_risk);
        assert!(by("b").at_risk && by("b").parked && by("b").unpushed == 1);
        let err = finish(&store, &locks(), &env_id, &a_id, &[], &|_| {}).unwrap_err();
        assert!(err.contains("b: parked changes, 1 unpushed commits"), "{err}");
        assert_eq!(head(&p[1]).as_deref(), Some("feat/a"), "nothing ran");

        let b = repo_id(&store, &env_id, 1);
        let out = finish(&store, &locks(), &env_id, &a_id, &[b], &|_| {}).unwrap();
        assert!(out.warnings.is_empty(), "{:?}", out.warnings);
        assert!(out.env.features.is_empty() && out.env.active_feature_id.is_none());
        for repo in &p {
            assert_eq!(head(repo).as_deref(), Some("main"));
            assert!(!local_branch_exists(&Repository::open(repo).unwrap(), "feat/a"));
            assert!(pins(repo).is_empty());
        }
        assert!(git_out(&p[1], &["stash", "list"]).unwrap().is_empty());
        // Remote branches are never deleted.
        let origin = s.path("remotes/a.git");
        assert!(git_out(&origin, &["branch", "--list", "feat/a"]).unwrap().contains("feat/a"));
    }
}
