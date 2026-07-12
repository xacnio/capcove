//! Settings → Games page commands: catalog listing, per-game enable/disable,
//! manual (custom) games, catalog+icon sync, and lazy per-row icon fetch.

use std::sync::Arc;

use serde::Serialize;
use tauri::{AppHandle, State};

use crate::games_db::{self, GameRow, GamesDb};

#[tauri::command]
pub fn list_games(db: State<'_, Arc<GamesDb>>) -> Vec<GameRow> {
    db.list_games()
}

/// Display name of the game currently detected in the foreground, if any.
#[tauri::command]
pub fn get_current_game(db: State<'_, Arc<GamesDb>>) -> Option<String> {
    db.current_game()
}

/// Rows group every exe of a game, so the toggle flips them all at once.
#[tauri::command]
pub fn set_game_enabled(db: State<'_, Arc<GamesDb>>, exes: Vec<String>, enabled: bool) {
    db.set_enabled(&exes, enabled);
}

/// Per-game capture overrides (all-default clears the entry).
#[tauri::command]
pub fn set_game_overrides(db: State<'_, Arc<GamesDb>>, name: String, overrides: crate::config::GameOverrides) {
    db.set_overrides(&name, overrides);
}

/// A single game's overrides without pulling the whole catalog through
/// `list_games` — used by the wheel to read/patch just the currently
/// detected game's settings.
#[tauri::command]
pub fn get_game_overrides(db: State<'_, Arc<GamesDb>>, name: String) -> crate::config::GameOverrides {
    db.overrides_for(&name).unwrap_or_default()
}

/// `icon`, if given, is a `data:image/png;base64,...` URL from
/// `inspect_exe_file`, cached under the resolved display name so
/// `get_app_icon` finds it without waiting for the game to be detected running.
#[tauri::command]
pub fn add_custom_game(
    db: State<'_, Arc<GamesDb>>,
    icons: State<'_, Arc<crate::icon_cache::IconCache>>,
    exe: String,
    name: String,
    icon: Option<String>,
) {
    let effective_name = db.add_custom(&exe, &name);
    let Some(data_url) = icon else { return };
    let Some(b64) = data_url.strip_prefix("data:image/png;base64,") else { return };
    use base64::Engine;
    if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(b64) {
        icons.store_png(&effective_name, &bytes);
    }
}

/// Full picked `.exe` path → its normalized stem (as `add_custom_game`
/// would store it), a suggested display name (the file's own name), and
/// its extracted icon as a data URL.
#[derive(Serialize)]
pub struct InspectedExe {
    pub exe_stem: String,
    pub suggested_name: String,
    pub icon: Option<String>,
}

#[tauri::command]
pub fn inspect_exe_file(path: String) -> InspectedExe {
    let exe_stem = games_db::exe_stem(&path);
    let suggested_name = std::path::Path::new(&path)
        .file_stem()
        .and_then(|s| s.to_str())
        .map(str::to_string)
        .unwrap_or_else(|| exe_stem.clone());
    let icon = crate::icon_cache::extract_png_from_exe_path(&path).map(|bytes| {
        use base64::Engine;
        format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes))
    });
    InspectedExe { exe_stem, suggested_name, icon }
}

#[tauri::command]
pub fn remove_custom_game(db: State<'_, Arc<GamesDb>>, exe: String) {
    db.remove_custom(&exe);
}

/// Deletes a whole custom game (every exe registered under its name) — the
/// settings row's own remove button, as opposed to `remove_custom_game`
/// which drops a single exe from a multi-exe entry.
#[tauri::command]
pub fn remove_custom_game_group(db: State<'_, Arc<GamesDb>>, name: String) {
    db.remove_custom_group(&name);
}

#[derive(Serialize)]
pub struct SyncResult {
    pub games: usize,
    pub icons_downloaded: usize,
}

/// Force-refreshes the catalog and mirrors all missing game icons locally.
/// Progress streams via the `games-sync-progress` event.
#[tauri::command]
pub async fn sync_games(app: AppHandle) -> Result<SyncResult, String> {
    let (games, icons_downloaded) = games_db::sync(&app).await?;
    Ok(SyncResult { games, icons_downloaded })
}

/// Lazy per-row icon: downloads (and caches) one game's icon on demand,
/// returning base64 PNG bytes.
#[tauri::command]
pub async fn fetch_game_icon(app: AppHandle, exe: String) -> Result<String, String> {
    games_db::fetch_icon(&app, exe).await
}

/// Lazy per-row wide cover art (the settings list's row backdrop) — same
/// shape as `fetch_game_icon`, cached under `<name>__cover`.
#[tauri::command]
pub async fn fetch_game_cover(app: AppHandle, exe: String) -> Result<String, String> {
    games_db::fetch_cover(&app, exe).await
}
