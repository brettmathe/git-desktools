//! Watches every Smart Environment repo for file changes (FSEvents on macOS) and
//! moves changed repos onto the active feature's branch, so auto-branching works
//! while the panel is closed or the repo has no tab. A sweep every few seconds
//! catches anything the watcher missed.
//!
//! Events sent to the frontend:
//!   sf-repo-changed        {envId, repoId}            refresh that repo's status
//!   sf-auto-branched       AutoBranched               a repo moved onto the feature branch
//!   sf-auto-branch-failed  {envId, repoId, error}     sent once per distinct error

use crate::smart::SmartStore;
use crate::smart_features::{auto_branch, sweep};
use git2::Repository;
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

const DEBOUNCE: Duration = Duration::from_millis(300);
/// Flush even while events keep coming (a long build), so branching isn't starved.
const MAX_DELAY: Duration = Duration::from_secs(2);
const SWEEP_EVERY: Duration = Duration::from_secs(10);
const MAX_PATHS: usize = 64;

pub type Emit = Box<dyn Fn(&'static str, Value) + Send>;

enum Msg {
    Fs(notify::Result<Event>),
    Resync,
}

/// Handle kept in app state. Call `resync` after environments change.
pub struct SmartWatcher {
    tx: Sender<Msg>,
}

impl SmartWatcher {
    pub fn resync(&self) {
        let _ = self.tx.send(Msg::Resync);
    }
}

pub fn spawn(store: SmartStore, emit: impl Fn(&'static str, Value) + Send + 'static) -> SmartWatcher {
    let (tx, rx) = channel();
    let fs_tx = tx.clone();
    let emit: Emit = Box::new(emit);
    std::thread::Builder::new()
        .name("smart-watch".into())
        .spawn(move || {
            let watcher = notify::recommended_watcher(move |res| {
                let _ = fs_tx.send(Msg::Fs(res));
            })
            .ok();
            Loop::new(store, emit, watcher).run(rx);
        })
        .expect("spawn smart-watch thread");
    let _ = tx.send(Msg::Resync);
    SmartWatcher { tx }
}

type Key = (String, String); // (env id, repo id)

#[derive(Default)]
struct Pending {
    /// Changed working-tree paths (capped).
    work: Vec<PathBuf>,
    work_overflow: bool,
    /// Something under .git changed (HEAD, refs, index): status only.
    git: bool,
}

struct Loop {
    store: SmartStore,
    emit: Emit,
    watcher: Option<RecommendedWatcher>,
    roots: HashMap<PathBuf, Key>,
    pending: HashMap<Key, Pending>,
    first: Option<Instant>,
    last: Option<Instant>,
    next_sweep: Instant,
    last_error: HashMap<Key, String>,
}

impl Loop {
    fn new(store: SmartStore, emit: Emit, watcher: Option<RecommendedWatcher>) -> Self {
        Loop {
            store,
            emit,
            watcher,
            roots: HashMap::new(),
            pending: HashMap::new(),
            first: None,
            last: None,
            next_sweep: Instant::now() + SWEEP_EVERY,
            last_error: HashMap::new(),
        }
    }

    fn run(mut self, rx: Receiver<Msg>) {
        loop {
            let now = Instant::now();
            let mut due = self.next_sweep;
            if let (Some(first), Some(last)) = (self.first, self.last) {
                due = due.min((last + DEBOUNCE).min(first + MAX_DELAY));
            }
            match rx.recv_timeout(due.saturating_duration_since(now)) {
                Ok(Msg::Fs(Ok(event))) => self.note(event),
                Ok(Msg::Fs(Err(_))) | Err(RecvTimeoutError::Timeout) => {}
                Ok(Msg::Resync) => self.resync(),
                Err(RecvTimeoutError::Disconnected) => return,
            }
            let now = Instant::now();
            if let (Some(first), Some(last)) = (self.first, self.last) {
                if now >= last + DEBOUNCE || now >= first + MAX_DELAY {
                    self.flush();
                }
            }
            if now >= self.next_sweep {
                self.resync();
                self.sweep();
                self.next_sweep = Instant::now() + SWEEP_EVERY;
            }
        }
    }

    /// Watch exactly the repos the environments list (that exist on disk).
    fn resync(&mut self) {
        let mut want: HashMap<PathBuf, Key> = HashMap::new();
        for env in self.store.load_all().envs {
            for r in &env.repos {
                let p = PathBuf::from(&r.path);
                if p.is_dir() {
                    want.insert(p, (env.id.clone(), r.id.clone()));
                }
            }
        }
        let Some(watcher) = self.watcher.as_mut() else {
            self.roots = want;
            return;
        };
        for root in self.roots.keys().filter(|p| !want.contains_key(*p)).cloned().collect::<Vec<_>>() {
            let _ = watcher.unwatch(&root);
            self.roots.remove(&root);
        }
        for (root, key) in want {
            if let Some(k) = self.roots.get_mut(&root) {
                *k = key;
            } else if watcher.watch(&root, RecursiveMode::Recursive).is_ok() {
                self.roots.insert(root, key);
            }
        }
    }

    fn note(&mut self, event: Event) {
        if matches!(event.kind, EventKind::Access(_)) {
            return;
        }
        for path in event.paths {
            // Longest root wins, in case one environment repo sits inside another's folder.
            let Some((root, key)) =
                self.roots.iter().filter(|(root, _)| path.starts_with(root)).max_by_key(|(root, _)| root.as_os_str().len())
            else {
                continue;
            };
            let in_git = matches!(path.strip_prefix(root).ok().and_then(|rel| rel.components().next()),
                Some(Component::Normal(c)) if c == ".git");
            let p = self.pending.entry(key.clone()).or_default();
            if in_git {
                p.git = true;
            } else if p.work.len() < MAX_PATHS {
                p.work.push(path);
            } else {
                p.work_overflow = true;
            }
            let now = Instant::now();
            self.first.get_or_insert(now);
            self.last = Some(now);
        }
    }

    fn flush(&mut self) {
        self.first = None;
        self.last = None;
        for (key, p) in std::mem::take(&mut self.pending) {
            let root = self.roots.iter().find(|(_, k)| **k == key).map(|(r, _)| r.clone());
            let work = !p.work.is_empty() && (p.work_overflow || root.is_none_or(|r| !all_ignored(&r, &p.work)));
            if work {
                self.branch(&key);
            }
            if work || p.git {
                (self.emit)("sf-repo-changed", json!({ "envId": key.0, "repoId": key.1 }));
            }
        }
    }

    fn branch(&mut self, key: &Key) {
        let result = auto_branch(&self.store, &key.0, &key.1, false);
        self.report(key, result.map(|r| r.map(|d| json!(d))));
    }

    fn sweep(&mut self) {
        for (env_id, repo_id, result) in sweep(&self.store) {
            self.report(&(env_id, repo_id), result.map(|d| Some(json!(d))));
        }
    }

    fn report(&mut self, key: &Key, result: Result<Option<Value>, String>) {
        match result {
            Ok(Some(done)) => {
                self.last_error.remove(key);
                (self.emit)("sf-auto-branched", done);
                (self.emit)("sf-repo-changed", json!({ "envId": key.0, "repoId": key.1 }));
            }
            Ok(None) => {
                self.last_error.remove(key);
            }
            Err(error) => {
                if self.last_error.get(key) != Some(&error) {
                    self.last_error.insert(key.clone(), error.clone());
                    (self.emit)("sf-auto-branch-failed", json!({ "envId": key.0, "repoId": key.1, "error": error }));
                }
            }
        }
    }
}

/// True when every changed path is gitignored (build output, node_modules...),
/// which can never make the repo dirty.
fn all_ignored(root: &Path, paths: &[PathBuf]) -> bool {
    let Ok(repo) = Repository::open(root) else { return false };
    paths.iter().all(|p| p.strip_prefix(root).map(|rel| repo.is_path_ignored(rel).unwrap_or(false)).unwrap_or(false))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::smart::tests::{env_with, repo_with_origin, Scratch};
    use std::sync::{Arc, Mutex};

    /// End to end: a file written into a repo on base, with a feature active,
    /// lands on the feature branch without anyone asking.
    #[test]
    fn watcher_auto_branches_on_file_change() {
        let s = Scratch::new("watch");
        let a = repo_with_origin(&s, "a", "main");
        let b = repo_with_origin(&s, "b", "main");
        let store = SmartStore::new(s.path("cfg"));
        let env = store.save(env_with("E", &[&a, &b])).unwrap();
        crate::smart_features::start_feature(&store, &crate::git::NetLocks::default(), &env.id, "", "feat/watch", false).unwrap();

        let events: Arc<Mutex<Vec<(String, Value)>>> = Arc::default();
        let sink = events.clone();
        let _w = spawn(store.clone(), move |name, payload| sink.lock().unwrap().push((name.to_string(), payload)));
        std::thread::sleep(Duration::from_millis(500)); // let the watch register

        std::fs::write(a.join("a.txt"), "edited\n").unwrap();
        let deadline = Instant::now() + Duration::from_secs(8);
        let branched = loop {
            let head = Repository::open(&a).ok().and_then(|r| crate::git::head_branch(&r));
            if head.as_deref() == Some("feat/watch") || Instant::now() > deadline {
                break head;
            }
            std::thread::sleep(Duration::from_millis(100));
        };
        assert_eq!(branched.as_deref(), Some("feat/watch"));
        // The edit came along, and the untouched repo stayed on base.
        assert_eq!(std::fs::read_to_string(a.join("a.txt")).unwrap(), "edited\n");
        assert_eq!(crate::git::head_branch(&Repository::open(&b).unwrap()).as_deref(), Some("main"));
        std::thread::sleep(Duration::from_millis(300));
        let ev = events.lock().unwrap();
        assert!(ev.iter().any(|(n, _)| n == "sf-auto-branched"), "{ev:?}");
        let saved = store.get(&env.id).unwrap();
        assert_eq!(saved.active_feature().unwrap().repos.len(), 1);
    }

    #[test]
    fn ignored_paths_are_ignored() {
        let s = Scratch::new("ignored");
        let a = repo_with_origin(&s, "a", "main");
        std::fs::write(a.join(".gitignore"), "out/\n").unwrap();
        assert!(all_ignored(&a, &[a.join("out/x.o"), a.join("out/deep/y")]));
        assert!(!all_ignored(&a, &[a.join("out/x.o"), a.join("src/main.rs")]));
    }
}
