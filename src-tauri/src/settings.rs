//! App settings persisted as JSON. The frontend owns the schema.

use std::path::PathBuf;
use tauri::{AppHandle, Manager};

use crate::git::CmdResult;

fn settings_path(app: &AppHandle) -> CmdResult<PathBuf> {
    let dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join("settings.json"))
}

#[tauri::command]
pub fn load_settings(app: AppHandle) -> CmdResult<serde_json::Value> {
    let path = settings_path(&app)?;
    match std::fs::read_to_string(&path) {
        Ok(text) => Ok(serde_json::from_str(&text).unwrap_or(serde_json::Value::Null)),
        Err(_) => Ok(serde_json::Value::Null),
    }
}

#[tauri::command]
pub fn save_settings(app: AppHandle, settings: serde_json::Value) -> CmdResult<()> {
    let path = settings_path(&app)?;
    let tmp = path.with_extension("json.tmp");
    let text = serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())?;
    std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())
}
