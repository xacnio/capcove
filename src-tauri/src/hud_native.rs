//! Native (no WebView2) rendering backend for the recording HUD badges —
//! see `recording::hud`'s `USE_NATIVE` switch. State/settings/monitor logic
//! stays in `recording::hud`; this module only owns the layered window and
//! drawing it. Icons mirror `src/lib/hudIcons.js`'s SVG markup, rasterized
//! via `resvg` and cached.

use std::collections::HashMap;
use std::sync::Mutex;

use tauri::{AppHandle, Monitor};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, SelectObject, BITMAPINFO,
    BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HGDIOBJ,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, RegisterClassExW, SetWindowPos, ShowWindow,
    UpdateLayeredWindow, CS_HREDRAW, CS_VREDRAW, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE,
    SWP_NOSIZE, SW_SHOWNOACTIVATE, ULW_ALPHA, WM_DESTROY, WNDCLASSEXW, WS_EX_LAYERED,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};

use crate::config::HudCorner;
use crate::recording::hud::Badges;

const CLASS_NAME: PCWSTR = w!("CapcoveNativeHud");
const BADGE: i32 = 28;
const GAP: i32 = 6;
const ICON_PX: u32 = 15;

/// Exact copy of `src/lib/hudIcons.js`'s `HUD_ICONS` — inner SVG markup,
/// `viewBox="0 0 24 24"`, fills only.
const ICONS: &[(&str, &str)] = &[
    ("dot", r#"<circle cx="12" cy="12" r="7"/>"#),
    ("camera", r#"<path d="M17 10.5V7c0-.55-.45-1-1-1H4c-.55 0-1 .45-1 1v10c0 .55.45 1 1 1h12c.55 0 1-.45 1-1v-3.5l4 4v-11l-4 4z"/>"#),
    ("square", r#"<rect x="6" y="6" width="12" height="12" rx="2"/>"#),
    ("controller", r#"<rect x="2" y="9" width="20" height="8" rx="4"/><circle cx="6" cy="17" r="3"/><circle cx="18" cy="17" r="3"/>"#),
    ("flame", r#"<path d="M12 2c-1.5 3.5-5 5-5 10a5 5 0 0 0 10 0c0-1.2-.4-2.2-1-3 .2 2-.9 3.5-2 3.5-1.2 0-2-1-2-2.2 0-2 1.8-3 1.8-6.3-.9 1-1.8 2.3-1.8 4.3z"/>"#),
    ("target", r#"<path fill-rule="evenodd" d="M12 2a10 10 0 1 0 0.01 0z M12 5a7 7 0 1 0 0.01 0z"/><circle cx="12" cy="12" r="3"/>"#),
    ("history", r#"<path d="M13 3a9 9 0 0 0-9 9H1l3.89 3.89.07.14L9 12H6c0-3.87 3.13-7 7-7s7 3.13 7 7-3.13 7-7 7c-1.93 0-3.68-.79-4.94-2.06l-1.42 1.42A8.954 8.954 0 0 0 13 21a9 9 0 0 0 0-18zm-1 5v5l4.28 2.54.72-1.21-3.5-2.08V8H12z"/>"#),
    ("rewind", r#"<path d="M11 18V6l-8.5 6 8.5 6zm.5-6 8.5 6V6l-8.5 6z"/>"#),
    ("bolt", r#"<path d="M7 2v11h3v9l7-12h-4l4-8z"/>"#),
    ("sparkle", r#"<path d="M12 2l1.8 6.2L20 10l-6.2 1.8L12 18l-1.8-6.2L4 10l6.2-1.8z"/>"#),
    ("hourglass", r#"<polygon points="6,3 18,3 12,11"/><polygon points="6,21 18,21 12,13"/><rect x="6" y="2" width="12" height="1.6"/><rect x="6" y="20.4" width="12" height="1.6"/>"#),
    ("mic", r#"<path d="M12 14a3 3 0 0 0 3-3V5a3 3 0 0 0-6 0v6a3 3 0 0 0 3 3zm5-3a5 5 0 0 1-10 0H5a7 7 0 0 0 6 6.92V21h2v-3.08A7 7 0 0 0 19 11h-2z"/>"#),
    ("mic_alt", r#"<path d="M12 15c1.66 0 3-1.34 3-3V6c0-1.66-1.34-3-3-3S9 4.34 9 6v6c0 1.66 1.34 3 3 3zm6-3c0 3.31-2.69 6-6 6s-6-2.69-6-6H4c0 3.53 2.61 6.43 6 6.92V21h4v-2.08c3.39-.49 6-3.39 6-6.92h-2z"/>"#),
    ("speaker", r#"<path d="M4 9v6h4l5 4V5L8 9H4z"/>"#),
    ("headset", r#"<rect x="4" y="3" width="16" height="4" rx="2"/><rect x="3" y="6" width="3.5" height="9" rx="1.6"/><rect x="17.5" y="6" width="3.5" height="9" rx="1.6"/>"#),
    ("star", r#"<path d="M12 17.27L18.18 21l-1.64-7.03L22 9.24l-7.19-.61L12 2 9.19 8.63 2 9.24l5.46 4.73L5.82 21z"/>"#),
    ("heart", r#"<path d="M12 21.35l-1.45-1.32C5.4 15.36 2 12.28 2 8.5 2 5.42 4.42 3 7.5 3c1.74 0 3.41.81 4.5 2.09C13.09 3.81 14.76 3 16.5 3 19.58 3 22 5.42 22 8.5c0 3.78-3.4 6.86-8.55 11.54L12 21.35z"/>"#),
    ("diamond", r#"<polygon points="12,3 19,9 12,21 5,9"/>"#),
    ("crown", r#"<path d="M4 8l3 3 5-6 5 6 3-3-2 10H6z"/>"#),
    ("shield", r#"<path d="M12 2l7 3v6c0 5-3.5 8.5-7 10-3.5-1.5-7-5-7-10V5z"/>"#),
    ("trophy", r#"<path d="M7 4h10v2h3v2a4 4 0 0 1-4 4c-.6 1.6-1.9 2.8-3.5 3.3V18h3v2H8v-2h3v-2.7C9.4 14.8 8.1 13.6 7.5 12A4 4 0 0 1 4 8V6h3V4zm-3 4a2 2 0 0 0 2 2 8.6 8.6 0 0 1-.3-2H4zm16 0h-1.7a8.6 8.6 0 0 1-.3 2 2 2 0 0 0 2-2z"/>"#),
    ("rocket", r#"<path d="M12 2c3 2 5 6 5 10 0 2-1 4-2 5l-1-3-2 2-2-2-1 3c-1-1-2-3-2-5 0-4 2-8 5-10zm0 5a2 2 0 1 0 0 4 2 2 0 0 0 0-4zM8 17l-3 4 4-1zm8 0l3 4-4-1z"/>"#),
    ("skull", r#"<path d="M12 3a7 7 0 0 0-7 7v3l1.5 2v2h2v2h2v-2h3v2h2v-2h2v-2L19 13v-3a7 7 0 0 0-7-7zM9 11a1.3 1.3 0 1 1 0 2.6A1.3 1.3 0 0 1 9 11zm6 0a1.3 1.3 0 1 1 0 2.6A1.3 1.3 0 0 1 15 11z"/>"#),
    ("ghost", r#"<path d="M12 2a7 7 0 0 0-7 7v11l2.5-2 2 2 2.5-2 2.5 2 2-2 2.5 2V9a7 7 0 0 0-7-7zM9.5 9a1.3 1.3 0 1 1 0 2.6A1.3 1.3 0 0 1 9.5 9zm5 0a1.3 1.3 0 1 1 0 2.6A1.3 1.3 0 0 1 14.5 9z"/>"#),
    ("swords", r#"<path d="M3 3l7 7-1.5 1.5-7-7zM21 3l-7 7 1.5 1.5 7-7zM10 14l-6 6 1.5 1.5 6-6zm4 0l6 6-1.5 1.5-6-6z"/>"#),
];

/// Rasterized once per icon key and cached — an SVG parse+render per badge
/// per frame would be wasteful when nothing about a badge's icon changes
/// between renders.
static ICON_CACHE: Mutex<Option<HashMap<String, image::RgbaImage>>> = Mutex::new(None);

fn icon_bitmap(key: &str) -> Option<image::RgbaImage> {
    {
        let cache = ICON_CACHE.lock().unwrap();
        if let Some(map) = cache.as_ref() {
            if let Some(img) = map.get(key) {
                return Some(img.clone());
            }
        }
    }
    let markup = ICONS.iter().find(|(k, _)| *k == key)?.1;
    let svg = format!(r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="#ffffff">{markup}</svg>"##);
    let opt = resvg::usvg::Options::default();
    let tree = resvg::usvg::Tree::from_str(&svg, &opt).ok()?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(ICON_PX, ICON_PX)?;
    // Every icon in `ICONS` shares the same `viewBox="0 0 24 24"` — scaling
    // against that fixed, known size instead of `tree.size()` (which depends
    // on how usvg resolves the *absence* of explicit width/height attributes)
    // avoids a size mismatch silently mis-scaling every icon.
    let scale = ICON_PX as f32 / 24.0;
    resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    let img = image::RgbaImage::from_raw(ICON_PX, ICON_PX, pixmap.data().to_vec())?;
    ICON_CACHE.lock().unwrap().get_or_insert_with(HashMap::new).insert(key.to_string(), img.clone());
    Some(img)
}

struct WinState {
    hwnd: isize,
}

static STATE: Mutex<Option<WinState>> = Mutex::new(None);
// Reused across `render()` calls instead of a fresh `Vec` every call.
static PIXEL_BUF: Mutex<Vec<u8>> = Mutex::new(Vec::new());

fn hwnd_of(state: &WinState) -> HWND {
    HWND(state.hwnd as *mut _)
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_DESTROY => LRESULT(0),
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn ensure_window(_app: &AppHandle) -> Option<HWND> {
    if let Some(state) = STATE.lock().unwrap().as_ref() {
        return Some(hwnd_of(state));
    }
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
            ex_style, CLASS_NAME, w!("Capcove HUD"), WS_POPUP,
            0, 0, 1, 1, None, None,
            windows::Win32::Foundation::HINSTANCE::from(hinstance), None,
        ) else {
            log::warn!("native HUD window could not be created");
            return None;
        };
        let hwnd_u32 = hwnd.0 as usize as u32;
        crate::win_util::make_overlay_ghost(hwnd_u32);
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        *STATE.lock().unwrap() = Some(WinState { hwnd: hwnd.0 as isize });
        Some(hwnd)
    }
}

pub fn apply_capture_hidden(_app: &AppHandle, hidden: bool) {
    if let Some(state) = STATE.lock().unwrap().as_ref() {
        crate::win_util::set_capture_hidden(state.hwnd as usize as u32, hidden);
    }
}

/// Shrinks the window to nothing — mirrors the webview backend closing its
/// window when no badge is visible, without actually tearing down the
/// layered window (cheap enough to just leave parked at 1x1).
pub fn hide(_app: &AppHandle) {
    if let Some(state) = STATE.lock().unwrap().as_ref() {
        let hwnd = hwnd_of(state);
        unsafe {
            let _ = SetWindowPos(hwnd, HWND_TOPMOST, 0, 0, 1, 1, SWP_NOACTIVATE);
        }
    }
}

/// Computes the exact physical-pixel position for `corner`, flush against
/// the monitor's full physical bounds (not the work area — the HUD sits on
/// top of the taskbar). Mirrors `hud::corner_physical_position` but takes
/// the window size directly instead of querying a live `WebviewWindow`.
fn corner_position(monitor: &Monitor, corner: HudCorner, win_w: i32, win_h: i32) -> (i32, i32) {
    const MARGIN_PX: i32 = 6;
    let mpos = monitor.position();
    let msize = monitor.size();
    let (mw, mh) = (msize.width as i32, msize.height as i32);
    let (x, y) = match corner {
        HudCorner::TopLeft => (MARGIN_PX, MARGIN_PX),
        HudCorner::TopRight => (mw - win_w - MARGIN_PX, MARGIN_PX),
        HudCorner::BottomLeft => (MARGIN_PX, mh - win_h - MARGIN_PX),
        HudCorner::BottomRight => (mw - win_w - MARGIN_PX, mh - win_h - MARGIN_PX),
    };
    (mpos.x + x, mpos.y + y)
}

pub fn render(app: &AppHandle, badges: &Badges, monitor: &Monitor, corner: HudCorner, win_w: i32, win_h: i32) {
    let Some(hwnd) = ensure_window(app) else { return };
    let (win_x, win_y) = corner_position(monitor, corner, win_w, win_h);

    // Reused across calls instead of a fresh allocation each time.
    let mut pixels_buf = PIXEL_BUF.lock().unwrap();
    let pixel_len = (win_w * win_h * 4) as usize;
    if pixels_buf.len() < pixel_len {
        pixels_buf.resize(pixel_len, 0);
    } else {
        pixels_buf.truncate(pixel_len);
    }
    pixels_buf.fill(0);
    let pixels: &mut [u8] = &mut pixels_buf;
    let entries: [(bool, &str); 3] = [
        (badges.recording, badges.recording_icon.as_str()),
        (badges.buffer, badges.buffer_icon.as_str()),
        (badges.mic, badges.mic_icon.as_str()),
    ];
    let mut x = 0;
    for (on, icon_key) in entries {
        if !on {
            continue;
        }
        draw_badge(pixels, win_w, win_h, x, icon_key);
        x += BADGE + GAP;
    }

    // Premultiply RGB by alpha — required for `ULW_ALPHA` compositing.
    for px in pixels.chunks_exact_mut(4) {
        let a = px[3] as u32;
        px[0] = ((px[0] as u32 * a) / 255) as u8;
        px[1] = ((px[1] as u32 * a) / 255) as u8;
        px[2] = ((px[2] as u32 * a) / 255) as u8;
    }

    unsafe {
        let hdc_screen = windows::Win32::Graphics::Gdi::GetDC(None);
        let hdc_mem = CreateCompatibleDC(hdc_screen);

        let mut bmi: BITMAPINFO = std::mem::zeroed();
        bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bmi.bmiHeader.biWidth = win_w;
        bmi.bmiHeader.biHeight = -win_h;
        bmi.bmiHeader.biPlanes = 1;
        bmi.bmiHeader.biBitCount = 32;
        bmi.bmiHeader.biCompression = BI_RGB.0;

        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        if let Ok(hbitmap) = CreateDIBSection(hdc_screen, &bmi, DIB_RGB_COLORS, &mut bits, None, 0) {
            if !bits.is_null() {
                std::ptr::copy_nonoverlapping(pixels.as_ptr(), bits as *mut u8, pixels.len());
            }
            let old_bmp = SelectObject(hdc_mem, HGDIOBJ(hbitmap.0));

            let dst_pt = POINT { x: win_x, y: win_y };
            let size = SIZE { cx: win_w, cy: win_h };
            let src_pt = POINT { x: 0, y: 0 };
            let blend = windows::Win32::Graphics::Gdi::BLENDFUNCTION {
                BlendOp: windows::Win32::Graphics::Gdi::AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: 255,
                AlphaFormat: windows::Win32::Graphics::Gdi::AC_SRC_ALPHA as u8,
            };
            let _ = UpdateLayeredWindow(hwnd, hdc_screen, Some(&dst_pt), Some(&size), hdc_mem, Some(&src_pt), COLORREF(0), Some(&blend), ULW_ALPHA);

            SelectObject(hdc_mem, old_bmp);
            let _ = DeleteObject(HGDIOBJ(hbitmap.0));
        }
        let _ = DeleteDC(hdc_mem);
        windows::Win32::Graphics::Gdi::ReleaseDC(None, hdc_screen);
        let _ = SetWindowPos(hwnd, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
    }
}

/// One badge: a `rgba(0,0,0,0.7)` circle, its icon centered in white.
fn draw_badge(pixels: &mut [u8], win_w: i32, win_h: i32, x_off: i32, icon_key: &str) {
    let cx = x_off + BADGE / 2;
    let cy = BADGE / 2;
    let r = BADGE / 2;
    let bg_alpha = (255.0 * 0.7) as u8;
    for dy in -r..r {
        for dx in -r..r {
            if dx * dx + dy * dy > r * r {
                continue;
            }
            let px = cx + dx;
            let py = cy + dy;
            if px < 0 || px >= win_w || py < 0 || py >= win_h {
                continue;
            }
            let i = ((py * win_w + px) * 4) as usize;
            pixels[i] = 0;
            pixels[i + 1] = 0;
            pixels[i + 2] = 0;
            pixels[i + 3] = bg_alpha;
        }
    }
    let Some(icon) = icon_bitmap(icon_key) else { return };
    let icon_x = x_off + (BADGE - ICON_PX as i32) / 2;
    let icon_y = (BADGE - ICON_PX as i32) / 2;
    for iy in 0..ICON_PX as i32 {
        let py = icon_y + iy;
        if py < 0 || py >= win_h {
            continue;
        }
        for ix in 0..ICON_PX as i32 {
            let px = icon_x + ix;
            if px < 0 || px >= win_w {
                continue;
            }
            let src = icon.get_pixel(ix as u32, iy as u32).0;
            let src_a = src[3] as u32;
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
}
