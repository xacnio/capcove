//! Game detection: when the foreground window is a game, react per
//! `replay_buffer.game_detect_mode` (`Clips`/`FullSession`/`Off`). "Is a
//! game" is decided by exe-name match, not fullscreen state; alt-tabbing
//! away is not a stop condition, only the game window closing is.

use std::sync::Arc;
use std::time::Duration;

use tauri::{AppHandle, Manager};

use crate::config::{ConfigStore, GameDetectMode, ReplayBufferTarget};
use crate::games_db::GamesDb;
use super::replay_buffer::{self, ReplayBufferManager};
use super::RecordingManager;

/// Processes that can legitimately run fullscreen but are never a game.
const EXCLUDED_EXES: &[&str] = &[
    "capcove", "explorer", "searchhost", "lockapp", "dwm",
    "chrome", "msedge", "firefox", "opera", "opera_gx", "brave", "vivaldi",
    "vlc", "mpc-hc64", "mpc-hc", "wmplayer",
    // Screenshot / region-selection tools run fullscreen overlays that would
    // otherwise look like a borderless game to the heuristic below.
    "snippingtool", "screenclippinghost", "screensketch",
    "sharex", "greenshot", "lightshot", "picpick", "flameshot", "magnify",
];

/// What the loop auto-started, so it knows what to stop when the game dies.
/// Carries the owning pid alongside the hwnd — see `is_window_alive`'s doc
/// comment for why the hwnd alone isn't a reliable "still the same window" check.
#[derive(Clone, Copy, PartialEq)]
enum Started {
    Buffer(u32, u32),
    Recording(u32, u32),
}

impl Started {
    fn hwnd(self) -> u32 {
        match self {
            Started::Buffer(h, _) | Started::Recording(h, _) => h,
        }
    }

    fn pid(self) -> u32 {
        match self {
            Started::Buffer(_, p) | Started::Recording(_, p) => p,
        }
    }
}

/// True only if `hwnd_u32` still exists *and* is still owned by `pid` —
/// checking `IsWindow` alone isn't enough: once a window is destroyed,
/// Windows can immediately hand its exact hwnd value to a brand-new,
/// unrelated window (some background process's hidden helper window, most
/// often), which would otherwise make a just-closed game look "still alive"
/// forever and leave its buffer/recording running indefinitely.
fn is_window_alive(hwnd_u32: u32, pid: u32) -> bool {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{GetWindowThreadProcessId, IsWindow};
    unsafe {
        let hwnd = HWND(hwnd_u32 as usize as *mut _);
        if !IsWindow(hwnd).as_bool() {
            return false;
        }
        let mut owner_pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut owner_pid));
        owner_pid == pid
    }
}

/// Plain `GetWindowRect` fallback — exclusive-fullscreen games can make the
/// DWM extended-frame query (`window_frame_rect`) fail, and for a fullscreen
/// window there's no shadow margin to strip anyway.
fn raw_window_rect(hwnd_u32: u32) -> Option<(i32, i32, u32, u32)> {
    use windows::Win32::Foundation::{HWND, RECT};
    use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;
    unsafe {
        let mut r = RECT::default();
        GetWindowRect(HWND(hwnd_u32 as usize as *mut _), &mut r).ok()?;
        Some((r.left, r.top, (r.right - r.left).max(0) as u32, (r.bottom - r.top).max(0) as u32))
    }
}

/// Launcher splashes and updater windows (GTA's is 146×28) match the game's
/// exe but are never the game itself — and NVENC outright rejects frames
/// that small, killing the whole capture. Require a plausible game size.
fn is_reasonable_game_window(hwnd_u32: u32) -> bool {
    match crate::capture::window_frame_rect(hwnd_u32).or_else(|| raw_window_rect(hwnd_u32)) {
        Some((_, _, w, h)) => w >= 320 && h >= 240,
        None => false,
    }
}

/// A detected game candidate in the foreground.
struct Detected {
    hwnd: u32,
    pid: u32,
    title: String,
    /// Display name (catalog title when known, exe stem otherwise).
    name: String,
    /// Exe stem — the persistent identity prefs are keyed by.
    exe: String,
    icon_url: Option<String>,
    cover_url: Option<String>,
}

/// One immediate detection pass that refreshes the "current game" state
/// without any auto-start side effects, so the wheel doesn't show stale
/// state while waiting on the polling interval.
pub fn refresh_current_now(app: &AppHandle) {
    if crate::capture::is_foreground_own_process() {
        return;
    }
    let games = app.state::<Arc<GamesDb>>();
    match detect_game(&games) {
        Some(d) => {
            games.set_current_game(Some(d.name.clone()));
            games.set_current_target(Some(crate::games_db::CurrentGameTarget {
                hwnd: d.hwnd,
                title: d.title,
                app: d.name,
            }));
        }
        None => {
            games.set_current_game(None);
            games.set_current_target(None);
        }
    }
}

/// Foreground window if it currently looks like a game — catalog/custom
/// exe-name match only (see `GamesDb::lookup`), no fullscreen fallback.
/// Games the user disabled on the settings Games page never match.
fn detect_game(games: &GamesDb) -> Option<Detected> {
    use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

    let hwnd = unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_invalid() {
            return None;
        }
        hwnd.0 as usize as u32
    };

    let (title, exe) = crate::capture::window_info(hwnd);
    let exe = exe?;
    if EXCLUDED_EXES.iter().any(|e| exe.eq_ignore_ascii_case(e)) {
        return None;
    }
    if !is_reasonable_game_window(hwnd) {
        return None;
    }

    // Known game by exe name — windowed or fullscreen, doesn't matter.
    // The exe path rides along for the catalog's path-qualified entries.
    let exe_path = crate::capture::window_exe_path(hwnd);
    let pid = crate::capture::pid_for_hwnd(hwnd)?;
    games.lookup(&exe, exe_path.as_deref()).map(|entry| Detected {
        hwnd,
        pid,
        title: title.unwrap_or_else(|| entry.name.clone()),
        name: entry.name,
        exe,
        icon_url: entry.icon_url,
        cover_url: entry.cover_url,
    })
}

/// Best-effort download of catalog game art not already in this build's
/// embedded packs. Custom games never hit this, since `lookup()` only
/// returns URLs for catalog entries.
fn spawn_icon_fetch(app: &tauri::AppHandle, game_name: String, icon_url: Option<String>, cover_url: Option<String>) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let jobs = [
            (game_name.clone(), icon_url, crate::games_db::embedded_icon_data_url(&app, &game_name).is_some()),
            (format!("{game_name}__cover"), cover_url, crate::games_db::packed_cover_data_url(&app, &game_name).is_some()),
        ];
        for (key, url, already_packed) in jobs {
            if already_packed {
                continue;
            }
            let Some(url) = url else { continue };
            let Ok(resp) = reqwest::get(&url).await else { continue };
            if !resp.status().is_success() {
                continue;
            }
            if let Ok(bytes) = resp.bytes().await {
                app.state::<Arc<crate::icon_cache::IconCache>>().store_catalog_png(&key, &bytes);
            }
        }
    });
}

/// Sets up a YouTube live broadcast for a full-session recording when the
/// feature is on for this game; shares the account check and broadcast
/// creation logic with `recording::try_start_live_broadcast`.
async fn maybe_create_live_broadcast(
    app: &AppHandle,
    games: &GamesDb,
    game_app: &str,
) -> Option<crate::drive::youtube::LiveBroadcast> {
    let settings = app.state::<Arc<ConfigStore>>().get();
    let overrides = games.overrides_for(game_app);
    // Per-game override beats the global toggle in both directions.
    let enabled = overrides
        .as_ref()
        .and_then(|o| o.youtube_live)
        .unwrap_or(settings.video.replay_buffer.full_session_youtube_live);
    if !enabled {
        return None;
    }
    crate::recording::try_start_live_broadcast(app, Some(game_app)).await
}

/// Spawned once at startup; runs for the app's lifetime.
pub fn spawn_detection_loop(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        // What this loop auto-started, if anything.
        let mut started: Option<Started> = None;
        // Window the user manually stopped clipping/recording during — don't
        // fight them by restarting for it; cleared once that window dies.
        let mut suppressed: Option<(u32, u32)> = None;
        // Last fullscreen candidate we logged, to log transitions once
        // instead of every 3s tick.
        let mut last_logged: Option<u32> = None;

        loop {
            tokio::time::sleep(Duration::from_secs(3)).await;

            let rb_mgr = app.state::<Arc<ReplayBufferManager>>();
            let rec_mgr = app.state::<Arc<RecordingManager>>();

            if let Some(what) = started {
                let (hwnd, pid) = (what.hwnd(), what.pid());
                if !is_window_alive(hwnd, pid) {
                    started = None;
                    last_logged = None;
                    app.state::<Arc<GamesDb>>().set_current_game(None);
                    app.state::<Arc<GamesDb>>().set_current_target(None);
                    match what {
                        Started::Buffer(..) => {
                            if rb_mgr.is_running() {
                                let confirm = app.state::<Arc<ConfigStore>>().get().video.replay_buffer.confirm_save_on_close;
                                if confirm {
                                    replay_buffer::stop_replay_buffer_for_pending_save(&app);
                                } else {
                                    let _ = replay_buffer::stop_replay_buffer(&app);
                                    crate::notify(&app, "Capcove", "Auto clipping stopped (game closed)");
                                }
                            }
                        }
                        Started::Recording(..) => {
                            if rec_mgr.is_recording() {
                                let _ = super::stop_recording(&app);
                            }
                        }
                    }
                    crate::tray::refresh_tray_menu(&app);
                } else {
                    // Still alive: detect a manual stop (nothing running
                    // explains the gap) and back off for this window.
                    let manually_stopped = match what {
                        Started::Buffer(..) => !rb_mgr.is_running(),
                        Started::Recording(..) => !rec_mgr.is_recording(),
                    };
                    if manually_stopped {
                        log::info!("game detect: auto capture was stopped manually, suppressing until the game closes");
                        suppressed = Some((hwnd, pid));
                        started = None;
                    }
                }
                continue;
            }

            if let Some((hwnd, pid)) = suppressed {
                if !is_window_alive(hwnd, pid) {
                    suppressed = None;
                } else {
                    continue;
                }
            }

            let rb = app.state::<Arc<ConfigStore>>().get().video.replay_buffer.clone();

            let games = app.state::<Arc<GamesDb>>();
            let Some(found) = detect_game(&games) else {
                // Our own overlay taking OS focus isn't the user leaving the
                // game (detect_game already excludes our exe) — only clear
                // state for an actual switch away.
                if !crate::capture::is_foreground_own_process() {
                    last_logged = None;
                    games.set_current_game(None);
                    games.set_current_target(None);
                }
                continue;
            };
            let Detected { hwnd, pid, title, name: game_app, exe, icon_url, cover_url } = found;
            // Keep the "Playing now" badge fresh regardless of what (if
            // anything) gets auto-started for this game.
            games.set_current_game(Some(game_app.clone()));
            games.set_current_target(Some(crate::games_db::CurrentGameTarget {
                hwnd,
                title: title.clone(),
                app: game_app.clone(),
            }));
            // A per-game override (settings -> Games) beats the global mode,
            // so a game set to Clips must still work under global Off.
            let mode = games
                .overrides_for(&game_app)
                .and_then(|o| o.game_detect_mode)
                .unwrap_or(rb.game_detect_mode);
            if mode == GameDetectMode::Off {
                last_logged = None;
                continue;
            }
            if last_logged != Some(hwnd) {
                log::info!("game detect: game candidate '{game_app}' (hwnd {hwnd}, mode {mode:?})");
                last_logged = Some(hwnd);
            }

            match mode {
                GameDetectMode::Clips => {
                    if rb_mgr.is_running() {
                        continue; // buffer already covering things (e.g. always-on)
                    }
                    let target = ReplayBufferTarget::SpecificWindow { hwnd, title, app: game_app.clone() };
                    match replay_buffer::start_replay_buffer_with_target(&app, Some(target)).await {
                        Ok(()) => {
                            started = Some(Started::Buffer(hwnd, pid));
                            games.touch_played(&exe);
                            spawn_icon_fetch(&app, game_app.clone(), icon_url.clone(), cover_url.clone());
                            crate::notify(&app, "Capcove", &format!("Auto clipping started: {game_app}"));
                            crate::tray::refresh_tray_menu(&app);
                        }
                        Err(e) => {
                            log::warn!("game detect: failed to start replay buffer for {game_app}: {e}");
                            // Without this, an uncapturable window would be
                            // retried every 3s for as long as it's foreground.
                            suppressed = Some((hwnd, pid));
                        }
                    }
                }
                GameDetectMode::FullSession => {
                    if rec_mgr.is_recording() {
                        continue; // one full recording at a time
                    }
                    let live = maybe_create_live_broadcast(&app, &games, &game_app).await;
                    match super::start_window_recording_live(&app, hwnd, title, game_app.clone(), live, None, true, false).await {
                        Ok(_) => {
                            started = Some(Started::Recording(hwnd, pid));
                            games.touch_played(&exe);
                            spawn_icon_fetch(&app, game_app.clone(), icon_url.clone(), cover_url.clone());
                            crate::tray::refresh_tray_menu(&app);
                        }
                        Err(e) => {
                            log::warn!("game detect: failed to start session recording for {game_app}: {e}");
                            crate::notify_error(&app, &format!("Couldn't start recording {game_app}: {e}"));
                            // Without this, a window that reliably fails to
                            // capture gets retried every 3s, each attempt
                            // leaking a YouTube broadcast and audio threads.
                            suppressed = Some((hwnd, pid));
                        }
                    }
                }
                GameDetectMode::Off => {}
            }
        }
    });
}
