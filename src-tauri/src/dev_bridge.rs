//! Test-only HTTP bridge so the UI can be driven in a plain browser against the
//! real git backend (used for automated UI checks; never compiled into the app).
//!
//!   GDT_BRIDGE_SETTINGS='{"openTabs":["/path/to/repo"]}' \
//!     cargo test dev_bridge -- --ignored --nocapture
//!
//! then open the Vite dev server (http://localhost:1420) in a browser.
//!
//! Commands that write to a repo or touch the network are refused unless the
//! repo lives under `GDT_BRIDGE_WRITE_ROOT`, so a UI test can never push, fetch
//! or commit in a real repository.
//!
//! Smart Environments are stored under `GDT_BRIDGE_CONFIG_DIR` (point it at a
//! scratch folder); without it, saving or deleting an environment is refused so
//! a test can't touch the real app config. Folder pickers answer with the path
//! in `GDT_BRIDGE_PICK`. Feature commands that run git (start, back to base,
//! push all...) are refused unless every repo of the environment is under
//! `GDT_BRIDGE_WRITE_ROOT`.

use crate::git::{self, NetLocks};
use crate::smart::{self, SmartStore};
use crate::smart_features as sf;
use crate::smart_switch as sw;
use crate::smart_watch::{self, SmartWatcher};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

struct Bridge {
    locks: NetLocks,
    settings: Mutex<Value>,
    write_root: Option<String>,
    smart: Option<SmartStore>,
    watcher: Option<SmartWatcher>,
}

const MUTATING: &[&str] = &[
    "init_repo",
    "stage_files",
    "unstage_files",
    "commit",
    "clone_repo",
    "fetch_repo",
    "pull_repo",
    "push_repo",
];

const SF_MUTATING: &[&str] =
    &[
        "sf_start_feature",
        "sf_preflight",
        "sf_back_to_base",
        "sf_move_to_feature",
        "sf_checkout_remote",
        "sf_push_all",
        "sf_switch_feature",
        "sf_resume_switch",
        "sf_mark_resolved",
        "sf_finish_feature",
    ];

fn s(a: &Value, k: &str) -> String {
    a.get(k).and_then(Value::as_str).unwrap_or_default().to_string()
}
fn opt(a: &Value, k: &str) -> Option<String> {
    a.get(k).and_then(Value::as_str).map(str::to_string)
}
fn flag(a: &Value, k: &str) -> bool {
    a.get(k).and_then(Value::as_bool).unwrap_or(false)
}
fn list(a: &Value, k: &str) -> Vec<String> {
    a.get(k)
        .and_then(Value::as_array)
        .map(|v| v.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
        .unwrap_or_default()
}

fn to<T: serde::Serialize>(r: Result<T, String>) -> Result<Value, String> {
    r.map(|v| serde_json::to_value(v).unwrap())
}

fn dispatch(b: &Bridge, cmd: &str, a: &Value) -> Result<Value, String> {
    use tauri::async_runtime::block_on;
    if MUTATING.contains(&cmd) {
        let target = if cmd == "clone_repo" { s(a, "parentDir") } else { s(a, "path") };
        let allowed = b.write_root.as_deref().is_some_and(|root| !root.is_empty() && target.starts_with(root));
        if !allowed {
            return Err(format!("dev bridge: refusing {cmd} outside GDT_BRIDGE_WRITE_ROOT ({target})"));
        }
    }
    match cmd {
        "open_repo" => to(block_on(git::open_repo(s(a, "path")))),
        "init_repo" => to(block_on(git::init_repo(s(a, "path")))),
        "default_clone_dir" => Ok(json!(git::default_clone_dir())),
        "repo_summary" => to(block_on(git::repo_summary(s(a, "path")))),
        "repo_fingerprint" => to(block_on(git::repo_fingerprint(s(a, "path")))),
        "load_repo" => to(block_on(git::load_repo(s(a, "path")))),
        "commit_detail" => to(block_on(git::commit_detail(s(a, "path"), s(a, "oid")))),
        "file_diff" => to(block_on(git::file_diff(s(a, "path"), s(a, "oid"), s(a, "file"), opt(a, "oldFile")))),
        "wip_file_diff" => to(block_on(git::wip_file_diff(
            s(a, "path"),
            s(a, "file"),
            opt(a, "oldFile"),
            flag(a, "staged"),
        ))),
        "file_content" => to(block_on(git::file_content(s(a, "path"), opt(a, "oid"), s(a, "file")))),
        "wip_status" => to(block_on(git::wip_status(s(a, "path")))),
        "stage_files" => to(block_on(git::stage_files(s(a, "path"), list(a, "files")))),
        "unstage_files" => to(block_on(git::unstage_files(s(a, "path"), list(a, "files")))),
        "head_message" => to(block_on(git::head_message(s(a, "path")))),
        "commit" => to(block_on(git::commit_impl(b.smart.clone(), s(a, "path"), s(a, "message"), flag(a, "amend")))),
        "clone_repo" => to(block_on(git::clone_impl(s(a, "url"), s(a, "parentDir"), s(a, "name"), None))),
        "fetch_repo" => to(block_on(git::fetch_impl(&b.locks, s(a, "path"), flag(a, "quiet")))),
        "pull_repo" => to(block_on(git::pull_impl(&b.locks, s(a, "path"), None))),
        "push_repo" => to(block_on(git::push_impl(&b.locks, s(a, "path"), opt(a, "remote"), opt(a, "branch"), None))),
        "load_settings" => Ok(b.settings.lock().unwrap().clone()),
        "save_settings" => {
            *b.settings.lock().unwrap() = a.get("settings").cloned().unwrap_or(Value::Null);
            Ok(Value::Null)
        }
        cmd if cmd.starts_with("sf_") => smart_dispatch(b, cmd, a),
        // Native folder pickers can't open in a browser; answer with GDT_BRIDGE_PICK.
        "plugin:dialog|open" | "plugin:dialog%7Copen" => Ok(json!(std::env::var("GDT_BRIDGE_PICK").ok())),
        other => Err(format!("dev bridge: unsupported command {other}")),
    }
}

fn smart_dispatch(b: &Bridge, cmd: &str, a: &Value) -> Result<Value, String> {
    let Some(store) = b.smart.as_ref() else {
        return Err(format!("dev bridge: refusing {cmd} without GDT_BRIDGE_CONFIG_DIR"));
    };
    let env_id = opt(a, "envId");
    // Feature commands run git in every repo of the environment (and the watcher
    // follows an active feature), so all of its repos must be scratch repos.
    if SF_MUTATING.contains(&cmd) {
        let env = store.get(&s(a, "envId"))?;
        let root = b.write_root.as_deref().filter(|r| !r.is_empty());
        if let Some(r) = env.repos.iter().find(|r| !root.is_some_and(|root| r.path.starts_with(root))) {
            return Err(format!("dev bridge: refusing {cmd}: {} is outside GDT_BRIDGE_WRITE_ROOT", r.path));
        }
    }
    match cmd {
        "sf_list_envs" => to(Ok(smart::bridge::list(store))),
        "sf_save_env" => {
            let saved = store.save(serde_json::from_value(a.get("env").cloned().unwrap_or(Value::Null)).map_err(|e| e.to_string())?);
            b.watcher.as_ref().map(SmartWatcher::resync);
            to(saved)
        }
        "sf_delete_env" => {
            let done = store.delete(&s(a, "id"));
            b.watcher.as_ref().map(SmartWatcher::resync);
            to(done)
        }
        "sf_validate_repo" => to(smart::bridge::validate(store, &s(a, "path"), env_id.as_deref())),
        "sf_scan_folder" => to(Ok(smart::bridge::scan_folder(store, &s(a, "path"), env_id.as_deref()))),
        "sf_status" => to(smart::bridge::status(store, &s(a, "envId"))),
        "sf_check_branch" => to(sf::check_branch(&store.get(&s(a, "envId"))?, &s(a, "branch"))),
        "sf_start_feature" => {
            to(sf::start_feature(store, &b.locks, &s(a, "envId"), &s(a, "name"), &s(a, "branch"), flag(a, "adopt")))
        }
        "sf_back_to_base" => to(sw::switch(store, &b.locks, &s(a, "envId"), None, &|_| {})),
        "sf_preflight" => to(sw::preflight(store, &s(a, "envId"), opt(a, "targetId").as_deref())),
        "sf_switch_feature" => to(sw::switch(store, &b.locks, &s(a, "envId"), opt(a, "targetId").as_deref(), &|_| {})),
        "sf_resume_switch" => to(sw::resume(store, &b.locks, &s(a, "envId"), &s(a, "mode"), &|_| {})),
        "sf_mark_resolved" => to(sw::mark_resolved(store, &s(a, "envId"), &s(a, "featureId"), &s(a, "repoId"))),
        "sf_finish_preview" => to(sw::finish_preview(store, &s(a, "envId"), &s(a, "featureId"))),
        "sf_finish_feature" => {
            to(sw::finish(store, &b.locks, &s(a, "envId"), &s(a, "featureId"), &list(a, "discard"), &|_| {}))
        }
        "sf_move_to_feature" => to(sf::auto_branch(store, &s(a, "envId"), &s(a, "repoId"), true)),
        "sf_commit_guard" => to(Ok(sf::commit_guard(store, &s(a, "path")))),
        "sf_checkout_remote" => to(sf::checkout_remote(store, &s(a, "envId"), &s(a, "repoId"))),
        "sf_rename_feature" => to(sf::rename_feature(store, &s(a, "envId"), &s(a, "featureId"), &s(a, "name"))),
        "sf_push_all" => to(sf::push_all(store, &b.locks, &s(a, "envId"), &s(a, "featureId"))),
        other => Err(format!("dev bridge: unsupported command {other}")),
    }
}

fn respond(stream: &mut TcpStream, status: &str, body: &str) {
    let _ = write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\n\
         Access-Control-Allow-Headers: content-type\r\nAccess-Control-Allow-Methods: POST, OPTIONS\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
}

fn handle(b: &Bridge, mut stream: TcpStream) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut first = String::new();
    if reader.read_line(&mut first).is_err() {
        return;
    }
    let mut len = 0usize;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).is_err() || line.trim().is_empty() {
            break;
        }
        if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
            len = v.trim().parse().unwrap_or(0);
        }
    }
    let mut body = vec![0u8; len];
    let _ = reader.read_exact(&mut body);

    let mut parts = first.split_whitespace();
    let method = parts.next().unwrap_or("");
    let target = parts.next().unwrap_or("");
    if method == "OPTIONS" {
        return respond(&mut stream, "204 No Content", "");
    }
    let Some(cmd) = target.strip_prefix("/cmd/") else {
        return respond(&mut stream, "404 Not Found", "\"not found\"");
    };
    let args: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    match dispatch(b, cmd, &args) {
        Ok(v) => respond(&mut stream, "200 OK", &v.to_string()),
        Err(e) => respond(&mut stream, "500 Internal Server Error", &json!(e).to_string()),
    }
}

#[test]
#[ignore]
fn dev_bridge() {
    let port: u16 = std::env::var("GDT_BRIDGE_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(1421);
    let settings = std::env::var("GDT_BRIDGE_SETTINGS")
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(Value::Null);
    let write_root = std::env::var("GDT_BRIDGE_WRITE_ROOT").ok();
    let smart = std::env::var("GDT_BRIDGE_CONFIG_DIR").ok().map(|d| SmartStore::new(d.into()));
    // The browser can't receive backend events; the panel's polling picks up changes.
    // GDT_BRIDGE_NO_WATCH=1 turns auto-branching off (to test the commit guard).
    let watch = std::env::var("GDT_BRIDGE_NO_WATCH").is_err();
    let watcher = smart
        .clone()
        .filter(|_| watch)
        .map(|store| smart_watch::spawn(store, |event, payload| println!("event {event}: {payload}")));
    let bridge =
        Arc::new(Bridge { locks: NetLocks::default(), settings: Mutex::new(settings), write_root, smart, watcher });
    let listener = TcpListener::bind(("127.0.0.1", port)).expect("bind bridge port");
    println!("dev bridge listening on http://127.0.0.1:{port}");
    for stream in listener.incoming().flatten() {
        let b = bridge.clone();
        std::thread::spawn(move || handle(&b, stream));
    }
}
