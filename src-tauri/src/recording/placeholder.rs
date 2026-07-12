//! Renders the placeholder cards written into recordings while the capture
//! source can't (or shouldn't) be shown: a black frame with the Capcove
//! logo and a caption, drawn with GDI text plus an `image`-crate logo blit.

use std::sync::OnceLock;

use crate::config::MinimizedBehavior;

/// Why the captured window's content shouldn't be shown right now.
#[derive(Clone, Copy, PartialEq)]
pub enum Occlusion {
    None,
    Minimized,
    AltTabbed,
}

/// The captured window's occlusion state. `AltTabbed` is only reported when
/// the privacy toggle asks for it; our own overlays (shortcut wheel,
/// gallery) briefly taking focus never count as alt-tabbing away.
pub fn window_occlusion(window_hwnd: Option<u32>, alt_tab_privacy: bool) -> Occlusion {
    let Some(hwnd) = window_hwnd else { return Occlusion::None };
    if crate::capture::is_window_minimized(hwnd) {
        Occlusion::Minimized
    } else if alt_tab_privacy
        && !crate::capture::is_window_foreground(hwnd)
        && !crate::capture::is_foreground_own_process()
    {
        Occlusion::AltTabbed
    } else {
        Occlusion::None
    }
}

/// Card text per occlusion + configured minimized behavior. `None` = write
/// nothing special (real/last frame — or, for `Pause`, skip writing).
pub fn card_for(occlusion: Occlusion, minimized_behavior: MinimizedBehavior) -> Option<&'static str> {
    match occlusion {
        Occlusion::None => None,
        Occlusion::AltTabbed => Some("Alt-tabbed"),
        Occlusion::Minimized => match minimized_behavior {
            MinimizedBehavior::Branded => Some("Window minimized"),
            // Empty caption = plain black frame (logo skipped too).
            MinimizedBehavior::Black => Some(""),
            // No card → the writers fall back to repeating the last real
            // frame: a frozen picture while audio keeps running.
            MinimizedBehavior::Freeze => None,
            // Handled by the writer loops (they stop writing entirely).
            MinimizedBehavior::Pause => None,
        },
    }
}

/// Cheap "is this frame (near-)black?" test — samples ~2k pixels. Used to
/// swallow the black flash delivered right after a minimized window is
/// restored, so Freeze mode cuts straight to real content.
pub fn is_mostly_black(bgra: &[u8]) -> bool {
    let px = bgra.len() / 4;
    if px == 0 {
        return true;
    }
    let step = (px / 2048).max(1);
    let mut i = 0;
    while i < px {
        let o = i * 4;
        if bgra[o] > 24 || bgra[o + 1] > 24 || bgra[o + 2] > 24 {
            return false;
        }
        i += step;
    }
    true
}

const BG: [u8; 4] = [8, 8, 8, 255]; // BGRA near-black

fn logo_rgba() -> Option<&'static image::RgbaImage> {
    static LOGO: OnceLock<Option<image::RgbaImage>> = OnceLock::new();
    LOGO.get_or_init(|| {
        image::load_from_memory(include_bytes!("../../../src/assets/logo.png"))
            .ok()
            .map(|i| i.to_rgba8())
    })
    .as_ref()
}

/// A branded BGRA frame of exactly `width`×`height`: near-black background,
/// centered logo, `text` caption, dim "Capcove" wordmark below. Empty `text`
/// produces a plain black frame (the `MinimizedBehavior::Black` variant).
pub fn render(width: u32, height: u32, text: &str) -> Vec<u8> {
    let byte_len = (width as usize) * (height as usize) * 4;
    let mut buf = vec![0u8; byte_len];
    for px in buf.chunks_exact_mut(4) {
        px.copy_from_slice(&BG);
    }
    if width == 0 || height == 0 || text.is_empty() {
        return buf;
    }

    // Logo, centered slightly above the middle.
    let mut text_top = (height as i32) / 2;
    if let Some(logo) = logo_rgba() {
        let target_h = (height / 7).clamp(48, 200);
        let target_w = (logo.width() as u64 * target_h as u64 / logo.height().max(1) as u64) as u32;
        if target_w > 0 && target_w < width {
            let scaled = image::imageops::resize(logo, target_w, target_h, image::imageops::FilterType::Triangle);
            let ox = (width - target_w) as i32 / 2;
            let oy = (height as i32) / 2 - target_h as i32 + (target_h as i32 / 4);
            blit_over(&mut buf, width, height, &scaled, ox, oy);
            text_top = oy + target_h as i32 + (height as i32 / 30).max(18);
        }
    }

    // Caption + dim wordmark via GDI, composited onto the same buffer.
    if let Some(with_text) = draw_texts_gdi(&buf, width, height, text, text_top) {
        return with_text;
    }
    buf
}

/// Alpha-blends an RGBA image onto the BGRA frame at (ox, oy).
fn blit_over(buf: &mut [u8], fw: u32, fh: u32, img: &image::RgbaImage, ox: i32, oy: i32) {
    for (px, py, p) in img.enumerate_pixels() {
        let x = ox + px as i32;
        let y = oy + py as i32;
        if x < 0 || y < 0 || x >= fw as i32 || y >= fh as i32 {
            continue;
        }
        let a = p[3] as u32;
        if a == 0 {
            continue;
        }
        let idx = ((y as u32 * fw + x as u32) * 4) as usize;
        // src RGBA over dst BGRA
        buf[idx] = ((p[2] as u32 * a + buf[idx] as u32 * (255 - a)) / 255) as u8;
        buf[idx + 1] = ((p[1] as u32 * a + buf[idx + 1] as u32 * (255 - a)) / 255) as u8;
        buf[idx + 2] = ((p[0] as u32 * a + buf[idx + 2] as u32 * (255 - a)) / 255) as u8;
        buf[idx + 3] = 255;
    }
}

/// Draws the caption (and a dim wordmark under it) over a copy of `base`
/// using GDI, returning the composited frame.
fn draw_texts_gdi(base: &[u8], width: u32, height: u32, text: &str, text_top: i32) -> Option<Vec<u8>> {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{COLORREF, RECT};
    use windows::Win32::Graphics::Gdi::{
        CreateCompatibleDC, CreateDIBSection, CreateFontW, DeleteDC, DeleteObject, DrawTextW,
        GdiFlush, SelectObject, SetBkMode, SetTextColor, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
        CLEARTYPE_QUALITY, DEFAULT_CHARSET, DIB_RGB_COLORS, DT_CENTER, DT_SINGLELINE,
        FF_DONTCARE, FW_MEDIUM, FW_SEMIBOLD, HGDIOBJ, OUT_TT_PRECIS, TRANSPARENT,
    };

    unsafe {
        let dc = CreateCompatibleDC(None);
        if dc.is_invalid() {
            return None;
        }
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width as i32,
                biHeight: -(height as i32), // top-down
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let Ok(bitmap) = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, None, 0) else {
            let _ = DeleteDC(dc);
            return None;
        };
        if bits.is_null() {
            let _ = DeleteObject(bitmap);
            let _ = DeleteDC(dc);
            return None;
        }
        let old_bitmap = SelectObject(dc, bitmap);

        let byte_len = base.len();
        let pixels = std::slice::from_raw_parts_mut(bits as *mut u8, byte_len);
        pixels.copy_from_slice(base);

        let face: Vec<u16> = "Segoe UI".encode_utf16().chain(std::iter::once(0)).collect();
        let mk_font = |h: i32, weight: i32| {
            CreateFontW(
                h, 0, 0, 0, weight,
                0, 0, 0, DEFAULT_CHARSET.0.into(), OUT_TT_PRECIS.0.into(),
                0, CLEARTYPE_QUALITY.0.into(), FF_DONTCARE.0.into(), PCWSTR(face.as_ptr()),
            )
        };
        let _ = SetBkMode(dc, TRANSPARENT);

        // Caption.
        let caption_h = ((height as i32) / 22).max(20);
        let font = mk_font(caption_h, FW_SEMIBOLD.0 as i32);
        let old_font = SelectObject(dc, HGDIOBJ(font.0));
        let _ = SetTextColor(dc, COLORREF(0x00E8E8E8));
        let mut wtext: Vec<u16> = text.encode_utf16().collect();
        let mut rect = RECT { left: 0, top: text_top, right: width as i32, bottom: text_top + caption_h * 2 };
        DrawTextW(dc, &mut wtext, &mut rect, DT_CENTER | DT_SINGLELINE);

        // Dim wordmark below.
        let brand_h = (caption_h * 2 / 3).max(14);
        let brand_font = mk_font(brand_h, FW_MEDIUM.0 as i32);
        SelectObject(dc, HGDIOBJ(brand_font.0));
        let _ = SetTextColor(dc, COLORREF(0x00575757));
        let mut wbrand: Vec<u16> = "CAPCOVE".encode_utf16().collect();
        let brand_top = text_top + caption_h + (caption_h / 2);
        let mut brand_rect = RECT { left: 0, top: brand_top, right: width as i32, bottom: brand_top + brand_h * 2 };
        DrawTextW(dc, &mut wbrand, &mut brand_rect, DT_CENTER | DT_SINGLELINE);

        let _ = GdiFlush();
        let out = pixels.to_vec();

        SelectObject(dc, old_font);
        SelectObject(dc, old_bitmap);
        let _ = DeleteObject(HGDIOBJ(font.0));
        let _ = DeleteObject(HGDIOBJ(brand_font.0));
        let _ = DeleteObject(bitmap);
        let _ = DeleteDC(dc);
        Some(out)
    }
}
