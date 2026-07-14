//! The radial shortcut wheel: a centered, always-on-top transparent window
//! listing capture actions as clickable wedges. A wedge click comes back
//! through `wheel_action`, which dispatches the same paths the hotkeys use.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tauri::{window::Color, AppHandle, Emitter, Manager, PhysicalPosition, WebviewUrl, WebviewWindowBuilder};

const LABEL: &str = "wheel";

/// How long the (hidden) wheel window is kept alive before its WebView2
/// instance is destroyed to free memory — long enough repeated opens/closes
/// within a session skip a full rebuild, short enough RAM isn't held forever.
const IDLE_DESTROY_SECS: u64 = 300;

/// Invalidates any pending idle-destroy scheduled by an earlier close —
/// bumped on every open *and* close, so only the destroy scheduled by the
/// most recent close ever actually fires.
static DESTROY_EPOCH: AtomicU64 = AtomicU64::new(0);

/// Destroys the wheel window once `IDLE_DESTROY_SECS` has passed with
/// nothing reopening or re-closing it. Trusts the epoch alone rather than
/// re-checking `win.is_visible()`, which reflects the frontend's own
/// animated `hide()` and can read stale.
fn schedule_idle_destroy(app: &AppHandle) {
    let epoch = DESTROY_EPOCH.fetch_add(1, Ordering::SeqCst) + 1;
    log::info!("wheel: idle-destroy scheduled (epoch {epoch}, in {IDLE_DESTROY_SECS}s)");
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(IDLE_DESTROY_SECS)).await;
        if DESTROY_EPOCH.load(Ordering::SeqCst) != epoch {
            log::info!("wheel: idle-destroy (epoch {epoch}) superseded, skipping");
            return; // superseded by a later open/close — leave it alone
        }
        let app2 = app.clone();
        let _ = app.run_on_main_thread(move || {
            if let Some(win) = app2.get_webview_window(LABEL) {
                log::info!("wheel: idle-destroy (epoch {epoch}) firing, closing window");
                let _ = win.close();
            } else {
                log::info!("wheel: idle-destroy (epoch {epoch}) fired but window was already gone");
            }
        });
    });
}

/// Opens (or toggles closed) the wheel over the monitor the cursor is on.
/// The webview is reused (just shown/hidden) across opens within
/// `IDLE_DESTROY_SECS` of each other, so those are effectively instant —
/// only a truly cold open (first ever, or after a long idle destroy) pays
/// the full WebView2/page-load cost.
pub fn open(app: &AppHandle) {
    // Fresh "current game" read right now, while the game still holds OS
    // focus, so the context card reflects the window the user just clicked.
    #[cfg(windows)]
    crate::recording::game_detect::refresh_current_now(app);
    let app2 = app.clone();
    let _ = app.run_on_main_thread(move || {
        let monitor = app2
            .cursor_position()
            .ok()
            .and_then(|p| app2.monitor_from_point(p.x, p.y).ok().flatten())
            .or_else(|| app2.primary_monitor().ok().flatten());

        if let Some(win) = app2.get_webview_window(LABEL) {
            if win.is_visible().unwrap_or(false) {
                // The frontend owns the actual `hide()` (resets its entrance
                // animation first) and confirms back via `wheel_closed`.
                let _ = app2.emit("wheel-close-requested", ());
            } else {
                // Reopening a hidden-but-not-yet-destroyed window — cancel
                // whatever idle-destroy the last close scheduled.
                let epoch = DESTROY_EPOCH.fetch_add(1, Ordering::SeqCst) + 1;
                log::info!("wheel: reopened, idle-destroy cancelled (epoch now {epoch})");
                if let Some(m) = &monitor {
                    let _ = win.set_position(PhysicalPosition::new(m.position().x, m.position().y));
                    let _ = win.set_size(*m.size());
                }
                let _ = win.show();
                let _ = win.set_focus();
                // Tells the frontend to drop back to the root ring and close
                // any leftover Gallery/Player windows, so every summon starts
                // clean instead of resuming wherever it was left.
                let _ = app2.emit("wheel-shown", ());
            }
            return;
        }

        let build = || -> tauri::Result<()> {
            let win = WebviewWindowBuilder::new(&app2, LABEL, WebviewUrl::App("pages/wheel.html".into()))
                .title("Capcove")
                .decorations(false)
                .shadow(false)
                .always_on_top(true)
                .skip_taskbar(true)
                .resizable(false)
                .transparent(true)
                .background_color(Color(0, 0, 0, 0))
                .visible(false)
                .build()?;
            // Ghost window: shown without the DWM open animation, and (unless
            // opted out) hidden from any capture/streaming software. Must
            // happen before show().
            if let Ok(raw) = win.hwnd() {
                let hwnd_u32 = raw.0 as usize as u32;
                crate::win_util::make_overlay_ghost(hwnd_u32);
                let hide = app2.state::<Arc<crate::config::ConfigStore>>().get().hide_overlays_from_capture;
                crate::win_util::set_capture_hidden(hwnd_u32, hide);
            }
            if let Some(m) = &monitor {
                let _ = win.set_position(PhysicalPosition::new(m.position().x, m.position().y));
                let _ = win.set_size(*m.size());
            } else {
                let _ = win.set_size(tauri::LogicalSize::new(1280.0, 800.0));
                let _ = win.center();
            }
            let _ = win.show();
            let _ = win.set_focus();
            Ok(())
        };
        if let Err(e) = build() {
            log::warn!("shortcut wheel could not be opened: {e}");
        }
    });
}

/// Re-applies the capture-hidden preference to the wheel window if it
/// already exists, so flipping `hide_overlays_from_capture` takes effect
/// immediately instead of on the next open.
pub fn apply_capture_hidden(app: &AppHandle, hidden: bool) {
    if let Some(win) = app.get_webview_window(LABEL) {
        if let Ok(raw) = win.hwnd() {
            crate::win_util::set_capture_hidden(raw.0 as usize as u32, hidden);
        }
    }
}

fn close(app: &AppHandle) {
    // Same reasoning as the toggle-close branch in `open()`: only ask, don't
    // hide directly — the frontend owns the `hide()` timing, and confirms it
    // back via `wheel_closed`.
    let app2 = app.clone();
    let _ = app.run_on_main_thread(move || {
        if app2.get_webview_window(LABEL).is_some() {
            let _ = app2.emit("wheel-close-requested", ());
        }
    });
}

/// Called by the frontend's `closeSelf` right after it hides — some close
/// paths (Esc, blur, backdrop click) never otherwise round-trip through Rust.
#[tauri::command]
pub fn wheel_closed(app: AppHandle) {
    schedule_idle_destroy(&app);
}

/// Opens (or toggles) the radial wheel — same entry point as its hotkey and
/// tray item, exposed for the gallery title-bar button. `async` for the same
/// reason as `recorder::open_recorder`: a sync command building/showing a
/// window from inside its own IPC callback freezes the app.
#[tauri::command]
pub async fn open_wheel(app: AppHandle) {
    open(&app);
}

/// Wedge click → close the wheel and run the picked action through the same
/// paths the dedicated hotkeys use. `folder` overrides the recording folder
/// for actions that start a recording directly.
#[tauri::command]
pub fn wheel_action(app: AppHandle, action: String, folder: Option<String>) {
    use crate::config::ShortcutCapture;
    close(&app);
    match action.as_str() {
        "save_replay" => {
            #[cfg(windows)]
            {
                let app = app.clone();
                tauri::async_runtime::spawn_blocking(move || {
                    if let Err(e) = crate::recording::replay_buffer::save_replay(&app) {
                        crate::notify_error(&app, &e);
                    }
                });
            }
        }
        "toggle_recording" => {
            #[cfg(windows)]
            {
                if app.state::<Arc<crate::recording::RecordingManager>>().is_recording() {
                    let app = app.clone();
                    tauri::async_runtime::spawn_blocking(move || {
                        if let Err(e) = crate::recording::stop_recording(&app) {
                            log::warn!("failed to stop recording: {e}");
                        }
                    });
                } else {
                    crate::overlay::trigger(&app, ShortcutCapture::RecordMonitor, Vec::new(), true);
                }
            }
        }
        // Independent on/off toggles for an already-running session's two
        // outputs — no-ops when nothing is recording; disabled on the
        // frontend in that case anyway.
        "toggle_local_recording" => {
            #[cfg(windows)]
            {
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(e) = crate::recording::toggle_local_recording(&app).await {
                        crate::notify_error(&app, &e);
                    }
                });
            }
        }
        "toggle_live_streaming" => {
            #[cfg(windows)]
            {
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(e) = crate::recording::toggle_live_streaming(&app).await {
                        crate::notify_error(&app, &e);
                    }
                });
            }
        }
        "toggle_buffer" => {
            #[cfg(windows)]
            {
                use crate::recording::replay_buffer::{self, ReplayBufferManager};
                if app.state::<Arc<ReplayBufferManager>>().is_running() {
                    let app = app.clone();
                    // stop joins encoder/cleanup threads — keep it off the IPC thread.
                    tauri::async_runtime::spawn_blocking(move || {
                        let _ = replay_buffer::stop_replay_buffer(&app);
                    });
                } else {
                    // Prefer the game shown on the context card over Settings'
                    // default target; avoids re-running foreground detection,
                    // unreliable while the wheel overlay itself has focus.
                    let target = app.state::<Arc<crate::games_db::GamesDb>>().current_target().map(|t| {
                        crate::config::ReplayBufferTarget::SpecificWindow { hwnd: t.hwnd, title: t.title, app: t.app }
                    });
                    let app = app.clone();
                    tauri::async_runtime::spawn(async move {
                        if let Err(e) = replay_buffer::start_replay_buffer_with_target(&app, target).await {
                            crate::notify_error(&app, &e);
                        }
                    });
                }
            }
        }
        "record_window" => crate::overlay::trigger(&app, ShortcutCapture::RecordWindow, Vec::new(), true),
        "record_area" => crate::overlay::trigger(&app, ShortcutCapture::RecordArea, Vec::new(), true),
        // Same start as `toggle_recording`, but also streams the session to
        // YouTube as a private live broadcast (guardrails: connected Google
        // account, H.264 encoder, AAC audio; failure falls back to local-only).
        "record_monitor_live" => {
            #[cfg(windows)]
            {
                if !app.state::<Arc<crate::recording::RecordingManager>>().is_recording() {
                    let app = app.clone();
                    tauri::async_runtime::spawn(async move {
                        let live = crate::recording::try_start_live_broadcast(&app, None).await;
                        if let Err(e) = crate::recording::start_monitor_recording_live(&app, live, folder, true).await {
                            crate::notify_error(&app, &e);
                        }
                    });
                }
            }
        }
        // Stream-only: unlike `record_monitor_live`, a failed broadcast means
        // nothing starts at all, since falling back to local-only would
        // ignore the user's explicit "no local file" choice.
        "record_monitor_stream_only" => {
            #[cfg(windows)]
            {
                if !app.state::<Arc<crate::recording::RecordingManager>>().is_recording() {
                    let app = app.clone();
                    tauri::async_runtime::spawn(async move {
                        let Some(live) = crate::recording::try_start_live_broadcast(&app, None).await else { return };
                        if let Err(e) = crate::recording::start_monitor_recording_live(&app, Some(live), folder, false).await {
                            crate::notify_error(&app, &e);
                        }
                    });
                }
            }
        }
        // Records the currently-detected game's window directly, using
        // `GamesDb::current_target` rather than the whole-monitor capture or
        // the interactive window-picker. No-op when no game is detected.
        "record_session" => {
            #[cfg(windows)]
            if let Some(t) = app.state::<Arc<crate::games_db::GamesDb>>().current_target() {
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(e) = crate::recording::start_window_recording_live(&app, t.hwnd, t.title, t.app, None, folder, true, false).await {
                        crate::notify_error(&app, &e);
                    }
                });
            }
        }
        // Same target resolution as `record_session`, but also streams to
        // YouTube, with the same guardrails/fallback as `record_monitor_live`.
        "record_session_live" => {
            #[cfg(windows)]
            if let Some(t) = app.state::<Arc<crate::games_db::GamesDb>>().current_target() {
                if !app.state::<Arc<crate::recording::RecordingManager>>().is_recording() {
                    let app = app.clone();
                    tauri::async_runtime::spawn(async move {
                        let live = crate::recording::try_start_live_broadcast(&app, Some(&t.app)).await;
                        if let Err(e) = crate::recording::start_window_recording_live(&app, t.hwnd, t.title, t.app, live, folder, true, false).await {
                            crate::notify_error(&app, &e);
                        }
                    });
                }
            }
        }
        // Stream-only counterpart of `record_session` — see
        // `record_monitor_stream_only` for why a failed broadcast means
        // nothing starts, rather than falling back to local-only.
        "record_session_stream_only" => {
            #[cfg(windows)]
            if let Some(t) = app.state::<Arc<crate::games_db::GamesDb>>().current_target() {
                if !app.state::<Arc<crate::recording::RecordingManager>>().is_recording() {
                    let app = app.clone();
                    tauri::async_runtime::spawn(async move {
                        let Some(live) = crate::recording::try_start_live_broadcast(&app, Some(&t.app)).await else { return };
                        if let Err(e) = crate::recording::start_window_recording_live(&app, t.hwnd, t.title, t.app, Some(live), folder, false, false).await {
                            crate::notify_error(&app, &e);
                        }
                    });
                }
            }
        }
        // Overlay variants — always-on-top/off-taskbar/ghost-styled, centered
        // on the cursor's monitor, so opening them while a game is running
        // doesn't pull focus away like the regular desktop window does.
        "open_gallery" => crate::tray::show_main_overlay(&app),
        "open_settings" => crate::tray::show_settings_overlay(&app),
        other => log::warn!("wheel: unknown action '{other}'"),
    }
}
