//! Floating region-frame recorder: an always-visible control bar
//! (`recorder`) plus a frame window (`recorder-frame`) with a dashed border
//! around the capture region. The frame's interior stays click-through (only
//! the border catches drags/resizes) via a backend cursor-position poll
//! that flips the whole window between interactive/click-through — carving
//! a region (`SetWindowRgn`) left ghost artifacts, and subclassing the
//! window proc broke WebView2 input. Window mode reuses the frame as a
//! passive tracker on a picked window. Opened from the gallery toolbar.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use tauri::{window::Color, AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

const LABEL: &str = "recorder";
const FRAME_LABEL: &str = "recorder-frame";

/// CSS border width of the frame's dashed outline (`recorder-frame/App.jsx`),
/// logical px — subtracted from the frame's outer rect to get the interior
/// that actually gets recorded.
const FRAME_BORDER_LOGICAL_PX: f64 = 6.0;
/// How far in from the frame's edge (logical px) still counts as the
/// interactive "border ring" for the cursor poll — wider than the visible
/// dashed line so the ring, and the resize handles sitting on it, are easy
/// to grab.
const BORDER_HIT_LOGICAL_PX: f64 = 14.0;

/// True while Area mode is active (frame is the user-positioned capture rect,
/// not a Window-mode tracker). Gates the cursor poll and bar-drag behavior.
static AREA_MODE: AtomicBool = AtomicBool::new(false);
/// The recorder's current logical mode (`"area"`/`"window"`/`"fullscreen"`),
/// or `None` when the recorder is closed. Mirrored to the gallery toolbar
/// (for its active-button highlight) and the recorder's own toolbar via the
/// `recorder-mode-changed` event.
static RECORDER_MODE: Mutex<Option<String>> = Mutex::new(None);
/// The last window picked via the picker grid (`{ hwnd, title, app }`), or
/// `None` once the recorder's fully closed. See `recorder_current_window_target`.
static PICKED_WINDOW: Mutex<Option<serde_json::Value>> = Mutex::new(None);

/// Records the current mode and broadcasts it so both toolbars stay in sync.
fn set_recorder_mode(app: &AppHandle, mode: Option<String>) {
    log::info!("set_recorder_mode: {mode:?}");
    *RECORDER_MODE.lock().unwrap() = mode.clone();
    use tauri::Emitter;
    let _ = app.emit("recorder-mode-changed", mode);
}
/// True between an Area-mode drag/resize `mousedown` and its release; keeps
/// the frame forced-interactive (so the cursor poll doesn't flip it
/// click-through mid-gesture as the cursor crosses the center).
static AREA_DRAGGING: AtomicBool = AtomicBool::new(false);

/// Last position we ourselves commanded, so the resulting Moved event can be
/// recognized as our own echo rather than a user drag (a toggled boolean
/// isn't reliable since the event isn't guaranteed to arrive before the
/// `set_position` call returns).
static BAR_COMMANDED: Mutex<Option<(i32, i32)>> = Mutex::new(None);
static FRAME_COMMANDED: Mutex<Option<(i32, i32)>> = Mutex::new(None);

/// Last physical position observed for the bar, used to compute the drag
/// delta when the user moves it so the frame can follow by the same amount.
static LAST_BAR_POS: Mutex<Option<(i32, i32)>> = Mutex::new(None);

fn move_bar_to(bar: &tauri::WebviewWindow, x: i32, y: i32) {
    *BAR_COMMANDED.lock().unwrap() = Some((x, y));
    let _ = bar.set_position(tauri::PhysicalPosition::new(x, y));
}

fn move_frame_to(frame: &tauri::WebviewWindow, x: i32, y: i32) {
    *FRAME_COMMANDED.lock().unwrap() = Some((x, y));
    let _ = frame.set_position(tauri::PhysicalPosition::new(x, y));
}

/// Logical size of the control bar window — kept in sync with the
/// `inner_size` passed to its `WebviewWindowBuilder` below.
const BAR_W: f64 = 560.0;
const BAR_H: f64 = 56.0;

/// Gap (logical px) kept between the frame and the attached control bar.
const ATTACH_GAP: f64 = 12.0;

/// Bottom-right corner position (logical px) for a window of the given
/// logical size — used for the control bar in Fullscreen mode (no frame to
/// attach to) and as a last-resort fallback.
fn corner_position(app: &AppHandle, win_w: f64, win_h: f64) -> Option<(f64, f64)> {
    let monitor = app.primary_monitor().ok().flatten()?;
    let scale = monitor.scale_factor();
    let size = monitor.size();
    let pos = monitor.position();
    const MARGIN: f64 = 24.0;
    let mx = pos.x as f64 / scale;
    let my = pos.y as f64 / scale;
    let mw = size.width as f64 / scale;
    let mh = size.height as f64 / scale;
    Some((mx + mw - win_w - MARGIN, my + mh - win_h - MARGIN))
}

/// Parks the control bar in its Fullscreen/Window-mode default corner — no
/// frame to attach to (Fullscreen), or none picked yet (Window).
fn park_bar_in_corner(app: &AppHandle) {
    if let Some(bar) = app.get_webview_window(LABEL) {
        if let (Some((x, y)), Ok(scale)) = (corner_position(app, BAR_W, BAR_H), bar.scale_factor()) {
            move_bar_to(&bar, (x * scale).round() as i32, (y * scale).round() as i32);
        }
    }
}

/// Picks where to park the control bar relative to the frame's *current*
/// rect (logical px): below it by default, above it if there's no room
/// below, otherwise to whichever side fits — so the bar always reads as
/// attached to the frame instead of floating separately over it.
fn attach_position(app: &AppHandle, fx: f64, fy: f64, fw: f64, fh: f64) -> Option<(f64, f64)> {
    let monitor = app.primary_monitor().ok().flatten()?;
    let mscale = monitor.scale_factor();
    let msize = monitor.size();
    let mpos = monitor.position();
    let mx = mpos.x as f64 / mscale;
    let my = mpos.y as f64 / mscale;
    let mw = msize.width as f64 / mscale;
    let mh = msize.height as f64 / mscale;

    let centered_x = (fx + fw / 2.0 - BAR_W / 2.0).clamp(mx, (mx + mw - BAR_W).max(mx));

    let below_y = fy + fh + ATTACH_GAP;
    if below_y + BAR_H <= my + mh {
        return Some((centered_x, below_y));
    }
    let above_y = fy - ATTACH_GAP - BAR_H;
    if above_y >= my {
        return Some((centered_x, above_y));
    }

    let centered_y = (fy + fh / 2.0 - BAR_H / 2.0).clamp(my, (my + mh - BAR_H).max(my));
    let right_x = fx + fw + ATTACH_GAP;
    if right_x + BAR_W <= mx + mw {
        return Some((right_x, centered_y));
    }
    let left_x = fx - ATTACH_GAP - BAR_W;
    if left_x >= mx {
        return Some((left_x, centered_y));
    }

    corner_position(app, BAR_W, BAR_H)
}

/// Moves the control bar to sit against a given frame rect (logical px).
fn reposition_bar_to_rect(app: &AppHandle, fx: f64, fy: f64, fw: f64, fh: f64) {
    let Some(bar) = app.get_webview_window(LABEL) else { return };
    let Ok(scale) = bar.scale_factor() else { return };
    if let Some((x, y)) = attach_position(app, fx, fy, fw, fh) {
        move_bar_to(&bar, (x * scale).round() as i32, (y * scale).round() as i32);
    }
}

/// Moves the control bar to sit against the frame's current rect (Area mode
/// only — see `on_frame_moved`). No-op if the frame isn't open.
pub fn reposition_bar(app: &AppHandle) {
    let Some(frame) = app.get_webview_window(FRAME_LABEL) else { return };
    let Ok(scale) = frame.scale_factor() else { return };
    let Ok(fpos) = frame.outer_position() else { return };
    let Ok(fsize) = frame.outer_size() else { return };
    reposition_bar_to_rect(
        app,
        fpos.x as f64 / scale,
        fpos.y as f64 / scale,
        fsize.width as f64 / scale,
        fsize.height as f64 / scale,
    );
}

/// Called on every Moved event for the control bar. A real user drag (not an
/// echo of our own [`reposition_bar`]/[`move_bar_to`] calls) carries the
/// Area-mode frame along by the same delta. In Window mode the frame is
/// pinned to the tracked window, so the bar moves on its own.
pub fn on_bar_moved(app: &AppHandle, x: i32, y: i32) {
    let commanded = BAR_COMMANDED.lock().unwrap().take();
    let prev = LAST_BAR_POS.lock().unwrap().replace((x, y));
    if commanded == Some((x, y)) {
        return; // echo of our own reposition_bar/corner_position call
    }
    if !AREA_MODE.load(Ordering::SeqCst) {
        return;
    }
    let Some((px, py)) = prev else { return };
    let (dx, dy) = (x - px, y - py);
    if dx == 0 && dy == 0 {
        return;
    }
    let Some(frame) = app.get_webview_window(FRAME_LABEL) else { return };
    let Ok(fpos) = frame.outer_position() else { return };
    move_frame_to(&frame, fpos.x + dx, fpos.y + dy);
}

/// Called on every Moved event for the frame. In Area mode, a real user drag
/// repositions the bar to stay attached (ignoring our own echoed moves). A
/// no-op in Window mode — the bar stays parked rather than chasing the frame.
pub fn on_frame_moved(app: &AppHandle, x: i32, y: i32) {
    let commanded = FRAME_COMMANDED.lock().unwrap().take();
    if commanded == Some((x, y)) || AREA_DRAGGING.load(Ordering::SeqCst) || !AREA_MODE.load(Ordering::SeqCst) {
        return;
    }
    reposition_bar(app);
}

/// Builds the control bar window once, reusing it (show/focus) on every
/// later call. Only ever reached from the *async* `open_recorder` command —
/// see that command's doc comment for why it must stay async.
pub fn open(app: &AppHandle) {
    let app2 = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Some(win) = app2.get_webview_window(LABEL) {
            let _ = win.show();
            let _ = win.set_focus();
            return;
        }
        let mut builder = WebviewWindowBuilder::new(&app2, LABEL, WebviewUrl::App("pages/recorder.html".into()))
            .title("Capcove")
            .inner_size(BAR_W, BAR_H)
            .resizable(false)
            .decorations(false)
            .always_on_top(true)
            .background_color(Color(15, 15, 15, 255))
            .visible(false);
        builder = match corner_position(&app2, BAR_W, BAR_H) {
            Some((x, y)) => builder.position(x, y),
            None => builder.center(),
        };
        let result = builder.build();
        match result {
            Ok(win) => {
                if let Ok(raw) = win.hwnd() {
                    crate::win_util::set_capture_hidden(raw.0 as usize as u32, true);
                }
            }
            Err(e) => log::warn!("recorder window could not be opened: {e}"),
        }
        // Shown by the frontend's own `window_ready` invoke once it mounts.
    });
}

/// Builds (or reuses) the region-frame window, always starting hidden, then
/// runs `then` with it — all on the main thread, so two concurrent async
/// callers can't both pass the "doesn't exist yet" check and each build a
/// duplicate window under the same label (only one is ever reachable again).
fn open_frame_and(app: &AppHandle, then: impl FnOnce(&AppHandle, tauri::WebviewWindow) + Send + 'static) {
    let app2 = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Some(win) = app2.get_webview_window(FRAME_LABEL) {
            then(&app2, win);
            return;
        }
        let result = WebviewWindowBuilder::new(&app2, FRAME_LABEL, WebviewUrl::App("pages/recorder-frame.html".into()))
            .title("Capcove")
            .inner_size(640.0, 360.0)
            .resizable(true)
            .decorations(false)
            .shadow(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .transparent(true)
            .background_color(Color(0, 0, 0, 0))
            .visible(false)
            .center()
            .build();
        match result {
            Ok(win) => {
                if let Ok(raw) = win.hwnd() {
                    crate::win_util::make_overlay_ghost(raw.0 as usize as u32);
                    crate::win_util::set_capture_hidden(raw.0 as usize as u32, true);
                }
                then(&app2, win);
            }
            Err(e) => log::warn!("recorder frame window could not be opened: {e}"),
        }
    });
}

/// Reads the frame's current rect in logical px (x, y, w, h).
fn frame_logical_rect(frame: &tauri::WebviewWindow) -> Option<(f64, f64, f64, f64)> {
    let scale = frame.scale_factor().ok()?;
    let pos = frame.outer_position().ok()?;
    let size = frame.outer_size().ok()?;
    Some((
        pos.x as f64 / scale,
        pos.y as f64 / scale,
        size.width as f64 / scale,
        size.height as f64 / scale,
    ))
}

/// Moves/resizes the frame to a logical-px rect (clamped to a minimum size)
/// and keeps the control bar attached to it. The one place Area-mode
/// drag/resize actually changes the frame.
fn apply_frame_rect(app: &AppHandle, x: f64, y: f64, w: f64, h: f64) {
    const MIN: f64 = 60.0;
    let w = w.max(MIN);
    let h = h.max(MIN);
    if let Some(frame) = app.get_webview_window(FRAME_LABEL) {
        let _ = frame.set_position(tauri::LogicalPosition::new(x, y));
        let _ = frame.set_size(tauri::LogicalSize::new(w, h));
    }
    reposition_bar_to_rect(app, x, y, w, h);
}

/// Applies a resize handle's cumulative drag delta to a baseline rect.
fn resize_rect(base: (f64, f64, f64, f64), handle: &str, dx: f64, dy: f64) -> (f64, f64, f64, f64) {
    let (mut x, mut y, mut w, mut h) = base;
    match handle {
        "North" => { y += dy; h -= dy; }
        "South" => { h += dy; }
        "West" => { x += dx; w -= dx; }
        "East" => { w += dx; }
        "NorthWest" => { x += dx; y += dy; w -= dx; h -= dy; }
        "NorthEast" => { y += dy; w += dx; h -= dy; }
        "SouthWest" => { x += dx; w -= dx; h += dy; }
        "SouthEast" => { w += dx; h += dy; }
        _ => {}
    }
    (x, y, w, h)
}

/// True if the global cursor is over the frame's interactive border ring
/// (inside the outer edge, but not in the click-through center).
fn cursor_over_border(frame: &tauri::WebviewWindow) -> bool {
    let (Ok(scale), Ok(pos), Ok(size)) = (frame.scale_factor(), frame.outer_position(), frame.outer_size()) else {
        return false;
    };
    let (cx, cy) = crate::capture::cursor_position(); // physical px
    let x0 = pos.x;
    let y0 = pos.y;
    let x1 = pos.x + size.width as i32;
    let y1 = pos.y + size.height as i32;
    if cx < x0 || cx >= x1 || cy < y0 || cy >= y1 {
        return false; // outside the frame entirely
    }
    let b = (BORDER_HIT_LOGICAL_PX * scale).round() as i32;
    let in_center = cx >= x0 + b && cx < x1 - b && cy >= y0 + b && cy < y1 - b;
    !in_center
}

/// The Area-mode cursor poll: flips the frame interactive when the cursor is
/// on the border ring (or dragging), click-through otherwise — always
/// click-through once recording starts. Exits when `epoch` is superseded.
fn start_area_cursor_poll(app: &AppHandle, epoch: u64) {
    use std::sync::Arc;
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut last_ignore: Option<bool> = None;
        loop {
            if FRAME_EPOCH.load(Ordering::SeqCst) != epoch {
                return;
            }
            let Some(frame) = app.get_webview_window(FRAME_LABEL) else { return };
            let recording = app.state::<Arc<crate::recording::RecordingManager>>().is_recording();
            let interactive = !recording && (AREA_DRAGGING.load(Ordering::SeqCst) || cursor_over_border(&frame));
            let ignore = !interactive;
            if last_ignore != Some(ignore) {
                let _ = frame.set_ignore_cursor_events(ignore);
                last_ignore = Some(ignore);
            }
            tokio::time::sleep(std::time::Duration::from_millis(40)).await;
        }
    });
}

/// True while the left mouse button is physically held down. Lets the drag
/// loop end itself on release even if the `mouseup` never reaches the
/// frontend (the cursor can leave the window mid-drag).
fn left_button_down() -> bool {
    #[cfg(windows)]
    {
        use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
        // VK_LBUTTON = 0x01; high bit set means the button is currently down.
        unsafe { (GetAsyncKeyState(0x01) as u16 & 0x8000) != 0 }
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// Starts an Area-mode drag/resize gesture. `handle` is `"move"` to drag the
/// whole frame, or a compass direction to resize from that edge/corner.
/// Driven entirely from the backend via polled `GetCursorPos` deltas (an
/// absolute screen coordinate, unaffected by the frame moving under it),
/// self-terminating on physical button release so a stray `mouseup` can't
/// strand the gesture.
#[tauri::command]
pub async fn recorder_area_drag_begin(app: AppHandle, handle: String) {
    let Some(frame) = app.get_webview_window(FRAME_LABEL) else { return };
    let Some(base) = frame_logical_rect(&frame) else { return };
    let _ = frame.set_ignore_cursor_events(false);
    let cursor_start = crate::capture::cursor_position(); // absolute physical px
    AREA_DRAGGING.store(true, Ordering::SeqCst);
    let epoch = FRAME_EPOCH.load(Ordering::SeqCst);
    let app2 = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            if FRAME_EPOCH.load(Ordering::SeqCst) != epoch || !AREA_DRAGGING.load(Ordering::SeqCst) {
                break;
            }
            if !left_button_down() {
                break;
            }
            let Some(frame) = app2.get_webview_window(FRAME_LABEL) else { break };
            let scale = frame.scale_factor().unwrap_or(1.0);
            let (cx, cy) = crate::capture::cursor_position();
            let dx = (cx - cursor_start.0) as f64 / scale;
            let dy = (cy - cursor_start.1) as f64 / scale;
            let (x, y, w, h) = if handle == "move" {
                (base.0 + dx, base.1 + dy, base.2, base.3)
            } else {
                resize_rect(base, &handle, dx, dy)
            };
            apply_frame_rect(&app2, x, y, w, h);
            tokio::time::sleep(std::time::Duration::from_millis(8)).await;
        }
        AREA_DRAGGING.store(false, Ordering::SeqCst);
    });
}

/// Ends the current drag/resize gesture early (from the frontend's
/// `mouseup`). The loop above also ends itself on button release, so this is
/// just the fast path for the common in-window release.
#[tauri::command]
pub async fn recorder_area_drag_end() {
    AREA_DRAGGING.store(false, Ordering::SeqCst);
}

/// Bumped whenever the frame's mode changes (Area poll start, Window-mode
/// track start, or a switch to Fullscreen), so whichever loop was running
/// before — the Area cursor poll or the Window tracking poll — knows to stop.
static FRAME_EPOCH: AtomicU64 = AtomicU64::new(0);

/// Re-applies the capture-hidden preference to both recorder windows if they
/// already exist — mirrors `wheel::apply_capture_hidden`. The frame is
/// *always* hidden from capture regardless of the user setting (it's UI
/// chrome around the recording, never meant to appear in it), only the
/// control bar follows the user's `hide_overlays_from_capture` preference.
pub fn apply_capture_hidden(app: &AppHandle, hidden: bool) {
    if let Some(win) = app.get_webview_window(LABEL) {
        if let Ok(raw) = win.hwnd() {
            crate::win_util::set_capture_hidden(raw.0 as usize as u32, hidden);
        }
    }
}

/// Dev-only: reveals the region frame for the store-screenshot automation's
/// own screen grab, since it's otherwise always hidden from capture.
#[cfg(debug_assertions)]
pub fn reveal_for_screenshot(app: &AppHandle) {
    for label in [LABEL, FRAME_LABEL] {
        if let Some(win) = app.get_webview_window(label) {
            if let Ok(raw) = win.hwnd() {
                crate::win_util::set_capture_hidden(raw.0 as usize as u32, false);
            }
        }
    }
}

#[tauri::command]
pub async fn open_recorder(app: AppHandle) {
    open(&app);
}

/// Like the generic `window_ready` command, but re-asserts `always_on_top`
/// afterward — that flag doesn't reliably survive Alt-Tab otherwise.
#[tauri::command]
pub fn recorder_window_ready(window: tauri::WebviewWindow) {
    let _ = window.show();
    let _ = window.set_focus();
    let _ = window.set_always_on_top(true);
}

/// Shows/re-shows the Area frame and (re)starts its cursor poll.
/// `reposition_bar_flag` is false when resuming from minimized, so the bar
/// stays where the user left it instead of snapping back to the frame.
fn activate_area_mode(app: &AppHandle, epoch: u64, reposition_bar_flag: bool) {
    AREA_MODE.store(true, Ordering::SeqCst);
    open_frame_and(app, move |app, win| {
        let _ = win.set_ignore_cursor_events(false); // grabbable until first poll tick
        let _ = win.show();
        if reposition_bar_flag {
            reposition_bar(app);
        }
        start_area_cursor_poll(app, epoch);
    });
}

/// Re-shows the Area frame after the bar comes back from being minimized,
/// without moving the bar — see `activate_area_mode`'s doc comment. The
/// window-mode equivalent is `recorder_track_window`, which already has this
/// same "don't reposition" behavior.
#[tauri::command]
pub async fn recorder_resume_area(app: AppHandle) {
    let epoch = FRAME_EPOCH.fetch_add(1, Ordering::SeqCst) + 1;
    activate_area_mode(&app, epoch, false);
}

/// Switches the control bar between Area mode (frame is a user-positioned
/// capture rect, border interactive via the cursor poll), Window mode (frame
/// stays hidden until a target is picked — see `recorder_track_window`), and
/// Fullscreen (no frame at all, bar parks in a screen corner).
#[tauri::command]
pub async fn recorder_set_mode(app: AppHandle, mode: String) {
    // Cancel whatever loop (Area poll / Window track) was running before.
    let epoch = FRAME_EPOCH.fetch_add(1, Ordering::SeqCst) + 1;
    AREA_DRAGGING.store(false, Ordering::SeqCst);
    match mode.as_str() {
        "area" => activate_area_mode(&app, epoch, true),
        // "window": frame stays hidden until a target is picked (see
        // `recorder_track_window`). "fullscreen"/anything else: no frame at all.
        _ => {
            AREA_MODE.store(false, Ordering::SeqCst);
            if let Some(win) = app.get_webview_window(FRAME_LABEL) {
                let _ = win.hide();
            }
            park_bar_in_corner(&app);
        }
    }
    set_recorder_mode(&app, Some(mode));
}

/// Opens the recorder directly into a given mode — the entry point for the
/// gallery toolbar's Area / Window / Fullscreen buttons.
///
/// Window mode is picker-first: the control bar only appears once a window
/// is actually picked (`recorder_pick_window_select` opens it), so choosing
/// "Window" from the gallery doesn't pop an empty control
/// bar on screen before you've even chosen what to record.
#[tauri::command]
pub async fn recorder_open_mode(app: AppHandle, mode: String) {
    if mode == "window" {
        recorder_pick_window(app).await;
    } else {
        open(&app);
        recorder_set_mode(app, mode).await;
    }
}

/// The recorder's current mode (`None` when closed) — the gallery queries
/// this on mount to seed its toolbar's active state.
#[tauri::command]
pub fn recorder_current_mode() -> Option<String> {
    RECORDER_MODE.lock().unwrap().clone()
}

const PICKER_LABEL: &str = "recorder-picker";

/// Downscales a captured window image (to keep the picker's payload small
/// and fast to encode) and JPEG-encodes it as a data URI.
fn thumbnail_data_uri(image: &image::RgbaImage) -> Option<String> {
    const MAX_W: u32 = 320;
    let (w, h) = (image.width(), image.height());
    if w == 0 || h == 0 {
        return None;
    }
    let scale = (MAX_W as f32 / w as f32).min(1.0);
    let tw = ((w as f32 * scale).round() as u32).max(1);
    let th = ((h as f32 * scale).round() as u32).max(1);
    let resized = image::imageops::resize(image, tw, th, image::imageops::FilterType::Triangle);
    crate::overlay::encode_overlay_jpeg(&resized).map(|b64| format!("data:image/jpeg;base64,{b64}"))
}

/// A window entry for the Window-mode picker grid, thumbnail included.
#[derive(Clone, serde::Serialize)]
pub struct WindowThumb {
    pub hwnd: u32,
    pub title: String,
    pub app: String,
    pub thumbnail: Option<String>,
}

/// Background-refreshed so opening the picker doesn't have to screenshot
/// every window on demand (that took long enough to show a loading state).
/// Empty until the first tick of `spawn_window_thumb_cache_loop`.
static WINDOW_THUMB_CACHE: Mutex<Vec<WindowThumb>> = Mutex::new(Vec::new());

/// `only_new`: re-capturing an actively-rendering window disrupts its
/// DirectX swap chain with a visible flicker, so background ticks
/// (`only_new = true`) only capture hwnds not already in the cache; only the
/// first cold-start scan captures everything.
fn scan_window_thumbs(only_new: bool) -> Vec<WindowThumb> {
    let previous = WINDOW_THUMB_CACHE.lock().unwrap().clone();
    crate::capture::list_windows()
        .into_iter()
        .map(|w| {
            let cached = previous.iter().find(|p| p.hwnd == w.id).and_then(|p| p.thumbnail.clone());
            let thumbnail = if only_new && cached.is_some() {
                cached
            } else {
                crate::capture::capture_window_thumbnail(w.id).and_then(|img| thumbnail_data_uri(&img))
            };
            WindowThumb { hwnd: w.id, title: w.title, app: w.app, thumbnail }
        })
        .collect()
}

/// Keeps `WINDOW_THUMB_CACHE` warm so the picker opens instantly. Ticks every
/// few seconds — often enough to feel live, rare enough (and skipped
/// entirely while a recording is running, or while neither the gallery nor
/// the recorder bar is open) to avoid the screenshot pass competing with a
/// game for frame time while the user isn't even near the picker.
pub fn spawn_window_thumb_cache_loop(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            let recording = app.state::<std::sync::Arc<crate::recording::RecordingManager>>().is_recording();
            let picker_relevant = app.get_webview_window("main").is_some() || app.get_webview_window(LABEL).is_some();
            if !recording && picker_relevant {
                let fresh = tauri::async_runtime::spawn_blocking(|| scan_window_thumbs(true)).await.unwrap_or_default();
                *WINDOW_THUMB_CACHE.lock().unwrap() = fresh;
            }
            tokio::time::sleep(std::time::Duration::from_secs(4)).await;
        }
    });
}

/// Lists capturable windows with a small preview image each — backs the
/// picker grid (`recorder-picker` window). Serves the background-refreshed
/// cache when available; falls back to a synchronous scan on cold start
/// (before the loop's first tick) or if it's momentarily empty.
#[tauri::command]
pub async fn recorder_list_window_thumbs() -> Vec<WindowThumb> {
    let cached = WINDOW_THUMB_CACHE.lock().unwrap().clone();
    if !cached.is_empty() {
        return cached;
    }
    let fresh = tauri::async_runtime::spawn_blocking(|| scan_window_thumbs(false)).await.unwrap_or_default();
    *WINDOW_THUMB_CACHE.lock().unwrap() = fresh.clone();
    fresh
}

/// Switches to Window mode and opens the thumbnail picker — the recorder's
/// own toolbar and the gallery's Window button both land here.
#[tauri::command]
pub async fn recorder_pick_window(app: AppHandle) {
    recorder_set_mode(app.clone(), "window".into()).await;
    open_picker(&app);
}

/// Builds (or reuses/refocuses) the Window-mode picker grid — a small
/// Alt-Tab-style modal of window thumbnails, not the full-screen "click the
/// real window under your cursor" overlay used by the record-window
/// shortcut. Routed through `run_on_main_thread` for the same reasons as
/// `open`/`open_frame_and` above.
fn open_picker(app: &AppHandle) {
    let app2 = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Some(win) = app2.get_webview_window(PICKER_LABEL) {
            let _ = win.show();
            let _ = win.set_focus();
            return;
        }
        let (lw, lh) = app2
            .primary_monitor()
            .ok()
            .flatten()
            .map(|m| {
                let scale = m.scale_factor();
                let size = m.size();
                ((size.width as f64 / scale * 0.7).max(720.0), (size.height as f64 / scale * 0.7).max(480.0))
            })
            .unwrap_or((900.0, 620.0));
        let result = WebviewWindowBuilder::new(&app2, PICKER_LABEL, WebviewUrl::App("pages/recorder-picker.html".into()))
            .title("Capcove")
            .inner_size(lw, lh)
            .resizable(false)
            .decorations(false)
            .shadow(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .background_color(Color(15, 15, 15, 255))
            .visible(false)
            .center()
            .build();
        if let Err(e) = result {
            log::warn!("recorder picker window could not be opened: {e}");
        }
    });
}

/// Shown by the picker's own frontend once it mounts.
#[tauri::command]
pub fn recorder_picker_ready(window: tauri::WebviewWindow) {
    let _ = window.show();
    let _ = window.set_focus();
}

/// `async` — closes the picker window from *inside its own* IPC callback
/// otherwise, which is the same reentrancy hazard `open_recorder`'s doc
/// comment covers for window creation, just on the close/destroy path.
#[tauri::command]
pub async fn recorder_cancel_picker(app: AppHandle) {
    if let Some(win) = app.get_webview_window(PICKER_LABEL) {
        let _ = win.close();
    }
    // The gallery's picker-first "Window" launch never opened the control
    // bar (see `recorder_open_mode`) — if there's nothing else open, back
    // all the way out instead of leaving the mode stuck on "window".
    if app.get_webview_window(LABEL).is_none() {
        set_recorder_mode(&app, None);
    }
}

/// Called by the picker's frontend when the user clicks a window card.
/// Closes the picker, brings the picked window to front, opens/focuses the
/// control bar, starts tracking with the frame, and tells the bar's
/// frontend which window it is (`recorder_start` needs that target).
///
/// `async`: closing the picker then building/showing two more windows
/// synchronously froze the app. The small sleeps between steps guard
/// against a rare wry null-deref crash (`parent_subclass_proc`, wry 0.55.1)
/// triggered by back-to-back focus/move churn.
#[tauri::command]
pub async fn recorder_pick_window_select(app: AppHandle, hwnd: u32, title: String, app_name: String) {
    const STEP_DELAY: std::time::Duration = std::time::Duration::from_millis(60);
    if let Some(win) = app.get_webview_window(PICKER_LABEL) {
        let _ = win.close();
    }
    tokio::time::sleep(STEP_DELAY).await;
    crate::win_util::bring_window_to_foreground(hwnd);
    tokio::time::sleep(STEP_DELAY).await;
    // Persisted, not just emitted: a picker-first pick can reach here before
    // the bar even exists, so the event alone could fire before anyone
    // listens. The bar's mount effect pulls this on its own schedule instead
    // (see `recorder_current_window_target`).
    let payload = serde_json::json!({ "hwnd": hwnd, "title": title, "app": app_name });
    *PICKED_WINDOW.lock().unwrap() = Some(payload.clone());
    open(&app);
    tokio::time::sleep(STEP_DELAY).await;
    start_window_tracking(&app, hwnd, true);
    if let Some(bar) = app.get_webview_window(LABEL) {
        use tauri::Emitter;
        let _ = bar.emit("recorder-window-picked", payload);
    }
}

/// Last window picked via the picker grid — the bar's mount effect queries
/// this to seed `windowTarget`, since the `recorder-window-picked` event
/// alone can't be relied on to reach a bar that's still loading (see
/// `recorder_pick_window_select`).
#[tauri::command]
pub fn recorder_current_window_target() -> Option<serde_json::Value> {
    PICKED_WINDOW.lock().unwrap().clone()
}

/// Shows the frame over `hwnd` and keeps it snapped to that window's
/// on-screen rect (polling every 150ms) until the mode changes or the
/// window closes. Fully click-through the whole time — it's purely a visual
/// "this is what's being recorded" indicator here, not something the user
/// drags/resizes directly (unlike Area mode).
///
/// `reposition_bar_flag` is false when called from `recorder_track_window`
/// (resuming after the bar was minimized) — minimizing/restoring never moves
/// a window, so the bar should come back exactly where the user left it
/// instead of snapping back into the corner.
fn start_window_tracking(app: &AppHandle, hwnd: u32, reposition_bar_flag: bool) {
    AREA_MODE.store(false, Ordering::SeqCst);
    AREA_DRAGGING.store(false, Ordering::SeqCst);
    let epoch = FRAME_EPOCH.fetch_add(1, Ordering::SeqCst) + 1;
    open_frame_and(app, move |app, win| {
        let _ = win.set_ignore_cursor_events(true);
        let _ = win.show();

        if reposition_bar_flag {
            // Parking the bar and re-focusing it happen a beat after the frame's
            // own `show()` rather than in the same tick — see
            // `recorder_pick_window_select`'s doc comment on why these are
            // spaced out (defense-in-depth against a rare WebView2 crash, not a
            // proven fix).
            let app_for_bar = app.clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(60)).await;
                // Unlike Area mode's user-sized rect, a tracked window can be huge
                // (e.g. maximized) — attaching the bar below/above it could push it
                // off-screen or somewhere awkward. Park it in the corner instead,
                // same as Fullscreen mode.
                park_bar_in_corner(&app_for_bar);
                tokio::time::sleep(std::time::Duration::from_millis(60)).await;
                // Both the bar and frame are always-on-top; since the frame's own
                // `show()` already ran, it can end up above the bar in that topmost
                // band. Re-focusing the bar last brings it back to the front.
                if let Some(bar) = app_for_bar.get_webview_window(LABEL) {
                    let _ = bar.set_focus();
                }
            });
        }

        let app2 = app.clone();
        tauri::async_runtime::spawn(async move {
            // DWM's extended-frame-bounds query can transiently fail for a
            // moment right after `bring_window_to_foreground` restores a
            // minimized window — that miss doesn't mean the window closed.
            // Only give up after several consecutive misses (~1.5s); a real
            // close means every subsequent poll fails, not just the first.
            let mut misses = 0u32;
            const MAX_MISSES: u32 = 10;
            loop {
                if FRAME_EPOCH.load(Ordering::SeqCst) != epoch {
                    return; // superseded by a mode change or a newer pick
                }
                match crate::capture::window_frame_rect(hwnd) {
                    Some((x, y, w, h)) => {
                        misses = 0;
                        if let Some(frame) = app2.get_webview_window(FRAME_LABEL) {
                            let _ = frame.set_position(tauri::PhysicalPosition::new(x, y));
                            let _ = frame.set_size(tauri::PhysicalSize::new(w, h));
                        }
                    }
                    None => {
                        misses += 1;
                        if misses > MAX_MISSES {
                            return; // window really did close
                        }
                    }
                }
                tokio::time::sleep(std::time::Duration::from_millis(150)).await;
            }
        });
    });
}

/// Resumes tracking a previously-picked window — used when the bar's own
/// `onFocusChanged` listener re-shows the frame after being minimized.
#[tauri::command]
pub async fn recorder_track_window(app: AppHandle, hwnd: u32) -> Result<(), String> {
    start_window_tracking(&app, hwnd, false);
    Ok(())
}

/// A picked Window-mode recording target, sent up from the frontend's picker.
#[derive(serde::Deserialize)]
pub struct WindowTarget {
    pub hwnd: u32,
    pub title: String,
    pub app: String,
}

/// Starts a recording for the given mode:
/// - `"area"`: records the frame's interior (its outer rect minus the dashed
///   border), so the recording matches exactly what's inside the frame.
/// - `"window"`: records the picked `window` target.
/// - anything else: records the primary monitor.
///
/// `live` pre-arms a YouTube live broadcast before starting, matching the
/// wheel's `record_monitor_live` action — a failed broadcast just falls back
/// to a local-only recording rather than blocking the start.
#[tauri::command]
pub async fn recorder_start(app: AppHandle, mode: String, live: bool, window: Option<WindowTarget>) -> Result<(), String> {
    log::info!(
        "recorder_start called: mode={mode:?} live={live} window={:?}",
        window.as_ref().map(|w| (w.hwnd, &w.title, &w.app))
    );
    #[cfg(windows)]
    {
        let broadcast = if live { crate::recording::try_start_live_broadcast(&app, None).await } else { None };
        if mode == "area" {
            let frame = app.get_webview_window(FRAME_LABEL).ok_or("Area frame is not open")?;
            let scale = frame.scale_factor().map_err(|e| e.to_string())?;
            let pos = frame.outer_position().map_err(|e| e.to_string())?;
            let size = frame.outer_size().map_err(|e| e.to_string())?;
            let border = (FRAME_BORDER_LOGICAL_PX * scale).round() as i32;
            let px = pos.x + border;
            let py = pos.y + border;
            let pw = (size.width as i32 - border * 2).max(2) as u32;
            let ph = (size.height as i32 - border * 2).max(2) as u32;
            crate::recording::start_area_recording_live(&app, px, py, pw, ph, broadcast).await?;
        } else if mode == "window" {
            let target = window.ok_or("No window selected")?;
            // force_own_audio=true: the user explicitly picked this window to
            // record, so its own process audio should always get captured —
            // independent of the "Game audio only"/separate-tracks settings
            // that gate the equivalent auto-detected-game behavior elsewhere.
            crate::recording::start_window_recording_live(&app, target.hwnd, target.title, target.app, broadcast, None, true, true).await?;
        } else {
            crate::recording::start_monitor_recording_live(&app, broadcast, None, true).await?;
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = (mode, live, window);
        Err("Recording is only supported on Windows in this version.".into())
    }
}

/// Sets the control bar's own window opacity (0-100%), so it can be made
/// unobtrusive while parked over whatever's being recorded in Fullscreen mode.
#[tauri::command]
pub fn recorder_set_opacity(app: AppHandle, percent: u8) {
    if let Some(win) = app.get_webview_window(LABEL) {
        if let Ok(raw) = win.hwnd() {
            let alpha = ((percent.min(100) as u32 * 255) / 100) as u8;
            crate::win_util::set_window_opacity(raw.0 as usize as u32, alpha.max(40));
        }
    }
}

/// Minimizes the control bar and hides the frame with it (rather than
/// leaving a floating frame with no way to bring its bar back to front).
/// The frame reappears when the bar regains focus (see the frontend's
/// `onFocusChanged` listener), if Area mode is still selected.
#[tauri::command]
pub fn recorder_minimize(app: AppHandle) {
    if let Some(frame) = app.get_webview_window(FRAME_LABEL) {
        let _ = frame.hide();
    }
    if let Some(bar) = app.get_webview_window(LABEL) {
        let _ = bar.minimize();
    }
}

/// Closes the frame and the control bar together — the frame has no purpose
/// without the bar it's attached to.
///
/// `async`: this is invoked from the bar's own close button, so a
/// synchronous version would close the bar from inside its own IPC
/// callback — the same reentrancy hazard fixed above for the picker.
#[tauri::command]
pub async fn recorder_close(app: AppHandle) {
    FRAME_EPOCH.fetch_add(1, Ordering::SeqCst); // stop any running loop
    if let Some(frame) = app.get_webview_window(FRAME_LABEL) {
        let _ = frame.close();
    }
    if let Some(bar) = app.get_webview_window(LABEL) {
        let _ = bar.close();
    }
    *PICKED_WINDOW.lock().unwrap() = None;
    set_recorder_mode(&app, None);
}

