use crate::{
    config::{ConfigStore, Settings},
    drive::DriveClient,
    icon_cache, sync, tray,
};
use serde::Serialize;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_autostart::ManagerExt as AutostartExt;

#[tauri::command]
pub fn has_builtin_credentials() -> bool {
    crate::config::has_builtin_credentials()
}

/// True if launched with `--store-screenshots` (Windows debug builds only).
/// Lets the frontend skip onboarding/legal modals during the screenshot
/// automation, and drive its own scene navigation.
#[tauri::command]
pub fn is_store_screenshot_mode() -> bool {
    #[cfg(all(windows, debug_assertions))]
    {
        crate::store_screenshots::requested()
    }
    #[cfg(not(all(windows, debug_assertions)))]
    {
        false
    }
}

#[tauri::command]
pub fn window_ready(window: tauri::WebviewWindow) {
    let _ = window.show();
    let _ = window.set_always_on_top(true);
    let _ = window.set_always_on_top(false);
    let _ = window.set_focus();
}

#[tauri::command]
pub fn get_settings(config: State<'_, Arc<ConfigStore>>) -> Settings {
    config.get()
}

#[tauri::command]
pub async fn save_settings(
    app: AppHandle,
    config: State<'_, Arc<ConfigStore>>,
    settings: Settings,
) -> Result<(), String> {
    let old = config.get();
    config.save(settings.clone()).map_err(|e| e.to_string())?;
    tray::register_hotkeys(&app);
    tray::refresh_tray_menu(&app);
    let autostart_changed = settings.autostart != old.autostart;
    let admin_changed = settings.run_as_admin != old.run_as_admin;
    if admin_changed {
        crate::win_util::update_start_menu_shortcut(settings.run_as_admin);
    }
    if autostart_changed || admin_changed {
        if settings.run_as_admin {
            // Admin mode: use Task Scheduler when elevated; remove registry autostart.
            let _ = app.autolaunch().disable();
            if settings.autostart && crate::win_util::is_elevated() {
                if let Err(e) = crate::win_util::create_admin_autostart() {
                    log::warn!("failed to create admin autostart task: {e}");
                }
            } else if !settings.autostart {
                crate::win_util::remove_admin_autostart();
            }
        } else {
            // Normal mode: remove any scheduled task and use the registry
            crate::win_util::remove_admin_autostart();
            let result = if settings.autostart {
                app.autolaunch().enable()
            } else {
                app.autolaunch().disable()
            };
            if let Err(e) = result {
                log::warn!("failed to set autostart: {e}");
            }
        }
    }
    if settings.resolved_recordings_dir() != old.resolved_recordings_dir() {
        sync::restart_watcher(&app);
        sync::scan_and_enqueue(&app);
    }
    if settings.drive_folder_name != old.drive_folder_name {
        tray::on_library_folder_change(&app);
    }
    if settings.video.hud_corner != old.video.hud_corner
        || settings.video.hud_badges != old.video.hud_badges
        || settings.video.audio != old.video.audio
    {
        // Re-anchor/resize any HUD badges already on screen immediately,
        // instead of waiting for the next recording to pick up the change.
        crate::recording::hud::reanchor(&app);
    }
    if settings.hide_overlays_from_capture != old.hide_overlays_from_capture {
        let hidden = settings.hide_overlays_from_capture;
        crate::wheel::apply_capture_hidden(&app, hidden);
        crate::toast::apply_capture_hidden(&app, hidden);
        crate::recording::hud::apply_capture_hidden(&app, hidden);
        crate::recorder::apply_capture_hidden(&app, hidden);
    }
    let _ = app.emit("settings-changed", ());
    Ok(())
}

#[derive(Serialize)]
pub struct DriveStatus {
    connected: bool,
    email: Option<String>,
    name: Option<String>,
    photo: Option<String>,
    /// A separate YouTube-channel token is connected — uploads/streams go
    /// to that channel instead of the main account's.
    youtube_dedicated: bool,
    /// Google account that owns the uploaded videos' channel — for pinning
    /// YouTube Studio deep links with `authuser`.
    youtube_email: Option<String>,
}

#[derive(Serialize)]
pub struct DriveFolderInfo {
    id: String,
    name: String,
    empty: bool,
    is_capcove: bool,
}

fn is_capcove_filename(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or(name);
    if stem.len() < 19 { return false; }
    let b = stem.as_bytes();
    b[4] == b'-' && b[7] == b'-' && b[10] == b'_' && b[13] == b'-' && b[16] == b'-'
        && b[..19].iter().enumerate().all(|(i, &c)| matches!(i, 4|7|10|13|16) || c.is_ascii_digit())
}

#[tauri::command]
pub async fn list_drive_folders(
    config: State<'_, Arc<ConfigStore>>,
    drive: State<'_, Arc<DriveClient>>,
) -> Result<Vec<DriveFolderInfo>, String> {
    if !drive.is_connected() {
        return Err("Drive not connected".into());
    }
    let settings = config.get();
    let cid = settings.effective_google_client_id().to_string();
    let csec = settings.effective_google_client_secret().to_string();
    let folders = drive.list_root_folders(&cid, &csec).await.map_err(|e| e.to_string())?;
    let mut result = Vec::new();
    for (id, name) in folders {
        let files = drive.list_folder_file_names(&cid, &csec, &id, 20).await.unwrap_or_default();
        let empty = files.is_empty();
        let is_capcove = files.iter().any(|f| is_capcove_filename(f));
        result.push(DriveFolderInfo { id, name, empty, is_capcove });
    }
    Ok(result)
}

#[tauri::command]
pub fn get_drive_status(drive: State<'_, Arc<DriveClient>>) -> DriveStatus {
    DriveStatus {
        connected: drive.is_connected(),
        email: drive.account_email(),
        name: drive.account_name(),
        photo: drive.account_photo(),
        youtube_dedicated: drive.youtube_dedicated(),
        youtube_email: drive.youtube_account_email(),
    }
}

#[tauri::command]
pub async fn connect_drive(
    app: AppHandle,
    config: State<'_, Arc<ConfigStore>>,
    drive: State<'_, Arc<DriveClient>>,
    login_hint: Option<String>,
) -> Result<String, String> {
    let settings = config.get();
    let opener_app = app.clone();
    let email = drive
        .authorize(
            settings.effective_google_client_id(),
            settings.effective_google_client_secret(),
            login_hint.as_deref(),
            move |url| {
                use tauri_plugin_opener::OpenerExt;
                let _ = opener_app.opener().open_url(url, None::<&str>);
            },
        )
        .await
        .map_err(|e| e.to_string())?;
    sync::scan_and_enqueue(&app);
    let drain_app = app.clone();
    tauri::async_runtime::spawn(async move {
        crate::library::drain_offline_ops(&drain_app).await;
    });
    let sync_app = app.clone();
    tauri::async_runtime::spawn(async move {
        let _ = sync::sync_metadata_and_icons(&sync_app).await;
    });
    Ok(email)
}

#[tauri::command]
pub fn disconnect_drive(drive: State<'_, Arc<DriveClient>>) {
    drive.disconnect();
}

/// Connects a dedicated YouTube channel (second OAuth, YouTube scopes only)
/// — Google's chooser lets the user pick a brand channel without touching
/// the main Google/Drive connection or its identity.
#[tauri::command]
pub async fn connect_youtube(
    app: AppHandle,
    config: State<'_, Arc<ConfigStore>>,
    drive: State<'_, Arc<DriveClient>>,
) -> Result<(), String> {
    let settings = config.get();
    let opener_app = app.clone();
    // Pin the flow to the already-connected Google account — the chooser
    // then only offers THAT account's channels (main + brand), instead of
    // a from-scratch Google sign-in.
    let hint = drive.account_email();
    drive
        .authorize_youtube(
            settings.effective_google_client_id(),
            settings.effective_google_client_secret(),
            hint.as_deref(),
            move |url| {
                use tauri_plugin_opener::OpenerExt;
                let _ = opener_app.opener().open_url(url, None::<&str>);
            },
        )
        .await
        .map_err(|e| e.to_string())
}

/// Drops the dedicated channel token — YouTube falls back to the main
/// account's channel.
#[tauri::command]
pub fn disconnect_youtube(drive: State<'_, Arc<DriveClient>>) {
    drive.disconnect_youtube();
}

#[tauri::command]
pub fn cancel_drive_connect(drive: State<'_, Arc<DriveClient>>) {
    drive.cancel_authorize();
}

/// Live/status info of a streamed session's broadcast, for the gallery card.
#[tauri::command]
pub async fn get_youtube_live_info(
    config: State<'_, Arc<ConfigStore>>,
    drive: State<'_, Arc<DriveClient>>,
    video_id: String,
) -> Result<crate::drive::youtube::LiveVideoInfo, String> {
    if !drive.is_connected() {
        return Err("not_connected".into());
    }
    let settings = config.get();
    let cid = settings.effective_google_client_id().to_string();
    let csec = settings.effective_google_client_secret().to_string();
    drive.live_video_info(&cid, &csec, &video_id).await.map_err(|e| e.to_string())
}

/// The YouTube channel uploads go to (bound to the OAuth token). `None`
/// when not connected or when the token predates the youtube.readonly
/// scope — the UI offers a reconnect in that case.
#[tauri::command]
pub async fn get_youtube_channel(
    config: State<'_, Arc<ConfigStore>>,
    drive: State<'_, Arc<DriveClient>>,
) -> Result<Option<crate::drive::youtube::YouTubeChannelInfo>, String> {
    if !drive.is_connected() {
        return Ok(None);
    }
    let settings = config.get();
    let cid = settings.effective_google_client_id().to_string();
    let csec = settings.effective_google_client_secret().to_string();
    match drive.youtube_channel(&cid, &csec).await {
        Ok(info) => Ok(Some(info)),
        Err(e) => {
            log::info!("youtube channel lookup unavailable: {e}");
            Ok(None)
        }
    }
}

#[tauri::command]
pub fn sync_now(app: AppHandle) {
    sync::scan_and_enqueue(&app);
    let sync_app = app.clone();
    tauri::async_runtime::spawn(async move {
        let _ = sync::sync_metadata_and_icons(&sync_app).await;
    });
}


#[tauri::command]
pub fn open_settings(app: AppHandle) {
    tray::show_settings(&app);
}

/// Opens the app log file — lets an installed (packaged) build's log be reached
/// from Settings > General without hunting for its redirected package path.
#[tauri::command]
pub fn open_logs(app: AppHandle) {
    let Some(path) = crate::logging::log_file_path() else { return };
    use tauri_plugin_opener::OpenerExt;
    let _ = app.opener().open_path(path.to_string_lossy().to_string(), None::<&str>);
}

#[tauri::command]
pub async fn pick_folder(app: AppHandle) -> Result<Option<String>, String> {
    let picked = tauri::async_runtime::spawn_blocking(move || {
        use tauri_plugin_dialog::DialogExt;
        app.dialog().file().blocking_pick_folder()
    })
    .await
    .map_err(|e| e.to_string())?;
    Ok(picked.map(|p| p.to_string()))
}

/// Used by the Games settings page's "add a custom game" form — lets the
/// user pick the actual `.exe` instead of typing its name, so
/// `commands::games::inspect_exe_file` can derive the name/icon from it.
#[tauri::command]
pub async fn pick_exe_file(app: AppHandle) -> Result<Option<String>, String> {
    let picked = tauri::async_runtime::spawn_blocking(move || {
        use tauri_plugin_dialog::DialogExt;
        app.dialog().file().add_filter("Executable", &["exe"]).blocking_pick_file()
    })
    .await
    .map_err(|e| e.to_string())?;
    Ok(picked.map(|p| p.to_string()))
}

/// App/game icon as a ready-to-render data URL. Resolution order: embedded
/// pack → synced disk cache → catalog disk cache (downloaded on demand).
/// Embedded is checked first so it can't be shadowed by a cruder cached icon.
#[tauri::command]
pub fn get_app_icon(app: AppHandle, app_name: String) -> Result<String, String> {
    let ic = app.state::<Arc<icon_cache::IconCache>>();
    // `<name>__cover` requests (gallery status block, games list) fall back
    // to the bundled cover pack; plain names to the embedded icon pack.
    let embedded = if let Some(base) = app_name.strip_suffix("__cover") {
        crate::games_db::packed_cover_data_url(&app, base)
    } else {
        crate::games_db::embedded_icon_data_url(&app, &app_name)
    };
    if let Some(data_url) = embedded {
        return Ok(data_url);
    }
    if let Some(b64) = ic.get_base64(&app_name) {
        return Ok(format!("data:image/png;base64,{b64}"));
    }
    ic.get_catalog_base64(&app_name)
        .map(|b64| format!("data:image/png;base64,{b64}"))
        .ok_or_else(|| "not cached".to_string())
}

// ---------------------------------------------------------------------------
// Admin / elevation commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn get_is_elevated() -> bool {
    crate::win_util::is_elevated()
}

/// True when running from an installed MSIX/AppX package (e.g. Microsoft Store).
/// Such installs are updated by the Store, not by the in-app updater.
#[tauri::command]
pub fn is_packaged_install() -> bool {
    crate::win_util::is_packaged()
}

/// Re-launches the app with UAC elevation and exits the current process.
#[tauri::command]
pub fn request_admin() {
    crate::win_util::restart_as_admin();
}

#[derive(Serialize)]
pub struct PlatformCapabilities {
    pub os: &'static str,
    /// Whether the window picker (select-a-window capture) is expected to work.
    /// False on native Wayland sessions, where per-window introspection isn't available.
    pub window_capture: bool,
    /// True on Linux when running under native Wayland (not XWayland) — window
    /// capture is limited to X11/XWayland clients, so the UI should explain this.
    pub wayland_limited: bool,
}

#[tauri::command]
pub fn platform_capabilities() -> PlatformCapabilities {
    #[cfg(target_os = "windows")]
    {
        PlatformCapabilities { os: "windows", window_capture: true, wayland_limited: false }
    }
    #[cfg(target_os = "macos")]
    {
        PlatformCapabilities { os: "macos", window_capture: true, wayland_limited: false }
    }
    #[cfg(target_os = "linux")]
    {
        let wayland = std::env::var("XDG_SESSION_TYPE").map(|v| v == "wayland").unwrap_or(false)
            || std::env::var("WAYLAND_DISPLAY").is_ok();
        PlatformCapabilities { os: "linux", window_capture: !wayland, wayland_limited: wayland }
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        PlatformCapabilities { os: "unknown", window_capture: false, wayland_limited: false }
    }
}

