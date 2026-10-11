mod git;
mod graph;
mod pty;
mod settings;

#[cfg(test)]
mod dev_bridge;

/// GUI apps launched from Finder/the dock get a minimal PATH. Borrow the login
/// shell's PATH so git hooks (node, husky…), credential helpers and `claude` resolve.
#[cfg(not(windows))]
fn inherit_login_path() {
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
    let Ok(mut child) = Command::new(&shell)
        .args(["-l", "-i", "-c", "printf '__GDT_PATH__%s__GDT_PATH__' \"$PATH\""])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    else {
        return;
    };
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() < Duration::from_secs(5) => {
                std::thread::sleep(Duration::from_millis(20))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return;
            }
        }
    }
    let mut out = String::new();
    if let Some(mut stdout) = child.stdout.take() {
        use std::io::Read;
        let _ = stdout.read_to_string(&mut out);
    }
    let mut parts = out.split("__GDT_PATH__");
    if let (Some(_), Some(path)) = (parts.next(), parts.next()) {
        if !path.is_empty() {
            std::env::set_var("PATH", path);
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[cfg(not(windows))]
    inherit_login_path();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(pty::PtyState::default())
        .manage(git::NetLocks::default())
        .invoke_handler(tauri::generate_handler![
            git::open_repo,
            git::init_repo,
            git::default_clone_dir,
            git::repo_summary,
            git::repo_fingerprint,
            git::load_repo,
            git::commit_detail,
            git::file_diff,
            git::wip_file_diff,
            git::file_content,
            git::wip_status,
            git::stage_files,
            git::unstage_files,
            git::head_message,
            git::commit,
            git::clone_repo,
            git::fetch_repo,
            git::pull_repo,
            git::push_repo,
            pty::pty_spawn,
            pty::pty_write,
            pty::pty_resize,
            pty::pty_kill,
            settings::load_settings,
            settings::save_settings,
        ])
        .run(tauri::generate_context!())
        .expect("error while running git-desktools");
}
