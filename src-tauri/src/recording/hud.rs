//! On-screen status badges for recording, replay buffer, and an active
//! microphone track. Purely informational and click-through.
//!
//! Two rendering backends exist side by side, same idea as `crate::toast`:
//! - `webview`: the original, a WebviewWindow (`pages/rec-hud.html`).
//!   Excluded from `tauri_plugin_window_state`, whose geometry restore would
//!   fight this window's own positioning.
//! - `crate::hud_native`: an experimental GDI-drawn layered window, no
//!   WebView2 involved. All the state/settings/monitor-anchoring logic below
//!   is shared between both — only the actual window/drawing differs.
//!
//! `USE_NATIVE` is the only thing that picks between them.

use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Manager, Monitor};

use crate::config::{AudioSource, ConfigStore};
use super::replay_buffer::ReplayBufferManager;
use super::{RecordTarget, RecordingManager, RecordingSession};
use crate::config::ReplayBufferTarget;

/// Flip to `false` to go back to the original WebView2-hosted HUD — both
/// implementations are kept fully intact. See `toast.rs`'s matching switch
/// for why this needs the `#[cfg(windows)]` gate, not just a plain `bool`.
#[cfg(windows)]
const USE_NATIVE: bool = true;

/// Each badge is a fixed-size circle; the window is sized to fit however
/// many badges are visible, laid out side by side with `GAP` — see
/// `compute_size`.
const BADGE: f64 = 28.0;
const GAP: f64 = 6.0;

/// Which capture pipelines (recording, replay buffer) are actually running
/// right now — not gated by the user's per-badge on/off settings, unlike
/// `Badges` below.
#[derive(Default, Clone, Copy)]
struct RawState {
    recording: bool,
    buffer: bool,
}

static RAW: Mutex<RawState> = Mutex::new(RawState { recording: false, buffer: false });

/// The actually-visible state sent to the frontend: `RawState` filtered
/// through the user's enabled/disabled settings, plus each badge's icon.
/// Cached so `get_hud_badges` answers with the window's last-shown state.
#[derive(Default, Clone, serde::Serialize)]
pub struct Badges {
    pub(crate) recording: bool,
    pub(crate) buffer: bool,
    pub(crate) mic: bool,
    pub(crate) recording_icon: String,
    pub(crate) buffer_icon: String,
    pub(crate) mic_icon: String,
}

impl Badges {
    fn count(&self) -> usize {
        [self.recording, self.buffer, self.mic].iter().filter(|v| **v).count()
    }
}

static VISIBLE: Mutex<Option<Badges>> = Mutex::new(None);

/// Returns the current badge state — called by the HUD frontend on load to
/// paint its initial state directly, sidestepping the race where an emitted
/// event fires before the page's listener is registered.
#[tauri::command]
pub fn get_hud_badges() -> Badges {
    VISIBLE.lock().unwrap().clone().unwrap_or_default()
}

/// Whether a microphone track is part of the recording/buffer audio
/// configuration — checks configured sources, not that the device is open.
fn mic_configured(app: &AppHandle) -> bool {
    let audio = app.state::<Arc<ConfigStore>>().get().video.audio;
    !audio.mic_muted && audio.sources.iter().any(|s| matches!(s, AudioSource::Microphone { .. }))
}

/// Picks the monitor to anchor the HUD to: prefers the active recording's
/// target (if any), then the active replay buffer's, then the primary
/// monitor as a last resort.
fn anchor_monitor(app: &AppHandle) -> Option<Monitor> {
    if let Some(session) = app.state::<Arc<RecordingManager>>().current_session() {
        if let Some(m) = target_monitor(app, &session.target) {
            return Some(m);
        }
    }
    if let Some(target) = app.state::<Arc<ReplayBufferManager>>().current_target() {
        let point = match target {
            ReplayBufferTarget::SpecificWindow { hwnd, .. } => crate::capture::window_frame_rect(hwnd).map(|(x, y, _, _)| (x, y)),
            ReplayBufferTarget::PrimaryMonitor => None,
        };
        if let Some((x, y)) = point {
            if let Ok(Some(m)) = app.monitor_from_point(x as f64, y as f64) {
                return Some(m);
            }
        }
    }
    app.primary_monitor().ok().flatten()
}

/// Picks the monitor for a specific recording target: for `Window`/`Area`,
/// the monitor under the region's top-left corner; otherwise the primary
/// monitor. Must run on the main thread — monitor lookups are unreliable off it.
fn target_monitor(app: &AppHandle, target: &RecordTarget) -> Option<Monitor> {
    let point = match target {
        RecordTarget::Window { hwnd, .. } => crate::capture::window_frame_rect(*hwnd).map(|(x, y, _, _)| (x, y)),
        RecordTarget::Area { x, y, .. } => Some((*x, *y)),
        RecordTarget::Monitor => None,
    };
    point
        .and_then(|(x, y)| app.monitor_from_point(x as f64, y as f64).ok().flatten())
        .or_else(|| app.primary_monitor().ok().flatten())
}

/// Total window size for `count` visible badges, laid out side by side.
fn compute_size(count: usize) -> (f64, f64) {
    let w = (count as f64) * BADGE + (count.saturating_sub(1).max(0) as f64) * GAP;
    (w.max(BADGE), BADGE)
}

/// Recomputes `Badges` from `RAW` + the current `hud_badges` settings and
/// applies it. Hides once nothing is visible; shows it on the way back up.
fn refresh(app: &AppHandle) {
    let app2 = app.clone();
    let _ = app.run_on_main_thread(move || {
        let raw = *RAW.lock().unwrap();
        let cfg = app2.state::<Arc<ConfigStore>>().get().video;
        let hb = cfg.hud_badges;
        let mic = (raw.recording || raw.buffer) && hb.mic_enabled && mic_configured(&app2);
        let badges = Badges {
            recording: raw.recording && hb.recording_enabled,
            buffer: raw.buffer && hb.buffer_enabled,
            mic,
            recording_icon: hb.recording_icon,
            buffer_icon: hb.buffer_icon,
            mic_icon: hb.mic_icon,
        };
        *VISIBLE.lock().unwrap() = Some(badges.clone());
        let count = badges.count();

        #[cfg(windows)]
        if USE_NATIVE {
            if count == 0 {
                crate::hud_native::hide(&app2);
                return;
            }
            let corner = cfg.hud_corner;
            let Some(monitor) = anchor_monitor(&app2) else {
                log::warn!("recording HUD: no monitor found to anchor to");
                return;
            };
            let (w, h) = compute_size(count);
            crate::hud_native::render(&app2, &badges, &monitor, corner, w as i32, h as i32);
            return;
        }

        if count == 0 {
            webview::hide(&app2);
            return;
        }
        let corner = cfg.hud_corner;
        let Some(monitor) = anchor_monitor(&app2) else {
            log::warn!("recording HUD: no monitor found to anchor to");
            return;
        };
        let (w, h) = compute_size(count);
        webview::show_window(&app2, &badges, &monitor, corner, w, h);
    });
}

/// Turns the recording badge on for the duration of `session`.
pub fn show(app: &AppHandle, _session: &RecordingSession) {
    RAW.lock().unwrap().recording = true;
    refresh(app);
}

/// Turns the recording badge off.
pub fn hide(app: &AppHandle) {
    RAW.lock().unwrap().recording = false;
    refresh(app);
}

/// Turns the replay-buffer badge on/off.
pub fn set_buffer(app: &AppHandle, running: bool) {
    RAW.lock().unwrap().buffer = running;
    refresh(app);
}

/// Re-applies `hud_corner`/`hud_badges` to an already-visible HUD, or
/// shows/hides it if enabled state changed — a no-op if nothing should be
/// visible.
pub fn reanchor(app: &AppHandle) {
    refresh(app);
}

/// Re-applies the capture-hidden preference to the HUD window if it's
/// currently on screen — see `wheel::apply_capture_hidden` for why.
pub fn apply_capture_hidden(app: &AppHandle, hidden: bool) {
    #[cfg(windows)]
    if USE_NATIVE {
        crate::hud_native::apply_capture_hidden(app, hidden);
        return;
    }
    webview::apply_capture_hidden(app, hidden);
}

// `USE_NATIVE = true` makes the compiler correctly (but harmlessly) flag
// some of this as unreachable — a snapshot of the current toggle, not a
// sign anything here is actually dead; flip `USE_NATIVE` and this becomes
// the live path again.
#[allow(dead_code)]
mod webview {
    use std::sync::Arc;
    use tauri::{window::Color, AppHandle, Emitter, Manager, Monitor, PhysicalPosition, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

    use crate::config::{ConfigStore, HudCorner};
    use super::Badges;

    const LABEL: &str = "rec-hud";

    /// Computes the exact physical-pixel position for `corner`, flush against
    /// the monitor's full physical bounds (not `work_area()` — the HUD sits on
    /// top of the taskbar), using the window's actual queried physical size.
    fn corner_physical_position(monitor: &Monitor, win: &WebviewWindow, corner: HudCorner) -> PhysicalPosition<i32> {
        const MARGIN_PX: i32 = 6;

        let mpos = monitor.position();
        let msize = monitor.size();
        let wsize = win.outer_size().unwrap_or(tauri::PhysicalSize::new(28, 28));
        let (ww, wh) = (wsize.width as i32, wsize.height as i32);
        let (mw, mh) = (msize.width as i32, msize.height as i32);

        let (x, y) = match corner {
            HudCorner::TopLeft => (MARGIN_PX, MARGIN_PX),
            HudCorner::TopRight => (mw - ww - MARGIN_PX, MARGIN_PX),
            HudCorner::BottomLeft => (MARGIN_PX, mh - wh - MARGIN_PX),
            HudCorner::BottomRight => (mw - ww - MARGIN_PX, mh - wh - MARGIN_PX),
        };
        PhysicalPosition::new(mpos.x + x, mpos.y + y)
    }

    /// `WebviewWindow::set_always_on_top` alone isn't enough to consistently
    /// stay above the Windows taskbar, so use the raw Win32 call directly:
    /// `SetWindowPos` with `HWND_TOPMOST`.
    fn force_topmost(win: &WebviewWindow) {
        use windows::Win32::Foundation::HWND;
        use windows::Win32::UI::WindowsAndMessaging::{SetWindowPos, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE};
        let Ok(raw) = win.hwnd() else { return };
        let hwnd = HWND(raw.0 as usize as *mut _);
        unsafe {
            let _ = SetWindowPos(hwnd, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        }
    }

    /// Re-asserts topmost periodically while the HUD window exists, and
    /// self-terminates once it's closed. See `force_topmost` for why a one-shot
    /// `always_on_top(true)` isn't enough.
    fn start_topmost_loop(app: &AppHandle) {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                let Some(win) = app.get_webview_window(LABEL) else { break };
                force_topmost(&win);
            }
        });
    }

    /// Sizes and anchors the window into `corner` of `monitor`, then shows it.
    /// Order matters on mixed-DPI setups: move to the monitor first, then set
    /// the logical size, then compute the corner position from the physical size.
    fn place_and_show(win: &WebviewWindow, monitor: &Monitor, corner: HudCorner, w: f64, h: f64) {
        let mpos = monitor.position();
        let _ = win.set_position(PhysicalPosition::new(mpos.x, mpos.y));
        let _ = win.set_size(tauri::LogicalSize::new(w, h));
        let pos = corner_physical_position(monitor, win, corner);
        let _ = win.set_position(pos);
        let _ = win.show();
        force_topmost(win);
    }

    pub fn hide(app: &AppHandle) {
        if let Some(win) = app.get_webview_window(LABEL) {
            let _ = win.close();
        }
    }

    pub fn show_window(app: &AppHandle, badges: &Badges, monitor: &Monitor, corner: HudCorner, w: f64, h: f64) {
        if let Some(win) = app.get_webview_window(LABEL) {
            let _ = win.emit("hud-badges", badges);
            place_and_show(&win, monitor, corner, w, h);
            return;
        }

        let build = || -> tauri::Result<()> {
            let win = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("pages/rec-hud.html".into()))
                .title("Capcove")
                .decorations(false)
                .shadow(false)
                .always_on_top(true)
                .skip_taskbar(true)
                .resizable(false)
                .transparent(true)
                .background_color(Color(0, 0, 0, 0))
                .focused(false)
                .visible(false)
                .position(monitor.position().x as f64, monitor.position().y as f64)
                .inner_size(w, h)
                .build()?;
            let _ = win.set_ignore_cursor_events(true);
            // Ghost window: pops in without the DWM open animation, and (if
            // not opted out) stays out of our own recordings and others' capture.
            if let Ok(raw) = win.hwnd() {
                let hwnd_u32 = raw.0 as usize as u32;
                crate::win_util::make_overlay_ghost(hwnd_u32);
                let hide = app.state::<Arc<ConfigStore>>().get().hide_overlays_from_capture;
                crate::win_util::set_capture_hidden(hwnd_u32, hide);
            }
            place_and_show(&win, monitor, corner, w, h);
            Ok(())
        };
        match build() {
            Ok(()) => start_topmost_loop(app),
            Err(e) => log::warn!("recording HUD could not be opened: {e}"),
        }
    }

    /// Re-applies the capture-hidden preference to the HUD window if it's
    /// currently on screen — see `wheel::apply_capture_hidden` for why.
    pub fn apply_capture_hidden(app: &AppHandle, hidden: bool) {
        if let Some(win) = app.get_webview_window(LABEL) {
            if let Ok(raw) = win.hwnd() {
                crate::win_util::set_capture_hidden(raw.0 as usize as u32, hidden);
            }
        }
    }
}
