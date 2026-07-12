//! Native (no WebView2) toast overlay, drawn directly into a GDI layered
//! window — see `toast.rs`'s `USE_NATIVE` switch. Windows-only.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Manager};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, CreateFontW, DeleteDC, DeleteObject,
    DrawTextW, SelectObject, SetBkMode, SetTextColor, BITMAPINFO, BITMAPINFOHEADER,
    BI_RGB, CLEARTYPE_QUALITY, DEFAULT_CHARSET, DIB_RGB_COLORS, DT_CALCRECT, DT_NOPREFIX,
    DT_WORDBREAK, FW_BOLD, FW_NORMAL, HDC, HGDIOBJ, OUT_DEFAULT_PRECIS,
    CLIP_DEFAULT_PRECIS, TRANSPARENT as GDI_TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, KillTimer, RegisterClassExW, SetTimer,
    SetWindowPos, ShowWindow, UpdateLayeredWindow, CS_HREDRAW, CS_VREDRAW, HWND_TOPMOST, SWP_NOACTIVATE,
    SW_SHOWNOACTIVATE, ULW_ALPHA, WM_DESTROY, WM_TIMER, WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};

use crate::config::{ConfigStore, HudCorner};
use crate::toast::ToastCategory;

const CLASS_NAME: PCWSTR = w!("CapcoveNativeToast");
const CARD_W: i32 = 380;
const PAD_X: i32 = 16;
const PAD_Y: i32 = 14;
const ICON_SIZE: i32 = 44;
const GAP: i32 = 10;
const MAX_STACK: usize = 4;
const VISIBLE_MS: u64 = 3200;
// Entrance and exit mirror each other: same distance/duration, opposite easing.
const ENTER_MS: u64 = 340;
const EXIT_MS: u64 = 340;
const TOTAL_MS: u64 = VISIBLE_MS + EXIT_MS;
const TIMER_ID: usize = 1;
// `SetTimer` is coarse (OS may coalesce toward ~10-15ms); 8ms asks for headroom.
const TIMER_INTERVAL_MS: u32 = 8;
// How far a toast travels sliding in/out; `render` pads the canvas by this
// much so the slide has somewhere to go without clipping.
const SLIDE_DISTANCE: f32 = 460.0;

struct ToastEntry {
    kind: String,
    title: String,
    body: String,
    icon: Option<image::RgbaImage>,
    created: Instant,
}

struct SharedState {
    app: AppHandle,
    hwnd: isize,
    corner: HudCorner,
}

static STATE: Mutex<Option<SharedState>> = Mutex::new(None);
static TOASTS: Mutex<Vec<ToastEntry>> = Mutex::new(Vec::new());
// Reused across `render()` calls instead of a fresh `Vec` every frame.
static PIXEL_BUF: Mutex<Vec<u8>> = Mutex::new(Vec::new());

fn hwnd_of(state: &SharedState) -> HWND {
    HWND(state.hwnd as *mut _)
}

/// Builds the native toast window at startup, same timing as the WebView
/// version's `preload`.
pub fn preload(app: &AppHandle) {
    let app2 = app.clone();
    let _ = app.run_on_main_thread(move || {
        ensure_window(&app2);
    });
}

fn ensure_window(app: &AppHandle) {
    if STATE.lock().unwrap().is_some() {
        return;
    }
    let Ok(Some(monitor)) = app.primary_monitor() else { return };
    let hinstance = unsafe { GetModuleHandleW(None) }.unwrap_or_default();

    unsafe {
        let class = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wndproc),
            hInstance: hinstance.into(),
            lpszClassName: CLASS_NAME,
            ..Default::default()
        };
        RegisterClassExW(&class);

        let ex_style = WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT;
        let Ok(hwnd) = CreateWindowExW(
            ex_style,
            CLASS_NAME,
            w!("Capcove Toast"),
            WS_POPUP,
            monitor.position().x,
            monitor.position().y,
            1,
            1,
            None,
            None,
            windows::Win32::Foundation::HINSTANCE::from(hinstance),
            None,
        ) else {
            log::warn!("native toast window could not be created");
            return;
        };

        let hwnd_u32 = hwnd.0 as usize as u32;
        crate::win_util::make_overlay_ghost(hwnd_u32);
        let hide = app.state::<std::sync::Arc<ConfigStore>>().get().hide_overlays_from_capture;
        crate::win_util::set_capture_hidden(hwnd_u32, hide);
        // A plain `WS_POPUP` window is never actually mapped without this,
        // layered/`UpdateLayeredWindow`-driven or not; `NOACTIVATE` matches
        // the `WS_EX_NOACTIVATE` style so showing it never steals focus.
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);

        *STATE.lock().unwrap() = Some(SharedState {
            app: app.clone(),
            hwnd: hwnd.0 as isize,
            corner: HudCorner::default(),
        });
    }
}

pub fn apply_capture_hidden(_app: &AppHandle, hidden: bool) {
    if let Some(state) = STATE.lock().unwrap().as_ref() {
        crate::win_util::set_capture_hidden(state.hwnd as usize as u32, hidden);
    }
}

pub fn show_with_icon(app: &AppHandle, kind: &str, category: ToastCategory, title: &str, body: &str, icon: Option<String>) {
    if !category.enabled(app) {
        return;
    }
    let icon_img = icon.as_deref().and_then(decode_data_url_icon);
    let app2 = app.clone();
    let kind = kind.to_string();
    let title = title.to_string();
    let body = body.to_string();
    let _ = app.run_on_main_thread(move || {
        ensure_window(&app2);
        let corner = app2.state::<std::sync::Arc<ConfigStore>>().get().video.toast_corner;
        if let Some(state) = STATE.lock().unwrap().as_mut() {
            state.corner = corner;
        }
        {
            let mut toasts = TOASTS.lock().unwrap();
            toasts.push(ToastEntry { kind, title, body, icon: icon_img, created: Instant::now() });
            let len = toasts.len();
            if len > MAX_STACK {
                toasts.drain(0..len - MAX_STACK);
            }
        }
        ensure_timer_running();
        render(&app2);
    });
}

pub fn show_for_game(app: &AppHandle, kind: &str, category: ToastCategory, title: &str, body: &str, game_name: &str) {
    let icon = game_icon_data_url(app, game_name);
    show_with_icon(app, kind, category, title, body, icon);
}

fn game_icon_data_url(app: &AppHandle, name: &str) -> Option<String> {
    let ic = app.state::<std::sync::Arc<crate::icon_cache::IconCache>>();
    if let Some(b64) = ic.get_base64(name) {
        return Some(format!("data:image/png;base64,{b64}"));
    }
    crate::games_db::embedded_icon_data_url(app, name)
}

fn decode_data_url_icon(data_url: &str) -> Option<image::RgbaImage> {
    let b64 = data_url.split_once("base64,")?.1;
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD.decode(b64).ok()?;
    let img = image::load_from_memory(&bytes).ok()?;
    Some(image::imageops::resize(&img.to_rgba8(), ICON_SIZE as u32, ICON_SIZE as u32, image::imageops::FilterType::Triangle))
}

fn ensure_timer_running() {
    if let Some(state) = STATE.lock().unwrap().as_ref() {
        unsafe {
            SetTimer(hwnd_of(state), TIMER_ID, TIMER_INTERVAL_MS, None);
        }
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_TIMER => {
            let app = STATE.lock().unwrap().as_ref().map(|s| s.app.clone());
            if let Some(app) = app {
                tick(&app);
            }
            LRESULT(0)
        }
        WM_DESTROY => LRESULT(0),
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// Drops expired toasts and re-renders; stops the timer once nothing's left.
fn tick(app: &AppHandle) {
    let now = Instant::now();
    {
        let mut toasts = TOASTS.lock().unwrap();
        toasts.retain(|t| now.duration_since(t.created) < Duration::from_millis(TOTAL_MS));
    }
    let empty = TOASTS.lock().unwrap().is_empty();
    if empty {
        if let Some(state) = STATE.lock().unwrap().as_ref() {
            unsafe { let _ = KillTimer(hwnd_of(state), TIMER_ID); }
        }
        hide_window();
        return;
    }
    render(app);
}

fn hide_window() {
    if let Some(state) = STATE.lock().unwrap().as_ref() {
        let hwnd = hwnd_of(state);
        unsafe {
            let _ = SetWindowPos(hwnd, HWND_TOPMOST, 0, 0, 1, 1, SWP_NOACTIVATE);
        }
    }
}

fn ease_out(t: f32) -> f32 {
    1.0 - (1.0 - t) * (1.0 - t) * (1.0 - t)
}

fn ease_in(t: f32) -> f32 {
    t * t * t
}

/// Returns `(slide_px_from_rest, alpha_mul)` for a toast's current age.
/// Exit stays fully opaque through the whole slide (alpha only drops after
/// `TOTAL_MS`) — fading in step with position would make it look like it
/// vanishes in place rather than travels off-screen.
fn animate(age_ms: u64) -> (f32, f32) {
    if age_ms < ENTER_MS {
        let t = ease_out(age_ms as f32 / ENTER_MS as f32);
        ((1.0 - t) * SLIDE_DISTANCE, t)
    } else if age_ms < VISIBLE_MS {
        (0.0, 1.0)
    } else if age_ms < TOTAL_MS {
        let t = ease_in((age_ms - VISIBLE_MS) as f32 / EXIT_MS as f32);
        (t * SLIDE_DISTANCE, 1.0)
    } else {
        (SLIDE_DISTANCE, 0.0)
    }
}

struct Measured {
    height: i32,
    body_lines_rect: RECT,
}

/// Measures a toast's card height via GDI's own text layout (`DT_CALCRECT`),
/// so wrapped body text gets the same box the real render pass will use.
unsafe fn measure(hdc: HDC, entry: &ToastEntry, font_body: HGDIOBJ) -> Measured {
    let text_x = PAD_X + if entry.icon.is_some() { ICON_SIZE + 12 } else { 0 };
    let text_w = CARD_W - text_x - PAD_X;
    let title_h = 20;
    if entry.body.is_empty() {
        return Measured { height: (PAD_Y * 2 + title_h).max(if entry.icon.is_some() { ICON_SIZE + PAD_Y * 2 } else { 0 }), body_lines_rect: RECT::default() };
    }
    let old = SelectObject(hdc, font_body);
    let mut rect = RECT { left: 0, top: 0, right: text_w, bottom: 0 };
    let mut wide: Vec<u16> = entry.body.encode_utf16().chain(std::iter::once(0)).collect();
    DrawTextW(hdc, &mut wide, &mut rect, DT_CALCRECT | DT_WORDBREAK | DT_NOPREFIX);
    SelectObject(hdc, old);
    // Cap at 3 lines, matching the old CSS's `-webkit-line-clamp: 3`.
    let line_h = 18;
    let max_h = line_h * 3;
    if rect.bottom > max_h {
        rect.bottom = max_h;
    }
    let content_h = title_h + 4 + rect.bottom;
    let height = (PAD_Y * 2 + content_h).max(if entry.icon.is_some() { ICON_SIZE + PAD_Y * 2 } else { 0 });
    Measured { height, body_lines_rect: rect }
}

fn render(app: &AppHandle) {
    let Some((hwnd, corner)) = STATE.lock().unwrap().as_ref().map(|s| (hwnd_of(s), s.corner.clone())) else { return };
    let Ok(Some(monitor)) = app.primary_monitor() else { return };
    let mon_pos = *monitor.position();
    let mon_size = *monitor.size();

    let toasts = TOASTS.lock().unwrap();
    if toasts.is_empty() {
        return;
    }

    unsafe {
        let hdc_screen = windows::Win32::Graphics::Gdi::GetDC(None);
        let hdc_mem = CreateCompatibleDC(hdc_screen);
        let font_title = make_font(14, true);
        let font_body = make_font(13, false);

        // First pass: measure every visible card to lay out the stack.
        let old_font = SelectObject(hdc_mem, HGDIOBJ(font_body.0));
        let measured: Vec<Measured> = toasts.iter().map(|t| measure(hdc_mem, t, HGDIOBJ(font_body.0))).collect();
        SelectObject(hdc_mem, old_font);

        let total_h: i32 = measured.iter().map(|m| m.height).sum::<i32>() + GAP * (measured.len() as i32 - 1).max(0);
        let from_right = matches!(corner, HudCorner::TopRight | HudCorner::BottomRight);
        let from_bottom = matches!(corner, HudCorner::BottomLeft | HudCorner::BottomRight);
        // Canvas is padded on the side the toast slides toward, so the slide
        // has somewhere to go instead of clipping at a card-sized buffer.
        let pad = SLIDE_DISTANCE as i32;
        let win_w = CARD_W + pad;
        let win_h = total_h.max(1);
        let rest_offset = if from_right { 0 } else { pad };
        // Tighter clip than the padded buffer bounds, so a sliding toast
        // never renders onto a neighboring monitor sitting in the pad area.
        let vis_min = rest_offset;
        let vis_max = rest_offset + CARD_W;

        // Reused across frames; always re-zeroed so a stale previous frame
        // can't bleed through the padding a toast slides through.
        let mut pixels_buf = PIXEL_BUF.lock().unwrap();
        let pixel_len = (win_w * win_h * 4) as usize;
        if pixels_buf.len() < pixel_len {
            pixels_buf.resize(pixel_len, 0);
        } else {
            pixels_buf.truncate(pixel_len);
        }
        pixels_buf.fill(0);
        let pixels: &mut [u8] = &mut pixels_buf;
        // Oldest anchors at the corner; each newer toast stacks further away.
        let mut y = if from_bottom { win_h } else { 0 };
        for idx in 0..toasts.len() {
            let entry = &toasts[idx];
            let m = &measured[idx];
            if from_bottom {
                y -= m.height;
            }
            let age_ms = Instant::now().duration_since(entry.created).as_millis() as u64;
            let (slide, alpha_mul) = animate(age_ms);
            let signed_slide = if from_right { slide } else { -slide };
            let x_offset = rest_offset + signed_slide as i32;

            draw_card(pixels, win_w, win_h, vis_min, vis_max, x_offset, y, entry, m, alpha_mul, hdc_mem, HGDIOBJ(font_title.0), HGDIOBJ(font_body.0));
            if !from_bottom {
                y += m.height + GAP;
            } else {
                y -= GAP;
            }
        }

        // Premultiply RGB by alpha — required for `ULW_ALPHA` compositing.
        for px in pixels.chunks_exact_mut(4) {
            let a = px[3] as u32;
            px[0] = ((px[0] as u32 * a) / 255) as u8;
            px[1] = ((px[1] as u32 * a) / 255) as u8;
            px[2] = ((px[2] as u32 * a) / 255) as u8;
        }

        let mut bmi: BITMAPINFO = std::mem::zeroed();
        bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bmi.bmiHeader.biWidth = win_w;
        bmi.bmiHeader.biHeight = -win_h; // top-down
        bmi.bmiHeader.biPlanes = 1;
        bmi.bmiHeader.biBitCount = 32;
        bmi.bmiHeader.biCompression = BI_RGB.0;

        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let Ok(hbitmap) = CreateDIBSection(hdc_screen, &bmi, DIB_RGB_COLORS, &mut bits, None, 0) else {
            let _ = DeleteDC(hdc_mem);
            windows::Win32::Graphics::Gdi::ReleaseDC(None, hdc_screen);
            return;
        };
        if !bits.is_null() {
            std::ptr::copy_nonoverlapping(pixels.as_ptr(), bits as *mut u8, pixels.len());
        }
        let old_bmp = SelectObject(hdc_mem, HGDIOBJ(hbitmap.0));

        // Anchored by `CARD_W`, not the padded `win_w`, so the resting
        // position ignores the extra slide-out room.
        let win_x = if from_right { mon_pos.x + mon_size.width as i32 - CARD_W } else { mon_pos.x - pad };
        let win_y = if from_bottom { mon_pos.y + mon_size.height as i32 - win_h } else { mon_pos.y };

        let src_pt = POINT { x: 0, y: 0 };
        let dst_pt = POINT { x: win_x, y: win_y };
        let size = SIZE { cx: win_w, cy: win_h };
        let blend = windows::Win32::Graphics::Gdi::BLENDFUNCTION {
            BlendOp: windows::Win32::Graphics::Gdi::AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: windows::Win32::Graphics::Gdi::AC_SRC_ALPHA as u8,
        };
        let _ = UpdateLayeredWindow(hwnd, hdc_screen, Some(&dst_pt), Some(&size), hdc_mem, Some(&src_pt), COLORREF(0), Some(&blend), ULW_ALPHA);

        SelectObject(hdc_mem, old_bmp);
        let _ = DeleteObject(HGDIOBJ(hbitmap.0));
        let _ = DeleteObject(HGDIOBJ(font_title.0));
        let _ = DeleteObject(HGDIOBJ(font_body.0));
        let _ = DeleteDC(hdc_mem);
        windows::Win32::Graphics::Gdi::ReleaseDC(None, hdc_screen);
    }
}

unsafe fn make_font(px: i32, bold: bool) -> windows::Win32::Graphics::Gdi::HFONT {
    CreateFontW(
        -px, 0, 0, 0,
        if bold { FW_BOLD.0 as i32 } else { FW_NORMAL.0 as i32 },
        0, 0, 0,
        DEFAULT_CHARSET.0 as u32,
        OUT_DEFAULT_PRECIS.0 as u32,
        CLIP_DEFAULT_PRECIS.0 as u32,
        CLEARTYPE_QUALITY.0 as u32,
        0,
        w!("Segoe UI"),
    )
}

/// `vis_min`/`vis_max` clip drawing to the primary monitor's real span,
/// tighter than the padded buffer bounds — keeps a sliding toast off a
/// neighboring monitor that happens to sit in the pad area.
#[allow(clippy::too_many_arguments)]
unsafe fn draw_card(
    pixels: &mut [u8], win_w: i32, win_h: i32, vis_min: i32, vis_max: i32, x_offset: i32, y: i32,
    entry: &ToastEntry, m: &Measured, alpha_mul: f32,
    hdc: HDC, font_title: HGDIOBJ, font_body: HGDIOBJ,
) {
    let accent = if entry.kind == "error" { (0xef, 0x44, 0x44) } else { (0x22, 0xd3, 0xee) };
    let bg = (15u8, 14u8, 13u8);
    let card_alpha = (247.0 * alpha_mul.clamp(0.0, 1.0)) as u8;

    // Clipped to window bounds since a fast slide can push part of the card off-edge.
    for row in 0..m.height {
        let py = y + row;
        if py < 0 || py >= win_h {
            continue;
        }
        for col in 0..CARD_W {
            let px = col + x_offset;
            if px < vis_min || px >= vis_max {
                continue;
            }
            let i = ((py * win_w + px) * 4) as usize;
            pixels[i] = bg.2;
            pixels[i + 1] = bg.1;
            pixels[i + 2] = bg.0;
            pixels[i + 3] = card_alpha;
        }
    }
    // Faint top/bottom border: blend RGB into the existing opaque background
    // rather than layering translucent white, which reads as a solid line.
    const BORDER_RATIO: f32 = 0.08;
    for col in 0..CARD_W {
        for &row in &[0, m.height - 1] {
            let px = col + x_offset;
            let py = y + row;
            if px < vis_min || px >= vis_max || py < 0 || py >= win_h {
                continue;
            }
            let i = ((py * win_w + px) * 4) as usize;
            for c in 0..3 {
                pixels[i + c] = (pixels[i + c] as f32 * (1.0 - BORDER_RATIO) + 255.0 * BORDER_RATIO) as u8;
            }
        }
    }

    // Icon, blitted with its own alpha composited against the card background.
    let mut text_x = PAD_X;
    if let Some(icon) = &entry.icon {
        let icon_y = y + (m.height - ICON_SIZE) / 2;
        for iy in 0..ICON_SIZE {
            let py = icon_y + iy;
            if py < 0 || py >= win_h {
                continue;
            }
            for ix in 0..ICON_SIZE {
                let px = PAD_X + ix + x_offset;
                if px < vis_min || px >= vis_max {
                    continue;
                }
                let src = icon.get_pixel(ix as u32, iy as u32).0;
                let src_a = (src[3] as f32 * alpha_mul.clamp(0.0, 1.0)) as u32;
                if src_a == 0 {
                    continue;
                }
                let i = ((py * win_w + px) * 4) as usize;
                for c in 0..3 {
                    let dst = pixels[i + c] as u32;
                    pixels[i + c] = (((src[c] as u32) * src_a + dst * (255 - src_a)) / 255) as u8;
                }
                pixels[i + 3] = pixels[i + 3].max(src_a as u8);
            }
        }
        text_x = PAD_X + ICON_SIZE + 12;
    }

    // Accent dot + title + body, drawn via GDI directly into the (already
    // opaque, RGB-only) backdrop — GDI ignores the alpha channel, so its own
    // anti-aliasing blends correctly against the solid fill above, and the
    // uniform `card_alpha` we already wrote stays valid for these pixels.
    let dot = 3;
    for dy in 0..dot * 2 {
        for dx in 0..dot * 2 {
            let px = text_x + dx + x_offset;
            let py = y + PAD_Y + 8 + dy;
            if px < vis_min || px >= vis_max || py < 0 || py >= win_h {
                continue;
            }
            let i = ((py * win_w + px) * 4) as usize;
            pixels[i] = accent.2;
            pixels[i + 1] = accent.1;
            pixels[i + 2] = accent.0;
        }
    }

    let text_w = CARD_W - (text_x - PAD_X) - PAD_X;
    let hbitmap_stub = windows::Win32::Graphics::Gdi::HBITMAP::default();
    let _ = hbitmap_stub;

    draw_text_into(pixels, win_w, win_h, vis_min, vis_max, x_offset, text_x + dot * 2 + 6, y + PAD_Y - 3, text_w, 20, &entry.title, hdc, font_title, (240, 239, 238), card_alpha);
    if !entry.body.is_empty() {
        draw_text_wrapped(pixels, win_w, win_h, vis_min, vis_max, x_offset, text_x, y + PAD_Y + 20, text_w, m.body_lines_rect.bottom.max(18), &entry.body, hdc, font_body, (168, 162, 157), card_alpha);
    }
}

/// Renders one line of text via GDI into a scratch bitmap already filled
/// with the destination's own background color, then copies just the RGB
/// bytes back — see `draw_card`'s comment on why this keeps alpha correct.
#[allow(clippy::too_many_arguments)]
unsafe fn draw_text_into(
    pixels: &mut [u8], win_w: i32, win_h: i32, vis_min: i32, vis_max: i32, x_offset: i32,
    x: i32, y: i32, w: i32, h: i32, text: &str,
    hdc: HDC, font: HGDIOBJ, color: (u8, u8, u8), alpha: u8,
) {
    draw_text_region(pixels, win_w, win_h, vis_min, vis_max, x_offset, x, y, w, h, text, hdc, font, color, alpha, false);
}

#[allow(clippy::too_many_arguments)]
unsafe fn draw_text_wrapped(
    pixels: &mut [u8], win_w: i32, win_h: i32, vis_min: i32, vis_max: i32, x_offset: i32,
    x: i32, y: i32, w: i32, h: i32, text: &str,
    hdc: HDC, font: HGDIOBJ, color: (u8, u8, u8), alpha: u8,
) {
    draw_text_region(pixels, win_w, win_h, vis_min, vis_max, x_offset, x, y, w, h, text, hdc, font, color, alpha, true);
}

#[allow(clippy::too_many_arguments)]
unsafe fn draw_text_region(
    pixels: &mut [u8], win_w: i32, win_h: i32, vis_min: i32, vis_max: i32, x_offset: i32,
    x: i32, y: i32, w: i32, h: i32, text: &str,
    hdc: HDC, font: HGDIOBJ, color: (u8, u8, u8), alpha: u8,
    wrap: bool,
) {
    if w <= 0 || h <= 0 {
        return;
    }
    let mut bmi: BITMAPINFO = std::mem::zeroed();
    bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
    bmi.bmiHeader.biWidth = w;
    bmi.bmiHeader.biHeight = -h;
    bmi.bmiHeader.biPlanes = 1;
    bmi.bmiHeader.biBitCount = 32;
    bmi.bmiHeader.biCompression = BI_RGB.0;

    let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
    let Ok(hbitmap) = CreateDIBSection(None, &bmi, DIB_RGB_COLORS, &mut bits, None, 0) else { return };
    if bits.is_null() {
        let _ = DeleteObject(HGDIOBJ(hbitmap.0));
        return;
    }
    // Seed the scratch bitmap with the card's actual current background so
    // GDI's text anti-aliasing blends against the right color.
    let scratch = std::slice::from_raw_parts_mut(bits as *mut u8, (w * h * 4) as usize);
    for row in 0..h {
        for col in 0..w {
            let src_x = x + col + x_offset;
            let src_y = y + row;
            let si = ((row * w + col) * 4) as usize;
            if src_x >= vis_min && src_x < vis_max && src_y >= 0 && src_y < win_h {
                let di = ((src_y * win_w + src_x) * 4) as usize;
                scratch[si] = pixels[di];
                scratch[si + 1] = pixels[di + 1];
                scratch[si + 2] = pixels[di + 2];
            }
        }
    }

    let old_bmp = SelectObject(hdc, HGDIOBJ(hbitmap.0));
    let old_font = SelectObject(hdc, font);
    SetBkMode(hdc, GDI_TRANSPARENT);
    SetTextColor(hdc, COLORREF(u32::from_le_bytes([color.2, color.1, color.0, 0])));
    let mut rect = RECT { left: 0, top: 0, right: w, bottom: h };
    let flags = if wrap { DT_WORDBREAK | DT_NOPREFIX } else { DT_NOPREFIX };
    let mut wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    DrawTextW(hdc, &mut wide, &mut rect, flags);
    SelectObject(hdc, old_font);
    SelectObject(hdc, old_bmp);

    for row in 0..h {
        for col in 0..w {
            let dst_x = x + col + x_offset;
            let dst_y = y + row;
            if dst_x < vis_min || dst_x >= vis_max || dst_y < 0 || dst_y >= win_h {
                continue;
            }
            let si = ((row * w + col) * 4) as usize;
            let di = ((dst_y * win_w + dst_x) * 4) as usize;
            pixels[di] = scratch[si];
            pixels[di + 1] = scratch[si + 1];
            pixels[di + 2] = scratch[si + 2];
            pixels[di + 3] = alpha;
        }
    }
    let _ = DeleteObject(HGDIOBJ(hbitmap.0));
}
