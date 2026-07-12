//! The window/area picker overlay — transparent windows that let the user
//! pick a recording target (a window, or drag out a rectangle), then start a
//! recording.

use crate::{
    capture,
    config::{ShortcutAction, ShortcutCapture},
    MonitorInfo, OverlayMode, Pending, PendingCapture,
};
use image::RgbaImage;
#[allow(unused_imports)]
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

pub(crate) fn encode_overlay_jpeg(image: &RgbaImage) -> Option<String> {
    use base64::Engine;
    let (w, h) = (image.width(), image.height());
    let mut buf = Vec::new();
    let mut rgb = Vec::with_capacity((w * h * 3) as usize);
    let raw = image.as_raw();
    for chunk in raw.chunks_exact(4) {
        rgb.push(chunk[0]);
        rgb.push(chunk[1]);
        rgb.push(chunk[2]);
    }
    let rgb_buf = image::ImageBuffer::<image::Rgb<u8>, Vec<u8>>::from_raw(w, h, rgb)?;
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, 60);
    encoder.encode_image(&rgb_buf).ok()?;
    Some(base64::engine::general_purpose::STANDARD.encode(buf))
}

#[allow(dead_code)]
#[derive(serde::Deserialize)]
pub(crate) struct HighlightRect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

pub fn trigger(app: &AppHandle, capture: ShortcutCapture, _actions: Vec<ShortcutAction>, multi_monitor: bool) {
    match capture {
        #[cfg(windows)]
        ShortcutCapture::RecordWindow => trigger_record_window(app, multi_monitor),
        #[cfg(windows)]
        ShortcutCapture::RecordArea => trigger_record_area(app),
        #[cfg(windows)]
        ShortcutCapture::RecordMonitor => {
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = crate::recording::start_monitor_recording(&app).await {
                    crate::notify_error(&app, &e);
                }
            });
        }
        #[cfg(not(windows))]
        ShortcutCapture::RecordWindow | ShortcutCapture::RecordArea | ShortcutCapture::RecordMonitor => {
            crate::notify_error(app, "Recording is only supported on Windows in this version.");
        }
    }
}

/// Opens the window-picker overlay; the pick starts a recording — consumed
/// by `commands::overlay_cmd::window_selected`.
#[cfg(windows)]
fn trigger_record_window(app: &AppHandle, multi_monitor: bool) {
    if is_any_overlay_open(app) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let (monitors, windows) = tokio::join!(
            tauri::async_runtime::spawn_blocking(capture::list_monitors),
            tauri::async_runtime::spawn_blocking(capture::list_windows),
        );

        let mut monitors = monitors.unwrap_or_default();
        let windows = match windows {
            Ok(w) => w,
            Err(e) => {
                crate::notify_error(&app, &e.to_string());
                return;
            }
        };
        // Resolve each window's raw exe stem through the games catalog/custom
        // list the same way `game_detect::detect_game` does, so a manually
        // window-picked recording gets the same app name as an auto-detected one.
        let games = app.state::<std::sync::Arc<crate::games_db::GamesDb>>();
        let windows: Vec<capture::WinInfo> = windows
            .into_iter()
            .map(|mut w| {
                let exe_path = capture::window_exe_path(w.id);
                if let Some(entry) = games.lookup(&w.app, exe_path.as_deref()) {
                    w.app = entry.name;
                }
                w
            })
            .collect();

        if monitors.is_empty() {
            crate::notify_error(&app, "No monitors found");
            return;
        }

        if !multi_monitor {
            let (cx, cy) = capture::cursor_position();
            if let Some(pos) = monitors.iter().position(|m| {
                cx >= m.x && cx < m.x + m.w as i32 && cy >= m.y && cy < m.y + m.h as i32
            }) {
                monitors = vec![monitors.swap_remove(pos)];
            } else {
                monitors.truncate(1);
            }
        }

        let monitors_clone = monitors.clone();
        let mon_jpegs = tauri::async_runtime::spawn_blocking(move || {
            let mut mon_jpegs = Vec::new();
            for mon in &monitors_clone {
                let cx = mon.x + (mon.w / 2) as i32;
                let cy = mon.y + (mon.h / 2) as i32;
                match capture::capture_monitor_at(cx, cy) {
                    Ok(shot) => {
                        let jpeg = encode_overlay_jpeg(&shot.image)
                            .map(|b64| format!("data:image/jpeg;base64,{b64}"))
                            .unwrap_or_default();
                        mon_jpegs.push(jpeg);
                    }
                    Err(_) => mon_jpegs.push(String::new()),
                }
            }
            mon_jpegs
        })
        .await
        .unwrap_or_default();

        let first = &monitors[0];
        let first_jpeg = mon_jpegs.first().cloned();

        *app.state::<Pending>().0.lock().unwrap() = Some(PendingCapture {
            image_jpeg: if multi_monitor { None } else { first_jpeg },
            scale: first.scale,
            mode: OverlayMode::RecordWindow,
            windows,
            mon_x: first.x,
            mon_y: first.y,
            mon_w: first.w,
            mon_h: first.h,
            monitors: if multi_monitor { monitors.clone() } else { Vec::new() },
            mon_jpegs: if multi_monitor { mon_jpegs } else { Vec::new() },
            live_mode: false,
        });

        for (i, mon) in monitors.iter().enumerate() {
            open_overlay_for_monitor(&app, format!("overlay-{i}"), i, mon.x, mon.y, mon.w, mon.h, mon.scale, false);
        }
    });
}

/// Opens the area-picker overlay on the monitor under the cursor; the
/// dragged rectangle starts an area recording — consumed by
/// `commands::overlay_cmd::area_selected`.
#[cfg(windows)]
fn trigger_record_area(app: &AppHandle) {
    if is_any_overlay_open(app) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let (cx, cy) = capture::cursor_position();
        let shot = match tauri::async_runtime::spawn_blocking(move || capture::capture_monitor_at(cx, cy)).await {
            Ok(Ok(shot)) => shot,
            Ok(Err(e)) => { crate::notify_error(&app, &format!("Screen capture failed: {e}")); return; }
            Err(e) => { crate::notify_error(&app, &e.to_string()); return; }
        };
        let jpeg = encode_overlay_jpeg(&shot.image).map(|b64| format!("data:image/jpeg;base64,{b64}"));
        let (mx, my, mw, mh, scale) = (shot.x, shot.y, shot.width, shot.height, shot.scale);

        *app.state::<Pending>().0.lock().unwrap() = Some(PendingCapture {
            image_jpeg: jpeg,
            scale,
            mode: OverlayMode::RecordArea,
            windows: Vec::new(),
            mon_x: mx,
            mon_y: my,
            mon_w: mw,
            mon_h: mh,
            monitors: Vec::new(),
            mon_jpegs: Vec::new(),
            live_mode: false,
        });

        open_overlay(&app, mx, my, mw, mh, scale, false);
    });
}

fn is_any_overlay_open(app: &AppHandle) -> bool {
    if app.get_webview_window("overlay").is_some() { return true; }
    for i in 0..8 {
        if app.get_webview_window(&format!("overlay-{i}")).is_some() { return true; }
    }
    false
}

pub fn open_overlay(app: &AppHandle, mx: i32, my: i32, mw: u32, mh: u32, scale: f32, _live: bool) {
    let app2 = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Some(loading) = app2.get_webview_window("loading") {
            let _ = loading.close();
        }
        let build = || -> tauri::Result<()> {
            // On macOS, monitor coordinates are already in logical points, so
            // dividing by `scale` would mis-position the overlay; on Windows
            // they're physical pixels and need the division.
            #[cfg(target_os = "macos")]
            let (lx, ly, lw, lh) = { let _ = scale; (mx as f64, my as f64, mw as f64, mh as f64) };
            #[cfg(not(target_os = "macos"))]
            let (lx, ly, lw, lh) = (mx as f64 / scale as f64, my as f64 / scale as f64, mw as f64 / scale as f64, mh as f64 / scale as f64);

            let url = "pages/overlay.html";
            #[allow(unused_variables)]
            let win = WebviewWindowBuilder::new(
                &app2,
                "overlay",
                WebviewUrl::App(url.into()),
            )
            .title("Capcove")
            .decorations(false)
            .shadow(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .resizable(true)
            .transparent(true)
            .visible(false)
            .position(lx, ly)
            .inner_size(lw, lh)
            .build()?;
            #[cfg(target_os = "macos")]
            {
                let _ = win.set_size(tauri::LogicalSize::new(lw, lh));
                let _ = win.set_position(tauri::LogicalPosition::new(lx, ly));
            }
            Ok(())
        };
        if let Err(e) = build() {
            log::warn!("overlay could not be opened: {e}");
        }
    });
}

fn open_overlay_for_monitor(app: &AppHandle, label: String, mon_index: usize, mx: i32, my: i32, mw: u32, mh: u32, scale: f32, _live: bool) {
    let app2 = app.clone();
    let url = "pages/overlay.html";
    let _ = app.run_on_main_thread(move || {
        let build = || -> tauri::Result<()> {
            #[cfg(target_os = "macos")]
            let (lx, ly, lw, lh) = { let _ = scale; (mx as f64, my as f64, mw as f64, mh as f64) };
            #[cfg(not(target_os = "macos"))]
            let (lx, ly, lw, lh) = (mx as f64 / scale as f64, my as f64 / scale as f64, mw as f64 / scale as f64, mh as f64 / scale as f64);

            #[allow(unused_variables)]
            let win = WebviewWindowBuilder::new(
                &app2,
                &label,
                WebviewUrl::App(url.into()),
            )
            .title("Capcove")
            .decorations(false)
            .shadow(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .resizable(true)
            .transparent(true)
            .visible(false)
            .position(lx, ly)
            .inner_size(lw, lh)
            .build()?;
            #[cfg(target_os = "macos")]
            {
                let _ = win.set_size(tauri::LogicalSize::new(lw, lh));
                let _ = win.set_position(tauri::LogicalPosition::new(lx, ly));
            }
            Ok(())
        };
        if let Err(e) = build() {
            log::warn!("overlay-{mon_index} could not be opened: {e}");
        }
    });
}

pub(crate) fn close_all_overlays(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("overlay") { let _ = w.close(); }
    for i in 0..8 {
        if let Some(w) = app.get_webview_window(&format!("overlay-{i}")) { let _ = w.close(); }
    }
}

/// Snapshot of what's needed to reopen the overlay window(s), taken before
/// closing them (see `reopen_overlay_live` command).
pub(crate) struct LiveReopenInfo {
    mode: OverlayMode,
    monitors: Vec<MonitorInfo>,
    mon_x: i32,
    mon_y: i32,
    mon_w: u32,
    mon_h: u32,
    scale: f32,
}

pub(crate) fn live_reopen_info(app: &AppHandle) -> Option<LiveReopenInfo> {
    let pending = app.state::<Pending>();
    let mut guard = pending.0.lock().unwrap();
    let p = guard.as_mut()?;
    p.live_mode = true;
    Some(LiveReopenInfo {
        mode: p.mode,
        monitors: p.monitors.clone(),
        mon_x: p.mon_x,
        mon_y: p.mon_y,
        mon_w: p.mon_w,
        mon_h: p.mon_h,
        scale: p.scale,
    })
}

/// Reopens the overlay window(s) already in "live" mode, so the first paint
/// is transparent instead of toggling an opaque window transparent afterward.
pub(crate) fn open_overlays_live(app: &AppHandle, info: LiveReopenInfo) {
    match info.mode {
        OverlayMode::RecordArea => open_overlay(app, info.mon_x, info.mon_y, info.mon_w, info.mon_h, info.scale, true),
        OverlayMode::RecordWindow => {
            if info.monitors.is_empty() {
                open_overlay_for_monitor(app, "overlay-0".into(), 0, info.mon_x, info.mon_y, info.mon_w, info.mon_h, info.scale, true);
            } else {
                for (i, mon) in info.monitors.iter().enumerate() {
                    open_overlay_for_monitor(app, format!("overlay-{i}"), i, mon.x, mon.y, mon.w, mon.h, mon.scale, true);
                }
            }
        }
    }
}
