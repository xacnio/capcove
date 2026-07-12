use crate::{config::{ConfigStore, ShortcutAction}, library, overlay, sync};
use std::sync::Arc;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, WebviewUrl, WebviewWindowBuilder};
use tauri::window::Color;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

/// Native macOS rounded window corners via Tauri's window-effects API (the
/// CSS-clip + `transparent(true)` approach doesn't actually mask the
/// WKWebView's own backing layer, so corners stay square without this).
#[cfg(target_os = "macos")]
pub(crate) fn mac_rounded_effects() -> tauri::utils::config::WindowEffectsConfig {
    tauri::window::EffectsBuilder::new()
        .effect(tauri::window::Effect::WindowBackground)
        .radius(10.0)
        .build()
}

/// The main screen's usable work area (excluding menu bar/Dock) converted from
/// AppKit's bottom-left-origin frames to Tauri's top-left-origin logical
/// coordinates. Returns `(x, y, width, height)`.
#[cfg(target_os = "macos")]
pub(crate) fn mac_visible_frame() -> Option<(f64, f64, f64, f64)> {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSScreen;
    let mtm = MainThreadMarker::new()?;
    let screen = NSScreen::mainScreen(mtm)?;
    let full = screen.frame();
    let vf = screen.visibleFrame();
    let x = vf.origin.x;
    let y = full.size.height - (vf.origin.y + vf.size.height);
    Some((x, y, vf.size.width, vf.size.height))
}

/// Centers a window of size `w`x`h` within the visible work area (falls back
/// to `None` so callers can use Tauri's generic `.center()` instead).
#[cfg(target_os = "macos")]
pub(crate) fn mac_centered_position(w: f64, h: f64) -> Option<(f64, f64)> {
    let (vx, vy, vw, vh) = mac_visible_frame()?;
    let cw = w.min(vw);
    let ch = h.min(vh);
    Some((vx + (vw - cw) / 2.0, vy + (vh - ch) / 2.0))
}

pub fn register_hotkeys(app: &AppHandle) {
    let gs = app.global_shortcut();
    let _ = gs.unregister_all();
    let settings = app.state::<Arc<ConfigStore>>().get();
    if !settings.hotkeys_enabled {
        return;
    }
    for slot in &settings.shortcuts {
        if slot.combo.trim().is_empty() {
            continue;
        }
        let capture = slot.capture.clone();
        let actions = slot.actions.clone();
        let multi_monitor = slot.multi_monitor;
        let result = gs.on_shortcut(slot.combo.as_str(), move |app, _shortcut, event| {
            if event.state() == ShortcutState::Pressed {
                if actions.contains(&ShortcutAction::OpenWheel) {
                    crate::wheel::open(app);
                    return;
                }
                #[cfg(windows)]
                {
                    // Global-shortcut callbacks fire on the main thread, and these do
                    // blocking native work, so they must run via spawn_blocking.
                    if actions.contains(&ShortcutAction::SaveReplay) {
                        let app = app.clone();
                        tauri::async_runtime::spawn_blocking(move || {
                            if let Err(e) = crate::recording::replay_buffer::save_replay(&app) {
                                crate::notify_error(&app, &e);
                            }
                        });
                        return;
                    }
                    if app.state::<Arc<crate::recording::RecordingManager>>().is_recording() {
                        let app = app.clone();
                        tauri::async_runtime::spawn_blocking(move || {
                            if let Err(e) = crate::recording::stop_recording(&app) {
                                log::warn!("failed to stop recording: {e}");
                            }
                        });
                        return;
                    }
                }
                overlay::trigger(app, capture.clone(), actions.clone(), multi_monitor);
            }
        });
        if let Err(e) = result {
            log::warn!("failed to register shortcut ({}): {e}", slot.combo);
        }
    }
}

#[derive(serde::Deserialize)]
struct TrayLocale {
    open_gallery:               String,
    open_folder:                String,
    sync_now:                   String,
    settings:                   String,
    shortcuts_enabled:          String,
    quit:                       String,
    capture_record_window:      String,
    capture_record_area:        String,
    capture_record_monitor:     String,
    action_save_replay:         String,
    action_open_wheel:          String,
    recording_status:          String,
}

/// Single source of truth for tray menu strings, shared across languages —
/// keeps translation in one file instead of a Rust match arm per language.
static TRAY_LOCALES: std::sync::LazyLock<std::collections::HashMap<String, TrayLocale>> =
    std::sync::LazyLock::new(|| {
        serde_json::from_str(include_str!("../locales/tray-locale.json"))
            .expect("tray-locale.json must be valid")
    });

fn tray_locale(lang: &str) -> &'static TrayLocale {
    TRAY_LOCALES.get(lang).unwrap_or_else(|| &TRAY_LOCALES["en"])
}

fn slot_default_label(slot: &crate::config::ShortcutSlot, loc: &TrayLocale) -> String {
    use crate::config::{ShortcutAction, ShortcutCapture};
    if slot.actions.contains(&ShortcutAction::SaveReplay) {
        return loc.action_save_replay.clone();
    }
    if slot.actions.contains(&ShortcutAction::OpenWheel) {
        return loc.action_open_wheel.clone();
    }
    match slot.capture {
        ShortcutCapture::RecordWindow => loc.capture_record_window.clone(),
        ShortcutCapture::RecordArea => loc.capture_record_area.clone(),
        ShortcutCapture::RecordMonitor => loc.capture_record_monitor.clone(),
    }
}

fn menu_item(app: &AppHandle, id: &str, text: &str, accelerator: &str) -> tauri::Result<MenuItem<tauri::Wry>> {
    if !accelerator.trim().is_empty() {
        if let Ok(item) = MenuItem::with_id(app, id, text, true, Some(accelerator)) {
            return Ok(item);
        }
    }
    MenuItem::with_id(app, id, text, true, None::<&str>)
}

pub fn build_tray_menu(app: &AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let s = app.state::<Arc<ConfigStore>>().get();
    let loc = tray_locale(&s.language);

    // Disabled (non-clickable) status row shown only while a recording is
    // active, so opening the tray menu answers "is this actually recording"
    // without needing a separate always-visible indicator window.
    #[cfg(windows)]
    let recording_status_item = {
        let manager = app.state::<Arc<crate::recording::RecordingManager>>();
        match manager.current_session() {
            Some(rs) => {
                let elapsed = (chrono::Utc::now().timestamp() - rs.started_at).max(0);
                let label = format!("● {} · {:02}:{:02}", loc.recording_status, elapsed / 60, elapsed % 60);
                Some(MenuItem::with_id(app, "recording_status", &label, false, None::<&str>)?)
            }
            None => None,
        }
    };

    let slot_items: Vec<MenuItem<tauri::Wry>> = s
        .shortcuts
        .iter()
        .filter(|slot| slot.show_in_menu)
        .map(|slot| {
            let label = if slot.label.trim().is_empty() {
                slot_default_label(slot, loc)
            } else {
                slot.label.clone()
            };
            menu_item(app, &slot.id, &label, &slot.combo)
        })
        .collect::<tauri::Result<_>>()?;

    // Build the full menu: recording status + slot items + separator + static items
    let mut items: Vec<&dyn tauri::menu::IsMenuItem<tauri::Wry>> = Vec::new();
    #[cfg(windows)]
    let sep_recording = PredefinedMenuItem::separator(app)?;
    #[cfg(windows)]
    if let Some(item) = &recording_status_item {
        items.push(item);
        items.push(&sep_recording);
    }
    for item in &slot_items {
        items.push(item);
    }
    let sep1 = PredefinedMenuItem::separator(app)?;
    let gallery = menu_item(app, "gallery",  &loc.open_gallery, "")?;
    let sep_group = PredefinedMenuItem::separator(app)?;
    let folder  = menu_item(app, "folder",   &loc.open_folder,  "")?;
    let sync    = menu_item(app, "sync_now", &loc.sync_now,     "")?;
    let setts   = menu_item(app, "settings", &loc.settings,     "")?;
    let sep2    = PredefinedMenuItem::separator(app)?;
    let hotkeys = CheckMenuItem::with_id(app, "hotkeys_toggle", &loc.shortcuts_enabled, true, s.hotkeys_enabled, None::<&str>)?;
    let sep3    = PredefinedMenuItem::separator(app)?;
    let quit    = menu_item(app, "quit", &loc.quit, "")?;

    if !slot_items.is_empty() {
        items.push(&sep1);
    }
    items.push(&folder);
    items.push(&sync);
    items.push(&sep2);
    items.push(&hotkeys);
    items.push(&sep3);
    items.push(&gallery);
    items.push(&sep_group);
    items.push(&setts);
    items.push(&quit);

    Menu::with_items(app, &items)
}

/// Native tray icon/menu APIs need to run on the main thread on Windows, but
/// callers include `recording::stop_recording`, which runs on a worker thread —
/// dispatching here unconditionally gives every caller main-thread safety for free.
pub fn refresh_tray_menu(app: &AppHandle) {
    let app2 = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Some(tray) = app2.tray_by_id("main") {
            if let Ok(menu) = build_tray_menu(&app2) {
                let _ = tray.set_menu(Some(menu));
            }
        }
    });
}

/// Draws a small filled dot fully inside the icon's bounds (inset from the
/// corner, not clipped by it), ringed with a white outline so it stays
/// visible even over an icon region close to its own color — used to badge
/// both the tray icon and the gallery's taskbar icon for recording/replay-buffer state.
#[cfg(windows)]
fn draw_corner_dot(img: &mut image::RgbaImage, w: u32, h: u32, right: bool, color: [u8; 4]) {
    let radius = (w.min(h) as f32 * 0.22).max(3.0);
    let outline = radius + 1.3;
    let cx = if right { w as f32 - radius - 1.0 } else { radius + 1.0 };
    let cy = h as f32 - radius - 1.0;
    let (r2, outline_r2) = (radius * radius, outline * outline);
    for y in 0..h {
        for x in 0..w {
            let (dx, dy) = (x as f32 - cx, y as f32 - cy);
            let dist2 = dx * dx + dy * dy;
            if dist2 <= r2 {
                img.put_pixel(x, y, image::Rgba(color));
            } else if dist2 <= outline_r2 {
                img.put_pixel(x, y, image::Rgba([255, 255, 255, 255]));
            }
        }
    }
}

/// Composites recording (red, bottom-right) and/or replay-buffer (green,
/// bottom-left) badges onto the app's default icon — built fresh each time
/// either state changes, since both can be active at once. Green (not the
/// logo's own cyan) so the buffer badge doesn't blend into the icon.
#[cfg(windows)]
fn build_badge_icon(app: &AppHandle, recording: bool, buffering: bool) -> Option<tauri::image::Image<'static>> {
    let base = app.default_window_icon()?;
    let (w, h) = (base.width(), base.height());
    let mut img = image::RgbaImage::from_raw(w, h, base.rgba().to_vec())?;
    if buffering {
        draw_corner_dot(&mut img, w, h, false, [34, 197, 94, 255]); // green-500
    }
    if recording {
        draw_corner_dot(&mut img, w, h, true, [239, 68, 68, 255]); // red-500
    }
    Some(tauri::image::Image::new(img.as_raw(), w, h).to_owned())
}

/// Applies the combined recording/buffering badge to both the tray icon and
/// the gallery window's taskbar icon, so either surface reflects state at a
/// glance without opening a menu or bringing the window forward.
#[cfg(windows)]
fn apply_state_icon(app: &AppHandle, recording: bool, buffering: bool) {
    let app2 = app.clone();
    let _ = app.run_on_main_thread(move || {
        let icon = if recording || buffering {
            build_badge_icon(&app2, recording, buffering)
        } else {
            app2.default_window_icon().cloned()
        };
        let Some(icon) = icon else { return };
        if let Some(tray) = app2.tray_by_id("main") {
            let _ = tray.set_icon(Some(icon.clone()));
        }
        if let Some(win) = app2.get_webview_window("main") {
            let _ = win.set_icon(icon);
        }
    });
}

#[cfg(windows)]
pub fn set_tray_recording(app: &AppHandle, recording: bool) {
    let buffering = app.state::<Arc<crate::recording::replay_buffer::ReplayBufferManager>>().is_running();
    apply_state_icon(app, recording, buffering);
}

#[cfg(windows)]
pub fn set_tray_buffering(app: &AppHandle, buffering: bool) {
    let recording = app.state::<Arc<crate::recording::RecordingManager>>().is_recording();
    apply_state_icon(app, recording, buffering);
}

/// Re-applies the current recording/buffering badge to a just-(re)opened
/// gallery window — it's a fresh `WebviewWindowBuilder` with the plain default
/// icon, so it otherwise wouldn't pick up a badge that was set while it was closed.
#[cfg(windows)]
pub fn sync_main_window_icon(app: &AppHandle) {
    let recording = app.state::<Arc<crate::recording::RecordingManager>>().is_recording();
    let buffering = app.state::<Arc<crate::recording::replay_buffer::ReplayBufferManager>>().is_running();
    if recording || buffering {
        apply_state_icon(app, recording, buffering);
    }
}

/// Settings lives inside the gallery window as an embedded view, so "opening
/// settings" means showing the gallery and telling its frontend to switch views.
pub fn show_settings(app: &AppHandle) {
    show_main(app);
    let _ = app.emit("navigate-settings", ());
}

/// Overlay variant of `show_settings` — see `show_main_overlay`.
pub fn show_settings_overlay(app: &AppHandle) {
    show_main_overlay(app);
    let _ = app.emit("navigate-settings", ());
}

pub fn show_main(app: &AppHandle) {
    show_main_impl(app, false);
}

/// Same gallery/settings window, but borderless, always-on-top, and centered
/// on the cursor's monitor, for summoning over a running game. Doesn't help
/// with exclusive-fullscreen games, which Windows minimizes on focus loss.
pub fn show_main_overlay(app: &AppHandle) {
    show_main_impl(app, true);
}

fn show_main_impl(app: &AppHandle, overlay: bool) {
    let _ = app.emit("gallery-opened", ());
    if let Some(win) = app.get_webview_window("main") {
        // The window is a singleton — if it was already open in the other
        // mode, flip its taskbar/topmost flags to match this call instead of
        // leaving it stuck in whichever mode it was first created in.
        let _ = win.set_always_on_top(overlay);
        let _ = win.set_skip_taskbar(overlay);
        let _ = win.show();
        let _ = win.unminimize();
        let _ = win.set_focus();
    } else {
        let app_inner = app.clone();
        let _ = app.run_on_main_thread(move || {
            // Compute a safe initial size from the screen's usable work area so
            // Windows never auto-maximizes and macOS never sizes/centers into the Dock.
            #[cfg(target_os = "macos")]
            let usable = mac_visible_frame().map(|(_, _, w, h)| (w, h));
            #[cfg(not(target_os = "macos"))]
            let usable = app_inner.primary_monitor().ok().flatten().map(|m| {
                let sf = m.scale_factor();
                let ps = m.size();
                // Convert physical → logical, then subtract ~48 logical px for taskbar
                (ps.width as f64 / sf, ps.height as f64 / sf - 48.0)
            });
            let (win_w, win_h, min_w, min_h) = usable
                .map(|(lw, lh)| {
                    // Shrink the minimum constraints if the usable area is smaller,
                    // to avoid a tao/winit panic (min size must be <= available size).
                    let min_w = lw.min(1150.0);
                    let min_h = lh.min(720.0);

                    let w = (lw * 0.72).clamp(min_w, lw.min(1200.0));
                    let h = (lh * 0.84).clamp(min_h, lh.min(900.0));
                    (w, h, min_w, min_h)
                })
                .unwrap_or((1150.0, 720.0, 1150.0, 720.0));

            let mut gallery_builder = WebviewWindowBuilder::new(
                &app_inner, "main", WebviewUrl::App("pages/gallery.html".into()),
            )
            .title("Capcove")
            .inner_size(win_w, win_h)
            .min_inner_size(min_w, min_h)
            .resizable(true)
            .decorations(false)
            .transparent(cfg!(target_os = "macos"))
            .visible(false);
            if overlay {
                gallery_builder = gallery_builder.always_on_top(true).skip_taskbar(true);
            }
            #[cfg(target_os = "macos")]
            {
                // Center within the visible work area, not Tauri's generic
                // .center() (which uses the full screen and can leave the
                // window's bottom edge behind the Dock).
                gallery_builder = match mac_centered_position(win_w, win_h) {
                    Some((x, y)) => gallery_builder.position(x, y),
                    None => gallery_builder.center(),
                };
                gallery_builder = gallery_builder.effects(mac_rounded_effects());
            }
            #[cfg(not(target_os = "macos"))]
            {
                // Non-overlay opens center normally; overlay opens are
                // repositioned post-build onto the cursor's monitor below.
                if !overlay {
                    gallery_builder = gallery_builder.center();
                }
                gallery_builder = gallery_builder.background_color(Color(15, 15, 15, 255));
            }
            let win = match gallery_builder.build() {
                Ok(w) => w,
                Err(e) => {
                    log::warn!("failed to open gallery window: {e}");
                    return;
                }
            };
            // Visibility itself is deferred to the frontend's `main_ready`
            // call (avoids a flash of unstyled content) for both modes —
            // only positioning/ghost-styling need to happen here, pre-show.
            if overlay {
                #[cfg(not(target_os = "macos"))]
                {
                    // Center on the monitor under the cursor (presumably the
                    // game's), since `.center()` isn't guaranteed to pick that one.
                    let cursor_monitor = app_inner.cursor_position().ok()
                        .and_then(|p| app_inner.monitor_from_point(p.x, p.y).ok().flatten());
                    match cursor_monitor {
                        Some(m) => {
                            let sf = m.scale_factor();
                            let phys_w = (win_w * sf) as i32;
                            let phys_h = (win_h * sf) as i32;
                            let msz = m.size();
                            let x = m.position().x + (msz.width as i32 - phys_w) / 2;
                            let y = m.position().y + (msz.height as i32 - phys_h) / 2;
                            let _ = win.set_position(PhysicalPosition::new(x, y));
                        }
                        None => {
                            let _ = win.center();
                        }
                    }
                }
                if let Ok(raw) = win.hwnd() {
                    let hwnd_u32 = raw.0 as usize as u32;
                    crate::win_util::make_overlay_ghost(hwnd_u32);
                    let hide = app_inner.state::<Arc<ConfigStore>>().get().hide_overlays_from_capture;
                    crate::win_util::set_capture_hidden(hwnd_u32, hide);
                }
            }
            #[cfg(windows)]
            sync_main_window_icon(&app_inner);
        });
    }
}

pub fn open_folder(app: &AppHandle) {
    let dir = app.state::<Arc<ConfigStore>>().get().resolved_recordings_dir();
    let _ = std::fs::create_dir_all(&dir);
    use tauri_plugin_opener::OpenerExt;
    let _ = app.opener().open_path(dir.to_string_lossy().to_string(), None::<&str>);
}

pub fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let menu = build_tray_menu(app)?;
    TrayIconBuilder::with_id("main")
        .icon(app.default_window_icon().unwrap().clone())
        .tooltip("Capcove")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| {
            let app = tray.app_handle();
            match event {
                TrayIconEvent::DoubleClick { button: MouseButton::Left, .. } => {
                    show_main(app);
                }
                _ => {}
            }
        })
        .on_menu_event(|app, event| {
            use tauri::Emitter;
            let id = event.id().as_ref();
            match id {
                "gallery"        => show_main(app),
                "folder"         => open_folder(app),
                "sync_now"       => sync::scan_and_enqueue(app),
                "settings"       => show_settings(app),
                "hotkeys_toggle" => {
                    let config = app.state::<Arc<ConfigStore>>();
                    let mut s = config.get();
                    s.hotkeys_enabled = !s.hotkeys_enabled;
                    let _ = config.save(s);
                    register_hotkeys(app);
                    refresh_tray_menu(app);
                    let _ = app.emit("settings-changed", ());
                }
                "quit" => app.exit(0),
                slot_id => {
                    let settings = app.state::<Arc<ConfigStore>>().get();
                    if let Some(slot) = settings.shortcuts.iter().find(|s| s.id == slot_id) {
                        #[cfg(windows)]
                        {
                            if slot.actions.contains(&ShortcutAction::SaveReplay) {
                                let app = app.clone();
                                tauri::async_runtime::spawn_blocking(move || {
                                    if let Err(e) = crate::recording::replay_buffer::save_replay(&app) {
                                        crate::notify_error(&app, &e);
                                    }
                                });
                                return;
                            }
                            if app.state::<Arc<crate::recording::RecordingManager>>().is_recording() {
                                let app = app.clone();
                                tauri::async_runtime::spawn_blocking(move || {
                                    if let Err(e) = crate::recording::stop_recording(&app) {
                                        log::warn!("failed to stop recording: {e}");
                                    }
                                });
                                return;
                            }
                        }
                        overlay::trigger(app, slot.capture.clone(), slot.actions.clone(), slot.multi_monitor);
                    }
                }
            }
        })
        .build(app)?;
    Ok(())
}

pub fn on_library_folder_change(app: &AppHandle) {
    let drive = app.state::<Arc<crate::drive::DriveClient>>();
    drive.clear_folder_id();
    drive.clear_cache();
    app.state::<library::LibraryCache>().clear();
    app.state::<Arc<sync::SyncState>>().clear();
    let _ = app.emit("library-changed", ());
    sync::scan_and_enqueue(app);
    let sync_app = app.clone();
    tauri::async_runtime::spawn(async move {
        let _ = sync::sync_metadata_and_icons(&sync_app).await;
    });
}
