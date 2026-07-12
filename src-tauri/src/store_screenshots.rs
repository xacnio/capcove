//! Microsoft Store screenshot automation — dev builds only.
//!
//! Triggered by `--store-screenshots`: drives the real gallery window
//! (Folders view, Gallery view, Settings) and the video player's quick trim
//! tool through a fixed sequence of scenes in English then Turkish, saving
//! 1366x768 PNGs under `<repo>/store-screenshots/{lang}/`.
//!
//! Uses an isolated config dir under the OS temp folder, synthetic
//! ffmpeg-generated demo clips (never a screen capture of anything real),
//! and the app's own embedded game-icon catalog for cover art — never the
//! user's real library, settings, or Google account.

use crate::config::{ConfigStore, GameDetectMode, RecordingFolder, Settings};
use crate::meta::{MetaStore, VideoMeta};
use crate::tag::{Tag, TagStore};
use anyhow::{Context, Result};
use chrono::{Duration as ChronoDuration, Local};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Listener, LogicalPosition, LogicalSize, Manager, Position, Size, WebviewWindow};
use tauri_plugin_shell::ShellExt;

const TARGET_W: u32 = 1366;
const TARGET_H: u32 = 768;
const WIN_X: f64 = 40.0;
const WIN_Y: f64 = 40.0;

/// The `--store-screenshots` CLI flag is the "proper" trigger, but getting it
/// through `npm run tauri dev` unmangled means threading it past PowerShell's
/// own `--` handling, then npm's, then tauri-cli's `[runnerArgs] -- [appArgs]`
/// split, then finally cargo's own `run -- <args>` — any one of which can eat
/// or reposition it. The env var sidesteps all of that: it's inherited by
/// every process in the chain with no parsing involved.
pub fn requested() -> bool {
    std::env::args().any(|a| a == "--store-screenshots")
        || std::env::var("CAPCOVE_STORE_SCREENSHOTS").as_deref() == Ok("1")
}

fn root_dir() -> PathBuf {
    std::env::temp_dir().join("capcove-store-screenshots")
}

pub fn temp_config_dir() -> PathBuf {
    root_dir().join("config")
}

fn library_dir() -> PathBuf {
    root_dir().join("library")
}

fn output_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("store-screenshots")
}

/// Writes a settings.json with sane defaults before `ConfigStore::load`
/// reads it, so startup skips onboarding, sync, hotkeys, and auto-recording.
pub fn prepare_temp_config(config_dir: &Path) {
    let _ = std::fs::remove_dir_all(config_dir);
    let _ = std::fs::create_dir_all(config_dir);
    let mut settings = Settings::default();
    settings.onboarded = true;
    settings.start_with_gallery = false;
    settings.sync_enabled = false;
    settings.auto_update = false;
    settings.hotkeys_enabled = false;
    settings.video.recordings_dir = library_dir().to_string_lossy().into_owned();
    // Never let a "detected game" (however unlikely on a screenshot rig)
    // kick off a real recording mid-automation.
    settings.video.replay_buffer.enabled = false;
    settings.video.replay_buffer.game_detect_mode = GameDetectMode::Off;
    // Every overlay window this automation captures (recorder bar/frame,
    // wheel) would otherwise be excluded from any screen capture, including
    // our own — the default is on precisely so real recordings never show it.
    settings.hide_overlays_from_capture = false;
    if let Ok(json) = serde_json::to_string_pretty(&settings) {
        let _ = std::fs::write(config_dir.join("settings.json"), json);
    }
}

/// Kicks off the full automation as a background task and exits the
/// process once it's done (success or failure).
pub fn run(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        if let Err(e) = run_inner(&app).await {
            log::error!("store screenshots automation failed: {e}");
        }
        app.exit(0);
    });
}

async fn run_inner(app: &AppHandle) -> Result<()> {
    let out_root = output_dir();

    // Opened and confirmed ready exactly once — the window persists across
    // every scene in both language passes, never rebuilt in between.
    crate::tray::show_main(app);
    let window = wait_for_window(app, "main").await?;
    position_window(&window)?;
    wait_for_frontend_ready(app).await;

    for lang in ["en", "tr"] {
        let lang_dir = out_root.join(lang);
        std::fs::create_dir_all(&lang_dir)?;
        switch_language(app, lang)?;
        seed_demo_tags(app, lang);
        seed_demo_folders(app);
        // The Folders view's tiles/breadcrumb read `settings.recording_folders`
        // from the frontend's own (event-driven) copy, not a fresh fetch per
        // navigation — without this, it never learns about the folders just
        // configured above.
        let _ = app.emit("settings-changed", ());
        seed_demo_library(app, &library_dir()).await?;
        // Likewise, the video grid only ever (re)fetches `list_videos` on
        // mount or on one of these events — it mounts once with whatever's on
        // disk at that moment (often nothing, this early) and is never told
        // to look again otherwise, however many demo clips get seeded after.
        let _ = app.emit("library-changed", ());

        capture_folders_root(app, &lang_dir).await?;
        capture_folders_game(app, &lang_dir).await?;
        capture_folders_subfolder(app, &lang_dir).await?;
        capture_gallery_grid(app, &lang_dir).await?;
        capture_gallery_list(app, &lang_dir).await?;
        capture_recorder_area(app, &lang_dir).await?;
        capture_wheel_replay(app, &lang_dir).await?;
        capture_settings(app, &lang_dir, "07-settings-shortcuts.png", json!({"action":"goto-settings","page":"shortcuts"})).await?;
        capture_settings_record_quality(app, &lang_dir).await?;
        capture_settings(app, &lang_dir, "09-settings-audio.png", json!({"action":"goto-settings","page":"audio"})).await?;
        capture_settings(app, &lang_dir, "10-settings-games.png", json!({"action":"goto-settings","page":"games"})).await?;
        capture_settings_drive(app, &lang_dir, lang).await?;
        capture_settings_storage(app, &lang_dir).await?;
        capture_settings(app, &lang_dir, "13-settings-general.png", json!({"action":"goto-settings","page":"general"})).await?;
        capture_settings(app, &lang_dir, "16-settings-youtube.png", json!({"action":"goto-settings","page":"youtube"})).await?;
        capture_trim_tool(app, &lang_dir).await?;

        // Website marketing images: a fixed subset of the English-pass scenes,
        // re-encoded to the exact filenames/format `website/src` expects. The
        // site's screenshots aren't localized (see `Hero.jsx`/`Features.jsx`),
        // so only the "en" pass feeds them.
        if lang == "en" {
            export_website_screenshots(app, &lang_dir).await?;
        }
    }
    Ok(())
}

/// Re-encodes a fixed set of already-captured PNGs to `.webp` under
/// `website/public/screenshots/`, with the exact filenames the site's
/// components reference — via the ffmpeg sidecar (the `image` crate has no
/// webp *encoder* enabled), so no new dependency is needed.
async fn export_website_screenshots(app: &AppHandle, lang_dir: &Path) -> Result<()> {
    let website_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("website").join("public").join("screenshots");
    let pairs = [
        ("04-gallery-grid.png", "gallery-recordings.webp", None),
        ("14-editor.png", "editor-timeline.webp", None),
        ("08-settings-record.png", "recording-settings.webp", None),
        ("15-wheel-replay.png", "replay.webp", None),
        ("16-settings-youtube.png", "youtube-live.webp", None),
        // Cropped tighter than the full 1366x768 window grab — these two
        // frame a specific detail (the dashed selection + control bar; the
        // Drive panel past its settings sidebar) rather than the whole scene.
        ("06-recorder-area.png", "area-recording.webp", Some("crop=1020:610:320:135")),
        ("11-settings-drive.png", "drive-sync.webp", Some("crop=1066:768:300:0")),
    ];
    for (src_name, dest_name, crop) in pairs {
        convert_to_webp(app, &lang_dir.join(src_name), &website_dir.join(dest_name), crop).await?;
    }
    Ok(())
}

async fn convert_to_webp(app: &AppHandle, src: &Path, dest: &Path, crop_filter: Option<&str>) -> Result<()> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let cmd = app.shell().sidecar("ffmpeg").context("ffmpeg sidecar not found")?;
    let mut args = vec!["-y".to_string(), "-i".to_string(), src.to_string_lossy().into_owned()];
    if let Some(f) = crop_filter {
        args.push("-vf".to_string());
        args.push(f.to_string());
    }
    args.extend(["-c:v".to_string(), "libwebp".to_string(), "-lossless".to_string(), "0".to_string(), "-quality".to_string(), "90".to_string(), dest.to_string_lossy().into_owned()]);
    let output = cmd.args(args).output().await.context("failed to run ffmpeg webp conversion")?;
    if !output.status.success() {
        anyhow::bail!("ffmpeg webp conversion of {} failed: {}", src.display(), String::from_utf8_lossy(&output.stderr));
    }
    Ok(())
}

fn switch_language(app: &AppHandle, lang: &str) -> Result<()> {
    let store = app.state::<Arc<ConfigStore>>();
    let mut settings = store.get();
    settings.language = lang.into();
    store.save(settings)?;
    let _ = app.emit("settings-changed", ());
    Ok(())
}

// ---------------------------------------------------------------------------
// Window helpers
// ---------------------------------------------------------------------------

async fn wait_for_window(app: &AppHandle, label: &str) -> Result<WebviewWindow> {
    for _ in 0..200 {
        if let Some(w) = app.get_webview_window(label) {
            return Ok(w);
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    anyhow::bail!("window `{label}` did not appear in time")
}

fn position_window(window: &WebviewWindow) -> Result<()> {
    let _ = window.set_size(Size::Logical(LogicalSize::new(TARGET_W as f64, TARGET_H as f64)));
    let _ = window.set_position(Position::Logical(LogicalPosition::new(WIN_X, WIN_Y)));
    let _ = window.show();
    let _ = window.unminimize();
    let _ = window.set_focus();
    Ok(())
}

/// Sends a `store-screenshot-cmd` and waits (with a timeout) for the
/// frontend's ack. Uses `.listen()` not `.once()`: tauri's `.once()` panics
/// if a second delivery races in (e.g. React double-effects), so this
/// guards idempotency itself by taking the oneshot sender at most once.
async fn send_cmd_and_wait(app: &AppHandle, payload: serde_json::Value) {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let tx = std::sync::Mutex::new(Some(tx));
    let handler_id = app.listen("store-screenshot-ready", move |_event| {
        if let Some(tx) = tx.lock().unwrap().take() {
            let _ = tx.send(());
        }
    });
    let _ = app.emit("store-screenshot-cmd", payload);
    let _ = tokio::time::timeout(Duration::from_secs(2), rx).await;
    app.unlisten(handler_id);
}

/// Blocks until the frontend's own `store-screenshot-cmd` listener is
/// actually registered (see App.jsx's matching `emit`) — call exactly once,
/// right after the window first appears. Under `npm run tauri dev`, Vite's
/// on-demand cold compile can push the first real mount out several
/// seconds; any `store-screenshot-cmd` emitted before that listener exists
/// is just lost, not queued, so this has to be a real wait, not a guess.
async fn wait_for_frontend_ready(app: &AppHandle) {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let tx = std::sync::Mutex::new(Some(tx));
    let handler_id = app.listen("store-screenshot-frontend-ready", move |_event| {
        if let Some(tx) = tx.lock().unwrap().take() {
            let _ = tx.send(());
        }
    });
    if tokio::time::timeout(Duration::from_secs(60), rx).await.is_err() {
        log::warn!("store screenshots: never heard from the frontend after 60s — proceeding anyway");
    }
    app.unlisten(handler_id);
}

/// Direct `PrintWindow` capture of our own window — `capture_window_thumbnail`
/// (built on `xcap`) can't be reused here: xcap's own Windows enumeration
/// hardcodes skipping any window owned by the calling process (deliberately,
/// to dodge a `GetWindowText` deadlock risk — see xcap's `is_valid_window`),
/// so it can never even find this window to capture it, no matter how long
/// this waits/retries. This bypasses xcap entirely and talks to GDI directly.
fn capture_native(window: &WebviewWindow) -> Result<image::RgbaImage> {
    use windows::Win32::Foundation::{HWND, RECT};
    use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS};
    use windows::Win32::Graphics::Gdi::{
        CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetDIBits, ReleaseDC,
        SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HGDIOBJ,
    };
    use windows::Win32::Storage::Xps::{PrintWindow, PRINT_WINDOW_FLAGS};
    use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;

    let hwnd_id = window.hwnd().ok().map(|h| h.0 as usize as u32).context("no hwnd for window")?;
    let hwnd = HWND(hwnd_id as usize as *mut _);

    unsafe {
        // PrintWindow always renders relative to the raw GetWindowRect bounds
        // (any invisible resize-border/shadow margin included) — the capture
        // canvas must be sized to that, not to a DWM-trimmed rect, or the
        // rendered content ends up shifted inside an undersized bitmap.
        let mut raw = RECT::default();
        GetWindowRect(hwnd, &mut raw).context("GetWindowRect failed")?;
        let w = (raw.right - raw.left).max(1);
        let h = (raw.bottom - raw.top).max(1);

        let hdc_screen = GetDC(None);
        let hdc_mem = CreateCompatibleDC(hdc_screen);
        let hbm = CreateCompatibleBitmap(hdc_screen, w, h);
        let old = SelectObject(hdc_mem, HGDIOBJ(hbm.0));

        // PW_RENDERFULLCONTENT (2): needed to capture GPU-accelerated
        // (WebView2/DirectComposition) content, not just the GDI-drawn parts.
        let ok = PrintWindow(hwnd, hdc_mem, PRINT_WINDOW_FLAGS(2));

        let mut bi: BITMAPINFO = std::mem::zeroed();
        bi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bi.bmiHeader.biWidth = w;
        bi.bmiHeader.biHeight = -h; // negative = top-down row order
        bi.bmiHeader.biPlanes = 1;
        bi.bmiHeader.biBitCount = 32;
        bi.bmiHeader.biCompression = BI_RGB.0;

        let mut pixels = vec![0u8; (w * h * 4) as usize];
        let rows = GetDIBits(hdc_mem, hbm, 0, h as u32, Some(pixels.as_mut_ptr().cast()), &mut bi, DIB_RGB_COLORS);

        SelectObject(hdc_mem, old);
        let _ = DeleteObject(HGDIOBJ(hbm.0));
        let _ = DeleteDC(hdc_mem);
        ReleaseDC(None, hdc_screen);

        if !ok.as_bool() || rows == 0 {
            anyhow::bail!("PrintWindow failed (ok={}, rows={rows})", ok.as_bool());
        }

        // GDI hands back BGRA; `image::RgbaImage` wants RGBA.
        for chunk in pixels.chunks_exact_mut(4) {
            chunk.swap(0, 2);
            chunk[3] = 255;
        }
        let full_img = image::RgbaImage::from_raw(w as u32, h as u32, pixels)
            .context("failed to build RgbaImage from captured pixels")?;

        // Crop off the invisible shadow/border margin `GetWindowRect` (but not
        // DWM's extended-frame-bounds) includes, same trim the recorder's own
        // frame-tracking uses (see `capture::window_frame_rect`).
        let mut frame = RECT::default();
        let _ = DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            std::ptr::addr_of_mut!(frame).cast(),
            std::mem::size_of::<RECT>() as u32,
        );
        if frame.right > frame.left && frame.bottom > frame.top {
            let ox = (frame.left - raw.left).max(0) as u32;
            let oy = (frame.top - raw.top).max(0) as u32;
            let cw = (frame.right - frame.left).max(1) as u32;
            let ch = (frame.bottom - frame.top).max(1) as u32;
            let ox = ox.min(full_img.width().saturating_sub(1));
            let oy = oy.min(full_img.height().saturating_sub(1));
            let cw = cw.min(full_img.width() - ox).max(1);
            let ch = ch.min(full_img.height() - oy).max(1);
            Ok(image::imageops::crop_imm(&full_img, ox, oy, cw, ch).to_image())
        } else {
            Ok(full_img)
        }
    }
}

fn save_resized(img: &image::RgbaImage, out_path: &Path) -> Result<()> {
    let resized = if img.width() == TARGET_W && img.height() == TARGET_H {
        img.clone()
    } else {
        image::imageops::resize(img, TARGET_W, TARGET_H, image::imageops::FilterType::Lanczos3)
    };
    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    resized.save(out_path)?;
    Ok(())
}

/// Positions/sizes the (persistent — never rebuilt between scenes) main
/// window, gives it a beat to settle, captures, and saves.
///
/// A freshly shown/focused/resized WebView2 window doesn't necessarily have
/// a real frame painted yet the instant `show()`/`set_focus()` return.
/// `PrintWindow` can report success while the surface it captured is still
/// the pre-paint solid-black one — a GDI-level success, not a content
/// failure, so it wouldn't be caught by `capture_native`'s own `Result`.
///
/// Under `npm run tauri dev` specifically, the very first load is Vite's dev
/// server compiling and serving the page on demand — not a prebuilt bundle —
/// so the *first* scene's real first paint can legitimately take several
/// seconds, not milliseconds. Retrying with a generous budget (rather than
/// accepting/failing on the first attempt) covers both that and the
/// ordinary just-shown-not-painted-yet gap every later scene still has.
async fn capture_current(app: &AppHandle, out_path: &Path) -> Result<()> {
    let window = wait_for_window(app, "main").await?;
    position_window(&window)?;
    tokio::time::sleep(Duration::from_millis(400)).await;
    let mut last_err = None;
    for attempt in 0..60 {
        match capture_native(&window) {
            Ok(img) if !crate::capture::is_blank_capture(&img) => {
                if attempt > 0 {
                    log::info!("store screenshots: capture of {} succeeded after {attempt} retries", out_path.display());
                }
                save_resized(&img, out_path)?;
                return Ok(());
            }
            Ok(_) => {
                if attempt == 0 {
                    log::warn!("store screenshots: capture of {} came back blank, retrying (dev-server cold start can take a while)", out_path.display());
                }
                last_err = Some(anyhow::anyhow!("capture stayed blank after retrying"));
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            Err(e) => {
                if attempt == 0 {
                    log::warn!("store screenshots: capture of {} failed ({e}), retrying", out_path.display());
                }
                last_err = Some(e);
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        }
    }
    Err(last_err.unwrap())
}

/// Grabs the actual composited desktop pixels at the same on-screen region
/// the main window occupies, instead of one window's own content — needed
/// for the recorder scene, where the bar + region frame are separate
/// always-on-top windows layered over the gallery, not part of it, so a
/// single-window capture could only ever show one of them.
async fn capture_desktop_region(app: &AppHandle, out_path: &Path) -> Result<()> {
    let window = wait_for_window(app, "main").await?;
    let scale = window.scale_factor().unwrap_or(1.0);
    let px = (WIN_X * scale).round() as i32;
    let py = (WIN_Y * scale).round() as i32;
    let pw = (TARGET_W as f64 * scale).round() as u32;
    let ph = (TARGET_H as f64 * scale).round() as u32;
    let mut last_err = None;
    for attempt in 0..40 {
        match crate::capture::capture_monitor_at(px, py) {
            Ok(shot) => {
                let ix = (px - shot.x).max(0) as u32;
                let iy = (py - shot.y).max(0) as u32;
                let cw = pw.min(shot.image.width().saturating_sub(ix)).max(1);
                let ch = ph.min(shot.image.height().saturating_sub(iy)).max(1);
                let cropped = image::imageops::crop_imm(&shot.image, ix, iy, cw, ch).to_image();
                if !crate::capture::is_blank_capture(&cropped) {
                    save_resized(&cropped, out_path)?;
                    return Ok(());
                }
                if attempt == 0 {
                    log::warn!("store screenshots: desktop region for {} came back blank, retrying", out_path.display());
                }
                last_err = Some(anyhow::anyhow!("desktop region capture stayed blank after retrying"));
            }
            Err(e) => {
                if attempt == 0 {
                    log::warn!("store screenshots: desktop region capture for {} failed ({e}), retrying", out_path.display());
                }
                last_err = Some(e);
            }
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    Err(last_err.unwrap())
}

// ---------------------------------------------------------------------------
// Scenes
// ---------------------------------------------------------------------------

async fn capture_folders_root(app: &AppHandle, lang_dir: &Path) -> Result<()> {
    // The window itself is already open and confirmed ready (see `run_inner`)
    // — just give the library scan + thumbnail generation a moment so the
    // screenshot isn't a "scanning..." placeholder.
    tokio::time::sleep(Duration::from_millis(1500)).await;
    send_cmd_and_wait(app, json!({"action":"goto-view","view":"folders"})).await;
    send_cmd_and_wait(app, json!({"action":"goto-folder-game","game": null})).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    capture_current(app, &lang_dir.join("01-folders-root.png")).await
}

async fn capture_folders_game(app: &AppHandle, lang_dir: &Path) -> Result<()> {
    send_cmd_and_wait(app, json!({"action":"goto-view","view":"folders"})).await;
    send_cmd_and_wait(app, json!({"action":"goto-folder-game","game": DEMO_GAME_WITH_FOLDERS})).await;
    // A live "clipping" badge (replay buffer running for this game) makes the
    // scene look like the app is actively working in the background, not
    // just a static browse — faked the same way the wheel's own replay scene
    // is, never a real running buffer.
    send_cmd_and_wait(
        app,
        json!({"action":"set-replay-demo","status":{"running": true, "app": DEMO_GAME_WITH_FOLDERS, "buffered_seconds": 143}}),
    )
    .await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let result = capture_current(app, &lang_dir.join("02-folders-game.png")).await;
    send_cmd_and_wait(app, json!({"action":"set-replay-demo","status": null})).await;
    result
}

/// Drilled one level deeper than `capture_folders_game` — into one of that
/// game's own folders, showing its contents rather than just its tiles.
async fn capture_folders_subfolder(app: &AppHandle, lang_dir: &Path) -> Result<()> {
    send_cmd_and_wait(app, json!({"action":"goto-view","view":"folders"})).await;
    send_cmd_and_wait(app, json!({"action":"goto-folder-game","game": DEMO_GAME_WITH_FOLDERS, "folder": DEMO_FOLDER_ID})).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    capture_current(app, &lang_dir.join("03-folders-subfolder.png")).await
}

async fn capture_gallery_grid(app: &AppHandle, lang_dir: &Path) -> Result<()> {
    send_cmd_and_wait(app, json!({"action":"goto-view","view":"gallery"})).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    capture_current(app, &lang_dir.join("04-gallery-grid.png")).await
}

/// Same flat gallery, switched to the list view mode — a different enough
/// look (compact rows, columns) to be worth its own screenshot.
async fn capture_gallery_list(app: &AppHandle, lang_dir: &Path) -> Result<()> {
    send_cmd_and_wait(app, json!({"action":"set-view-mode","mode":"list"})).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let result = capture_current(app, &lang_dir.join("05-gallery-list.png")).await;
    // Back to the default grid mode for the rest of the run.
    send_cmd_and_wait(app, json!({"action":"set-view-mode","mode":"xl"})).await;
    result
}

/// The floating recorder (control bar + dashed region frame) overlaid on the
/// gallery — these are separate always-on-top windows, not part of the main
/// window, so this needs an actual screen-region grab (`capture_desktop_region`)
/// instead of a single-window capture to show them together.
async fn capture_recorder_area(app: &AppHandle, lang_dir: &Path) -> Result<()> {
    send_cmd_and_wait(app, json!({"action":"goto-view","view":"folders"})).await;
    send_cmd_and_wait(app, json!({"action":"goto-folder-game","game": null})).await;
    tokio::time::sleep(Duration::from_millis(400)).await;
    crate::recorder::recorder_open_mode(app.clone(), "area".into()).await;
    // Let the bar + frame windows build, position, and render.
    tokio::time::sleep(Duration::from_millis(1200)).await;
    // The region frame is otherwise always excluded from capture (by design,
    // so it never shows up in a real recording) — this is the one
    // deliberate exception, since we're taking a screenshot, not recording.
    crate::recorder::reveal_for_screenshot(app);
    let result = capture_desktop_region(app, &lang_dir.join("06-recorder-area.png")).await;
    crate::recorder::recorder_close(app.clone()).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    result
}

/// The fullscreen radial wheel with the "Save Replay" wedge highlighted, a
/// running replay buffer, and a detected game — all faked via a dev-only
/// `set-wheel-demo` command (see `wheel/App.jsx`'s matching listener), never
/// an actual running buffer or capture session.
///
/// The wheel dims the *entire* monitor behind it (a fixed `rgba(0,0,0,0.72)`
/// backdrop over `100vw`/`100vh`), so whatever real window is on screen
/// would otherwise show faintly through the edges — a real-desktop leak this
/// automation must never produce. Stretching the main window to cover the
/// whole monitor first means only our own (synthetic) content is ever behind
/// that dim layer.
async fn capture_wheel_replay(app: &AppHandle, lang_dir: &Path) -> Result<()> {
    let window = wait_for_window(app, "main").await?;
    let monitor = app.primary_monitor().ok().flatten().context("no primary monitor")?;
    let mon_pos = *monitor.position();
    let mon_size = *monitor.size();
    let _ = window.set_position(Position::Physical(mon_pos));
    let _ = window.set_size(Size::Physical(mon_size));
    tokio::time::sleep(Duration::from_millis(300)).await;

    crate::wheel::open(app);
    wait_for_window(app, "wheel").await?;
    tokio::time::sleep(Duration::from_millis(600)).await;
    send_cmd_and_wait(
        app,
        json!({
            "action": "set-wheel-demo",
            "currentApp": DEMO_GAME_WITH_FOLDERS,
            "buffer": { "running": true, "buffered_seconds": 187, "app": DEMO_GAME_WITH_FOLDERS },
            "hover": "save_replay",
        }),
    )
    .await;
    tokio::time::sleep(Duration::from_millis(400)).await;

    let result = capture_full_monitor(mon_pos.x, mon_pos.y, &lang_dir.join("15-wheel-replay.png")).await;

    // Toggling `open` again closes it (see `wheel.rs`'s own show/hide toggle).
    crate::wheel::open(app);
    tokio::time::sleep(Duration::from_millis(300)).await;
    position_window(&window)?;
    result
}

/// Whole-monitor grab, no cropping — the wheel fills the entire screen
/// rather than sitting inside the main window's own bounds.
async fn capture_full_monitor(px: i32, py: i32, out_path: &Path) -> Result<()> {
    let mut last_err = None;
    for attempt in 0..40 {
        match crate::capture::capture_monitor_at(px, py) {
            Ok(shot) if !crate::capture::is_blank_capture(&shot.image) => {
                return save_resized(&shot.image, out_path);
            }
            Ok(_) => {
                if attempt == 0 {
                    log::warn!("store screenshots: wheel capture for {} came back blank, retrying", out_path.display());
                }
                last_err = Some(anyhow::anyhow!("wheel capture stayed blank after retrying"));
            }
            Err(e) => {
                if attempt == 0 {
                    log::warn!("store screenshots: wheel capture for {} failed ({e}), retrying", out_path.display());
                }
                last_err = Some(e);
            }
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    Err(last_err.unwrap())
}

async fn capture_settings(app: &AppHandle, lang_dir: &Path, filename: &str, cmd: serde_json::Value) -> Result<()> {
    send_cmd_and_wait(app, cmd).await;
    tokio::time::sleep(Duration::from_millis(400)).await;
    capture_current(app, &lang_dir.join(filename)).await
}

/// "Quality" page: encoder/container/resolution/bitrate — more useful as a
/// screenshot than the "mode" page, but its encoder dropdown enumerates
/// hardware encoders via a probe that takes several real seconds; capturing
/// on the generic 400ms beat shows it still empty/loading.
async fn capture_settings_record_quality(app: &AppHandle, lang_dir: &Path) -> Result<()> {
    send_cmd_and_wait(app, json!({"action":"goto-settings","page":"quality"})).await;
    tokio::time::sleep(Duration::from_millis(5000)).await;
    capture_current(app, &lang_dir.join("08-settings-record.png")).await
}

async fn capture_settings_drive(app: &AppHandle, lang_dir: &Path, lang: &str) -> Result<()> {
    send_cmd_and_wait(app, json!({"action":"goto-settings","page":"drive"})).await;
    send_cmd_and_wait(
        app,
        json!({
            "action":"set-drive-demo",
            "connected": true,
            "email": "capcove@xacnio.dev",
            "name": if lang == "tr" { "Demo Kullanıcı" } else { "Demo User" },
        }),
    )
    .await;
    tokio::time::sleep(Duration::from_millis(400)).await;
    capture_current(app, &lang_dir.join("11-settings-drive.png")).await
}

/// The Storage settings page's folder-location field otherwise displays the
/// automation's real (isolated, temp-dir) recordings path — ugly, and it
/// leaks the machine's Windows username via `%TEMP%`. Faked the same way the
/// Drive scene fakes a connected account, purely for display.
async fn capture_settings_storage(app: &AppHandle, lang_dir: &Path) -> Result<()> {
    send_cmd_and_wait(app, json!({"action":"goto-settings","page":"storage"})).await;
    send_cmd_and_wait(app, json!({"action":"set-storage-demo","path": r"C:\Users\Player\Videos\Capcove"})).await;
    tokio::time::sleep(Duration::from_millis(400)).await;
    capture_current(app, &lang_dir.join("12-settings-storage.png")).await
}

/// Opens a dedicated demo clip directly in the real video player (see
/// `VideoGrid.jsx`'s `"goto-player"` handling) — showing the quick trim tool
/// (drag handles, waveform, Save Clip/Advanced Export) that replaced the old
/// full editor view, now disabled (`EDITOR_ENABLED = false`) and unreachable
/// from the UI. Kept saving to the same filename the website's screenshot
/// export (`export_website_screenshots`) already maps, so that mapping
/// doesn't need touching just because what the scene shows changed.
///
/// Generates its own clip with a moving test pattern rather than reusing one
/// of `seed_demo_library`'s flat-color clips — a solid color square in the
/// player's video area would read as a rendering bug, not a video, in a
/// screenshot meant to sell the trim tool.
async fn capture_trim_tool(app: &AppHandle, lang_dir: &Path) -> Result<()> {
    // Flat at the recordings root (like the "no detected game" demo item) —
    // `get_video_metadata`/`get_video_details` resolve a `name` against
    // `resolved_recordings_dir()`, so this only ffprobes correctly if it
    // actually lives there, not just anywhere on disk.
    let clip_name = "trim-tool-demo.mp4";
    let clip_path = library_dir().join(clip_name);
    generate_trim_demo_clip(app, &clip_path, 90).await?;

    send_cmd_and_wait(app, json!({"action":"goto-player","path": clip_path.to_string_lossy(),"name": clip_name})).await;
    // The player's own video element needs a moment to load metadata/first frame.
    tokio::time::sleep(Duration::from_millis(1200)).await;
    let result = capture_current(app, &lang_dir.join("14-editor.png")).await;
    // Leave the app back on the folders view for the next language pass.
    send_cmd_and_wait(app, json!({"action":"goto-view","view":"folders"})).await;
    result
}

// ---------------------------------------------------------------------------
// Synthetic demo data
// ---------------------------------------------------------------------------

/// The one demo game that gets its own subfolders, so the "drilled into a
/// game" scene has folder tiles to show, matching real usage.
const DEMO_GAME_WITH_FOLDERS: &str = "Grand Theft Auto V";
/// Matches the `id` of the "Highlights" folder seeded in `seed_demo_folders`
/// — the target for the "drilled into one folder" scene.
const DEMO_FOLDER_ID: &str = "demo-highlights";

struct DemoItem {
    /// Must match a display name in the embedded game-icon catalog
    /// (`games_db::embedded_icon_data_url`) so its card art renders fully
    /// offline, no real install or network round-trip needed. `None` = no
    /// detected game (a plain desktop/area capture) — needed so the Folders
    /// view's true root has at least one loose, unclassified recording to
    /// show (see `rootOnly` in VideoGrid.jsx); every item used to have a
    /// game, so that scene came up "No matching items".
    app: Option<&'static str>,
    folder: Option<&'static str>,
    tag: Option<&'static str>,
    /// `Some("clip")` for a replay-style short clip; `None` for a full recording.
    kind: Option<&'static str>,
    days_ago: i64,
    duration_secs: u64,
    /// ffmpeg `color=c=...` spec for the synthetic clip's background.
    color_hex: &'static str,
}

const DEMO_ITEMS: &[DemoItem] = &[
    DemoItem {
        app: Some(DEMO_GAME_WITH_FOLDERS), folder: None, tag: Some("highlight"), kind: None,
        days_ago: 1, duration_secs: 620, color_hex: "0x1b3a6b",
    },
    DemoItem {
        app: Some(DEMO_GAME_WITH_FOLDERS), folder: Some("Highlights"), tag: Some("highlight"), kind: None,
        days_ago: 2, duration_secs: 480, color_hex: "0x3d1b6b",
    },
    DemoItem {
        app: Some(DEMO_GAME_WITH_FOLDERS), folder: Some("Fails"), tag: Some("fail"), kind: Some("clip"),
        days_ago: 3, duration_secs: 45, color_hex: "0x6b1b1b",
    },
    DemoItem {
        app: Some("VALORANT"), folder: None, tag: Some("highlight"), kind: Some("clip"),
        days_ago: 4, duration_secs: 38, color_hex: "0xb02e2e",
    },
    DemoItem {
        app: Some("Minecraft"), folder: None, tag: None, kind: None,
        days_ago: 6, duration_secs: 900, color_hex: "0x2e7d32",
    },
    DemoItem {
        app: Some("League of Legends"), folder: None, tag: Some("highlight"), kind: Some("clip"),
        days_ago: 8, duration_secs: 52, color_hex: "0x0a3d62",
    },
    DemoItem {
        app: Some("Among Us"), folder: None, tag: Some("funny"), kind: None,
        days_ago: 11, duration_secs: 700, color_hex: "0xb8860b",
    },
    DemoItem {
        // No detected game — a plain desktop/area capture, loose at the
        // recordings root. Without at least one of these, the Folders
        // view's true root has nothing to show (every other item belongs
        // to some game) and reads as "No matching items".
        app: None, folder: None, tag: None, kind: None,
        days_ago: 5, duration_secs: 240, color_hex: "0x44403c",
    },
];

/// Three small demo tags (localized), so the gallery scene has something to
/// show in its tag filter.
fn seed_demo_tags(app: &AppHandle, lang: &str) {
    let tags = if lang == "tr" {
        vec![
            Tag { id: "highlight".into(), name: "Öne Çıkan".into(), color: "#22c55e".into() },
            Tag { id: "funny".into(), name: "Komik".into(), color: "#f59e0b".into() },
            Tag { id: "fail".into(), name: "Başarısız".into(), color: "#ef4444".into() },
        ]
    } else {
        vec![
            Tag { id: "highlight".into(), name: "Highlight".into(), color: "#22c55e".into() },
            Tag { id: "funny".into(), name: "Funny".into(), color: "#f59e0b".into() },
            Tag { id: "fail".into(), name: "Fail".into(), color: "#ef4444".into() },
        ]
    };
    app.state::<Arc<TagStore>>().save(tags);
}

/// Two subfolders under the one demo game that has them, matching the
/// on-disk layout `recording::prepare` uses for a real folder-targeted
/// recording (`<root>/<game>/<folder>/...`).
fn seed_demo_folders(app: &AppHandle) {
    let store = app.state::<Arc<ConfigStore>>();
    let mut settings = store.get();
    settings.recording_folders = vec![
        RecordingFolder {
            id: DEMO_FOLDER_ID.into(), name: "Highlights".into(),
            game: Some(DEMO_GAME_WITH_FOLDERS.into()), auto_delete_days: None,
            never_upload_to_drive: false, always_keep: false,
        },
        RecordingFolder {
            id: "demo-fails".into(), name: "Fails".into(),
            game: Some(DEMO_GAME_WITH_FOLDERS.into()), auto_delete_days: None,
            never_upload_to_drive: false, always_keep: false,
        },
    ];
    let _ = store.save(settings);
}

/// Shared by `generate_demo_clip` (flat color) and `generate_trim_demo_clip`
/// (moving test pattern) — everything but the `lavfi` video source is
/// identical: silent H.264, entirely offline, never a capture of anything
/// real. Valid enough for the app's own thumbnail extraction
/// (`ensure_thumbnail_cached`) and for a real video element to actually play.
async fn generate_demo_clip_from_source(app: &AppHandle, out_path: &Path, video_source: &str) -> Result<()> {
    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let cmd = app.shell().sidecar("ffmpeg").context("ffmpeg sidecar not found")?;
    let output = cmd
        .args([
            "-y",
            "-f", "lavfi", "-i", video_source,
            "-f", "lavfi", "-i", "anullsrc=r=44100:cl=stereo",
            "-c:v", "libx264", "-pix_fmt", "yuv420p", "-preset", "ultrafast",
            "-c:a", "aac", "-shortest",
            &out_path.to_string_lossy(),
        ])
        .output()
        .await
        .context("failed to run ffmpeg")?;
    if !output.status.success() {
        anyhow::bail!("ffmpeg demo clip generation failed: {}", String::from_utf8_lossy(&output.stderr));
    }
    Ok(())
}

async fn generate_demo_clip(app: &AppHandle, out_path: &Path, color_hex: &str, duration_secs: u64) -> Result<()> {
    let color_src = format!("color=c={color_hex}:s=1280x720:d={duration_secs}:r=30");
    generate_demo_clip_from_source(app, out_path, &color_src).await
}

/// A colorful, moving SMPTE-style test pattern (gradient bars + a sweeping
/// clock hand) instead of a flat color — used only for the trim tool's own
/// demo clip, whose screenshot needs to look like a real video, not a
/// solid-color rectangle.
async fn generate_trim_demo_clip(app: &AppHandle, out_path: &Path, duration_secs: u64) -> Result<()> {
    let pattern_src = format!("testsrc=size=1280x720:rate=30:duration={duration_secs}");
    generate_demo_clip_from_source(app, out_path, &pattern_src).await
}

/// Seeds the demo library: synthetic ffmpeg clips spread across different
/// days, each tagged and attributed to a real (embedded-catalog) game name.
/// Never touches the user's actual library. Returns each clip's absolute
/// path, for the editor scene.
async fn seed_demo_library(app: &AppHandle, dir: &Path) -> Result<Vec<PathBuf>> {
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(dir)?;
    let meta_store = app.state::<Arc<MetaStore>>();
    let now = Local::now();
    let mut paths = Vec::new();
    for item in DEMO_ITEMS {
        let ts = now - ChronoDuration::days(item.days_ago) - ChronoDuration::hours((item.days_ago * 3) % 17);
        let filename = format!("{}.mp4", ts.format("%Y-%m-%d_%H-%M-%S-%3f"));

        // `/`-joined regardless of platform — matches `list_video_files`'s
        // own relative-name format, which every metadata lookup keys off.
        // No game (`item.app == None`) means no game segment at all — the
        // file sits directly at the recordings root, same as a real
        // undetected desktop/area capture would.
        let mut rel_name = item.app.map(crate::drive::sanitize_filename).unwrap_or_default();
        if let Some(folder) = item.folder {
            rel_name = if rel_name.is_empty() { folder.to_string() } else { format!("{rel_name}/{folder}") };
        }
        rel_name = if rel_name.is_empty() { filename.clone() } else { format!("{rel_name}/{filename}") };
        let abs_path = dir.join(&rel_name);
        generate_demo_clip(app, &abs_path, item.color_hex, item.duration_secs).await?;

        // No `title`: a real recording is never auto-titled — the card falls
        // back to the plain timestamp filename, exactly like this one.
        meta_store.set(
            rel_name,
            VideoMeta {
                app: item.app.map(str::to_string),
                created: Some(ts.timestamp()),
                kind: item.kind.map(str::to_string),
                duration_secs: Some(item.duration_secs),
                tags: item.tag.map(|t| vec![t.to_string()]).unwrap_or_default(),
                favorite: false,
                ..Default::default()
            },
        );
        paths.push(abs_path);
    }
    Ok(paths)
}
