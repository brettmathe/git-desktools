//! Pseudo-terminal sessions for the docked terminal panel.

use base64::Engine;
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use serde::Serialize;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, State};

use crate::git::CmdResult;

struct Session {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
}

#[derive(Default)]
pub struct PtyState {
    sessions: Mutex<HashMap<u32, Session>>,
    next: AtomicU32,
}

#[derive(Serialize, Clone)]
struct DataEvent {
    id: u32,
    data: String,
}

#[derive(Serialize, Clone)]
struct ExitEvent {
    id: u32,
}

fn build_command(kind: &str, cwd: &str) -> CommandBuilder {
    #[cfg(windows)]
    let mut cmd = {
        let mut c = CommandBuilder::new("powershell.exe");
        c.arg("-NoLogo");
        if kind == "claude" {
            c.args(["-NoExit", "-Command", "claude"]);
        }
        c
    };
    #[cfg(not(windows))]
    let mut cmd = {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
        let mut c = CommandBuilder::new(&shell);
        c.args(["-l", "-i"]);
        if kind == "claude" {
            // Keep a shell open after Claude exits.
            c.args(["-c", &format!("claude; exec '{shell}' -l -i")]);
        }
        c
    };
    cmd.cwd(cwd);
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLORTERM", "truecolor");
    // A Claude session that launched this app must not leak into the nested one.
    for (key, _) in std::env::vars() {
        if key == "CLAUDECODE" || key.starts_with("CLAUDE_CODE_") {
            cmd.env_remove(key);
        }
    }
    cmd
}

#[tauri::command]
pub fn pty_spawn(
    app: AppHandle,
    state: State<'_, PtyState>,
    cwd: String,
    kind: String,
    cols: u16,
    rows: u16,
) -> CmdResult<u32> {
    let pair = native_pty_system()
        .openpty(PtySize { rows: rows.max(2), cols: cols.max(2), pixel_width: 0, pixel_height: 0 })
        .map_err(|e| e.to_string())?;
    let child = pair.slave.spawn_command(build_command(&kind, &cwd)).map_err(|e| e.to_string())?;
    drop(pair.slave);
    let mut reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
    let writer = pair.master.take_writer().map_err(|e| e.to_string())?;
    let id = state.next.fetch_add(1, Ordering::SeqCst) + 1;

    std::thread::spawn(move || {
        let mut buf = [0u8; 16 * 1024];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let data = base64::engine::general_purpose::STANDARD.encode(&buf[..n]);
                    let _ = app.emit("pty-data", DataEvent { id, data });
                }
            }
        }
        let _ = app.emit("pty-exit", ExitEvent { id });
    });

    state.sessions.lock().unwrap().insert(id, Session { master: pair.master, writer, child });
    Ok(id)
}

#[tauri::command]
pub fn pty_write(state: State<'_, PtyState>, id: u32, data: String) -> CmdResult<()> {
    let mut sessions = state.sessions.lock().unwrap();
    let s = sessions.get_mut(&id).ok_or("terminal session closed")?;
    s.writer.write_all(data.as_bytes()).map_err(|e| e.to_string())?;
    s.writer.flush().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn pty_resize(state: State<'_, PtyState>, id: u32, cols: u16, rows: u16) -> CmdResult<()> {
    let sessions = state.sessions.lock().unwrap();
    let s = sessions.get(&id).ok_or("terminal session closed")?;
    s.master
        .resize(PtySize { rows: rows.max(2), cols: cols.max(2), pixel_width: 0, pixel_height: 0 })
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn pty_kill(state: State<'_, PtyState>, id: u32) {
    if let Some(mut s) = state.sessions.lock().unwrap().remove(&id) {
        let _ = s.child.kill();
    }
}

#[cfg(all(test, not(windows)))]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn run_session(kind: &str, input: &str, until: &str) -> String {
        let dir = std::env::temp_dir();
        let pair = native_pty_system()
            .openpty(PtySize { rows: 24, cols: 120, pixel_width: 0, pixel_height: 0 })
            .unwrap();
        let mut child = pair.slave.spawn_command(build_command(kind, dir.to_str().unwrap())).unwrap();
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().unwrap();
        let mut writer = pair.master.take_writer().unwrap();
        writer.write_all(input.as_bytes()).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            while let Ok(n) = reader.read(&mut buf) {
                if n == 0 || tx.send(buf[..n].to_vec()).is_err() {
                    break;
                }
            }
        });
        let mut out = String::new();
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(20) && !out.contains(until) {
            if let Ok(chunk) = rx.recv_timeout(Duration::from_millis(200)) {
                out.push_str(&String::from_utf8_lossy(&chunk));
            }
        }
        let _ = child.kill();
        out
    }

    #[test]
    fn shell_session_starts_in_cwd_without_claude_session_env() {
        let out = run_session(
            "shell",
            "printf '__CWD__%s__ENV__%s__END__\\n' \"$(pwd -P)\" \"${CLAUDECODE:-unset}\"; exit\n",
            // The pty echoes input, so wait for text only the command's output contains.
            "__ENV__unset__END__",
        );
        let expected = std::env::temp_dir().canonicalize().unwrap();
        assert!(out.contains(&format!("__CWD__{}", expected.display())), "output: {out}");
        assert!(out.contains("__ENV__unset__END__"), "output: {out}");
    }
}
