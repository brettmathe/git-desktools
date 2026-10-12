//! Pseudo-terminal sessions for the docked terminal panel.

use base64::Engine;
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use serde::Serialize;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicU32, Ordering};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, State};

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

/// The Claude Code plugin bundled with the app (skills such as
/// smart-environment). Terminals opened in the app load it with `--plugin-dir`.
pub fn plugin_dir(app: &AppHandle) -> Option<PathBuf> {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("claude-plugin");
    if cfg!(debug_assertions) && source.exists() {
        return Some(source);
    }
    app.path().resource_dir().ok().map(|d| d.join("claude-plugin")).filter(|p| p.exists())
}

#[cfg(not(windows))]
fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// zsh reads its startup files from $ZDOTDIR. These forward to the user's own
/// files (so their setup is unchanged) and then define `claude` to load the
/// app's plugin, the way VS Code injects its shell integration. The last file
/// restores the user's ZDOTDIR, so shells started from this one are untouched.
#[cfg(not(windows))]
const ZSH_FILES: &[(&str, &str)] = &[
    (
        ".zshenv",
        "# git-desktools: forward to the user's zsh files, then add the Claude plugin.\n\
         GDT_ZDOTDIR=\"$ZDOTDIR\"\n\
         ZDOTDIR=\"${GDT_USER_ZDOTDIR:-$HOME}\"\n\
         [[ -f \"$ZDOTDIR/.zshenv\" ]] && source \"$ZDOTDIR/.zshenv\"\n\
         GDT_USER_ZDOTDIR=\"$ZDOTDIR\"\n\
         ZDOTDIR=\"$GDT_ZDOTDIR\"\n",
    ),
    (
        ".zprofile",
        "ZDOTDIR=\"$GDT_USER_ZDOTDIR\"\n\
         [[ -f \"$ZDOTDIR/.zprofile\" ]] && source \"$ZDOTDIR/.zprofile\"\n\
         ZDOTDIR=\"$GDT_ZDOTDIR\"\n",
    ),
    (
        ".zshrc",
        "ZDOTDIR=\"$GDT_USER_ZDOTDIR\"\n\
         [[ -f \"$ZDOTDIR/.zshrc\" ]] && source \"$ZDOTDIR/.zshrc\"\n\
         ZDOTDIR=\"$GDT_ZDOTDIR\"\n\
         if [[ -n \"$GDT_CLAUDE_PLUGIN_DIR\" ]]; then\n\
         \x20 claude() { command claude --plugin-dir \"$GDT_CLAUDE_PLUGIN_DIR\" \"$@\"; }\n\
         fi\n",
    ),
    (
        ".zlogin",
        "ZDOTDIR=\"$GDT_USER_ZDOTDIR\"\n\
         [[ -f \"$ZDOTDIR/.zlogin\" ]] && source \"$ZDOTDIR/.zlogin\"\n\
         unset GDT_ZDOTDIR\n",
    ),
];

#[cfg(not(windows))]
fn zsh_wrapper_dir() -> std::io::Result<PathBuf> {
    let dir = std::env::temp_dir().join(format!("git-desktools-zsh-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    for (name, text) in ZSH_FILES {
        let path = dir.join(name);
        if std::fs::read_to_string(&path).ok().as_deref() != Some(*text) {
            std::fs::write(path, text)?;
        }
    }
    Ok(dir)
}

fn build_command(kind: &str, cwd: &str, plugin: Option<&Path>) -> CommandBuilder {
    #[cfg(windows)]
    let mut cmd = {
        let mut c = CommandBuilder::new("powershell.exe");
        c.arg("-NoLogo");
        if kind == "claude" {
            let launch = match plugin {
                Some(p) => format!("claude --plugin-dir '{}'", p.display().to_string().replace('\'', "''")),
                None => "claude".into(),
            };
            c.args(["-NoExit", "-Command", &launch]);
        }
        c
    };
    #[cfg(not(windows))]
    let mut cmd = {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
        let mut c = CommandBuilder::new(&shell);
        c.args(["-l", "-i"]);
        // zsh: `claude` typed in any terminal loads the plugin (see ZSH_FILES).
        let wrapper = plugin
            .filter(|_| shell.rsplit('/').next() == Some("zsh"))
            .and_then(|_| zsh_wrapper_dir().ok());
        if let (Some(dir), Some(p)) = (&wrapper, plugin) {
            let user = std::env::var("ZDOTDIR").unwrap_or_else(|_| std::env::var("HOME").unwrap_or_default());
            c.env("GDT_USER_ZDOTDIR", user);
            c.env("ZDOTDIR", dir);
            c.env("GDT_WRAPPER_ZDOTDIR", dir);
            c.env("GDT_CLAUDE_PLUGIN_DIR", p);
        }
        if kind == "claude" {
            // Keep a shell open after Claude exits (with the same setup).
            let q = sh_quote(&shell);
            let launch = match (wrapper.is_some(), plugin) {
                (true, _) => format!("claude; ZDOTDIR=\"$GDT_WRAPPER_ZDOTDIR\" exec {q} -l -i"),
                (false, Some(p)) => {
                    format!("claude --plugin-dir {}; exec {q} -l -i", sh_quote(&p.to_string_lossy()))
                }
                (false, None) => format!("claude; exec {q} -l -i"),
            };
            c.args(["-c", &launch]);
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
    let plugin = plugin_dir(&app);
    let child = pair.slave.spawn_command(build_command(&kind, &cwd, plugin.as_deref())).map_err(|e| e.to_string())?;
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
        run_session_with(kind, input, until, None)
    }

    fn run_session_with(kind: &str, input: &str, until: &str, plugin: Option<&Path>) -> String {
        let dir = std::env::temp_dir();
        let pair = native_pty_system()
            .openpty(PtySize { rows: 24, cols: 120, pixel_width: 0, pixel_height: 0 })
            .unwrap();
        let mut child = pair.slave.spawn_command(build_command(kind, dir.to_str().unwrap(), plugin)).unwrap();
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

    /// In zsh, `claude` loads the bundled plugin and the user's own setup still runs.
    #[test]
    fn zsh_terminal_loads_the_plugin() {
        if !std::env::var("SHELL").unwrap_or_default().ends_with("/zsh") {
            return;
        }
        let plugin = Path::new(env!("CARGO_MANIFEST_DIR")).join("claude-plugin");
        let out = run_session_with(
            "shell",
            "printf '__W__%s__P__%s__Z__%s__END__\\n' \"$(whence -w claude)\" \"$GDT_CLAUDE_PLUGIN_DIR\" \"$ZDOTDIR\"; exit\n",
            "__END__\r\n",
            Some(&plugin),
        );
        assert!(out.contains("__W__claude: function__P__"), "output: {out}");
        assert!(out.contains(&format!("__P__{}__Z__", plugin.display())), "output: {out}");
        // The user's ZDOTDIR is back for anything started from this shell.
        let home = std::env::var("ZDOTDIR").unwrap_or_else(|_| std::env::var("HOME").unwrap());
        assert!(out.contains(&format!("__Z__{home}__END__")), "output: {out}");
    }
}
