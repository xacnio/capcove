use std::path::PathBuf;
use std::sync::Arc;

use tauri::{AppHandle, Manager};

use crate::recording::audio_capture::{list_devices, AudioDeviceInfo, AudioFlow};
use crate::recording::encoder::{cached_available_encoders as list_encoders, EncoderInfo};
use crate::recording::replay_buffer::{self, PendingClipState, ReplayBufferManager, ReplayBufferStatus, ReplayCrashRecoveryState};
use crate::recording::{self, RecordingManager, RecordingSession};

#[tauri::command]
pub async fn start_window_recording(
    app: AppHandle,
    hwnd: u32,
    title: String,
    app_name: String,
) -> Result<RecordingSession, String> {
    recording::start_window_recording(&app, hwnd, title, app_name).await
}

// `recording::stop_recording` blocks on native-thread joins (capture thread,
// ffmpeg writer flush), which would freeze the IPC dispatch thread if run
// inline; `spawn_blocking` moves it onto a dedicated thread instead.
#[tauri::command]
pub async fn stop_recording(app: AppHandle) -> Result<PathBuf, String> {
    tauri::async_runtime::spawn_blocking(move || recording::stop_recording(&app))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn cancel_recording(app: AppHandle) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || recording::cancel_recording(&app))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn get_recording_status(app: AppHandle) -> Option<RecordingSession> {
    app.state::<Arc<RecordingManager>>().current_session()
}

/// Pull side of `recording::check_crash_recovery`'s result, in case the
/// startup check already fired before the window was done mounting and
/// listening for it. `None` once fetched or if nothing needed recovering.
#[tauri::command]
pub fn get_crash_recovery_result(app: AppHandle) -> Option<serde_json::Value> {
    app.state::<Arc<recording::CrashRecoveryState>>().take()
}

/// Pull side of `replay_buffer::stage_replay_buffer_crash_recovery` — same
/// startup-mount race as `get_crash_recovery_result`.
#[tauri::command]
pub fn get_replay_crash_recovery_result(app: AppHandle) -> Option<serde_json::Value> {
    app.state::<Arc<ReplayCrashRecoveryState>>().take()
}

// `replay_buffer::recover_replay_buffer_crash` blocks on an ffmpeg
// subprocess — same inline-on-IPC-thread freeze risk as `save_replay` above.
#[tauri::command]
pub async fn recover_replay_buffer_crash(app: AppHandle) -> Result<PathBuf, String> {
    replay_buffer::recover_replay_buffer_crash(&app).await
}

#[tauri::command]
pub fn discard_replay_buffer_crash(app: AppHandle) -> Result<(), String> {
    replay_buffer::discard_replay_buffer_crash(&app)
}

/// Pull side of `replay_buffer::stop_replay_buffer_for_pending_save` — same
/// startup-mount race as `get_replay_crash_recovery_result`.
#[tauri::command]
pub fn get_pending_clip_result(app: AppHandle) -> Option<serde_json::Value> {
    app.state::<Arc<PendingClipState>>().take()
}

// `replay_buffer::confirm_pending_clip` blocks on an ffmpeg subprocess —
// same inline-on-IPC-thread freeze risk as `save_replay`/crash recovery.
#[tauri::command]
pub async fn confirm_pending_clip(app: AppHandle) -> Result<PathBuf, String> {
    replay_buffer::confirm_pending_clip(&app).await
}

#[tauri::command]
pub fn discard_pending_clip(app: AppHandle) -> Result<(), String> {
    replay_buffer::discard_pending_clip(&app)
}

/// Pauses/resumes the active recording: video frames are skipped and audio
/// bytes dropped in lockstep, so the output simply has no gap.
#[tauri::command]
pub fn pause_recording(app: AppHandle, paused: bool) -> Result<(), String> {
    if app.state::<Arc<RecordingManager>>().set_paused(paused) {
        Ok(())
    } else {
        Err("No recording in progress".into())
    }
}

#[tauri::command]
pub fn get_recording_paused(app: AppHandle) -> bool {
    app.state::<Arc<RecordingManager>>().is_paused()
}

/// Turns the local file on/off independently of any live stream (see the
/// wheel's matching wedge in `wheel::wheel_action`). Returns the new state.
#[tauri::command]
pub async fn toggle_local_recording(app: AppHandle) -> Result<bool, String> {
    recording::toggle_local_recording(&app).await
}

/// Turns the YouTube live feed on/off independently of the local file.
#[tauri::command]
pub async fn toggle_live_streaming(app: AppHandle) -> Result<bool, String> {
    recording::toggle_live_streaming(&app).await
}

#[tauri::command]
pub async fn list_available_encoders(app: AppHandle) -> Vec<EncoderInfo> {
    list_encoders(&app).await
}

/// What "Auto" concretely resolves to on this machine right now (cached per
/// app run) — the settings UI shows it next to the Auto option.
#[tauri::command]
pub async fn resolve_auto_encoder(app: AppHandle) -> crate::config::EncoderChoice {
    recording::resolve_encoder(&app, &crate::config::EncoderChoice::Auto).await
}

#[derive(serde::Serialize)]
pub struct AudioDeviceLists {
    pub outputs: Vec<AudioDeviceInfo>,
    pub inputs: Vec<AudioDeviceInfo>,
}

#[tauri::command]
pub async fn list_audio_devices() -> Result<AudioDeviceLists, String> {
    tauri::async_runtime::spawn_blocking(|| {
        Ok(AudioDeviceLists {
            outputs: list_devices(AudioFlow::Render)?,
            inputs: list_devices(AudioFlow::Capture)?,
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[derive(serde::Serialize)]
pub struct AudioAppEntry {
    pub exe: String,
    /// Display name — the games catalog's title when the exe is a known
    /// game, the exe stem otherwise.
    pub name: String,
    /// `data:image/png;base64,...` icon extracted from the process's own
    /// executable, when available, shown next to the row instead of bare text.
    pub icon: Option<String>,
}

/// Per-exe icon memo for the audio-apps list — extraction reads the PE's
/// icon resource off disk, no need to redo it on every settings refresh.
fn cached_exe_icon(exe_lower: &str, exe_path: Option<&str>) -> Option<String> {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    // Capped so a long session that sees many distinct executables producing
    // audio (each icon can be tens of KB as a base64 PNG) doesn't grow this
    // forever — clearing occasionally just costs one extra icon extraction
    // next time that exe reappears, which is cheap and rare compared to the
    // common case this memo exists for (the same handful of apps refreshed
    // repeatedly as the settings page polls).
    const MAX_ENTRIES: usize = 500;
    static CACHE: OnceLock<Mutex<HashMap<String, Option<String>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(hit) = cache.lock().unwrap().get(exe_lower) {
        return hit.clone();
    }
    use base64::Engine;
    let icon = exe_path
        .and_then(crate::icon_cache::extract_png_from_exe_path)
        .map(|png| format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(png)));
    let mut guard = cache.lock().unwrap();
    if guard.len() >= MAX_ENTRIES {
        guard.clear();
    }
    guard.insert(exe_lower.to_string(), icon.clone());
    icon
}

/// Best-effort Discord install lookup (stable, then PTB/Canary):
/// `%LOCALAPPDATA%\Discord<flavor>\app-*\Discord<flavor>.exe`. Newest
/// `app-*` folder wins — that's the currently-installed version.
fn installed_discord() -> Option<(String, String)> {
    let local = std::env::var_os("LOCALAPPDATA")?;
    for flavor in ["Discord", "DiscordPTB", "DiscordCanary"] {
        let root = std::path::Path::new(&local).join(flavor);
        let Ok(entries) = std::fs::read_dir(&root) else { continue };
        let mut apps: Vec<_> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("app-")))
            .collect();
        apps.sort();
        for app_dir in apps.iter().rev() {
            let exe = app_dir.join(format!("{flavor}.exe"));
            if exe.exists() {
                return Some((flavor.to_string(), exe.to_string_lossy().into_owned()));
            }
        }
    }
    None
}

/// Apps that currently have an audio session (candidates for per-app
/// recording tracks), plus Discord pinned whenever it's installed even if
/// idle-silent, since capturing voice chat is the most common per-app track.
#[tauri::command]
pub async fn list_audio_apps(app: AppHandle) -> Result<Vec<AudioAppEntry>, String> {
    let games = app.state::<Arc<crate::games_db::GamesDb>>().inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let apps = crate::recording::audio_capture::list_audio_apps()?;
        // The currently-detected game is hidden: its audio is what the main
        // system/game capture already records, so listing it invites
        // accidentally double-tracking it.
        let current_game = games.current_game().map(|g| g.to_lowercase());
        let mut out: Vec<AudioAppEntry> = apps
            .into_iter()
            .filter_map(|a| {
                // Display-name lookup only (no full path at hand) — a
                // generic stem staying unresolved here just shows the exe
                // name, which is fine for an audio-track label.
                let name = games.lookup(&a.exe, None).map(|e| e.name).unwrap_or_else(|| a.exe.clone());
                if current_game.as_deref() == Some(name.to_lowercase().as_str()) {
                    return None;
                }
                let path = crate::capture::exe_path_for_pid(a.pid);
                let icon = cached_exe_icon(&a.exe.to_ascii_lowercase(), path.as_deref());
                Some(AudioAppEntry { exe: a.exe, name, icon })
            })
            .collect();
        if !out.iter().any(|e| e.exe.to_ascii_lowercase().starts_with("discord")) {
            if let Some((flavor, exe_path)) = installed_discord() {
                let icon = cached_exe_icon(&flavor.to_ascii_lowercase(), Some(&exe_path));
                out.push(AudioAppEntry { exe: flavor, name: "Discord".into(), icon });
            }
        }
        Ok(out)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn start_replay_buffer(app: AppHandle) -> Result<(), String> {
    replay_buffer::start_replay_buffer(&app).await
}

#[tauri::command]
pub async fn stop_replay_buffer(app: AppHandle) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || replay_buffer::stop_replay_buffer(&app))
        .await
        .map_err(|e| e.to_string())?
}

// `replay_buffer::save_replay` blocks on a native thread join plus a
// `block_on` of the ffmpeg concat subprocess — same inline-on-IPC-thread
// freeze risk as `stop_recording` above.
#[tauri::command]
pub async fn save_replay(app: AppHandle) -> Result<PathBuf, String> {
    tauri::async_runtime::spawn_blocking(move || replay_buffer::save_replay(&app))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn get_replay_buffer_status(app: AppHandle) -> ReplayBufferStatus {
    app.state::<Arc<ReplayBufferManager>>().status()
}
