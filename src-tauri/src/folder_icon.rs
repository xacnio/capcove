//! Sets a custom Windows Explorer icon (via `desktop.ini`'s `[.ShellClassInfo]`)
//! on a game's top-level recordings folder: a folder glyph with the game's own
//! icon badged on top. Best-effort — any failure just leaves the default icon.

use std::path::Path;

/// Flat-fills an axis-aligned rounded rectangle (`[x0,y0]`–`[x1,y1]`, corner
/// radius `r`, in pixels) directly into `canvas`, pixel by pixel.
#[cfg(windows)]
fn fill_rounded_rect(canvas: &mut image::RgbaImage, x0: f32, y0: f32, x1: f32, y1: f32, r: f32, color: [u8; 4]) {
    let (w, h) = (canvas.width(), canvas.height());
    for py in 0..h {
        for px in 0..w {
            let (fx, fy) = (px as f32 + 0.5, py as f32 + 0.5);
            if fx < x0 || fx > x1 || fy < y0 || fy > y1 {
                continue;
            }
            let corner_x = if fx < x0 + r { Some(x0 + r) } else if fx > x1 - r { Some(x1 - r) } else { None };
            let corner_y = if fy < y0 + r { Some(y0 + r) } else if fy > y1 - r { Some(y1 - r) } else { None };
            if let (Some(cx), Some(cy)) = (corner_x, corner_y) {
                let (dx, dy) = (fx - cx, fy - cy);
                if dx * dx + dy * dy > r * r {
                    continue;
                }
            }
            canvas.put_pixel(px, py, image::Rgba(color));
        }
    }
}

/// The system's generic folder icon at `size`×`size`, via `SHGetStockIconInfo`
/// + `PrivateExtractIconsW`. Not `SHGetFileInfoW` on `folder` itself, since
/// once it has our custom icon that would just return it recursively.
#[cfg(windows)]
fn stock_folder_icon_png(size: i32) -> Option<Vec<u8>> {
    use windows::Win32::UI::Shell::{SHGetStockIconInfo, SHGSI_ICONLOCATION, SHSTOCKICONINFO, SIID_FOLDER};
    use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, PrivateExtractIconsW, HICON};

    let mut info: SHSTOCKICONINFO = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of::<SHSTOCKICONINFO>() as u32;
    unsafe { SHGetStockIconInfo(SIID_FOLDER, SHGSI_ICONLOCATION, &mut info).ok()? };

    let mut path_buf = [0u16; 260];
    let n = info.szPath.len().min(259);
    path_buf[..n].copy_from_slice(&info.szPath[..n]);

    let mut icons = [HICON::default()];
    let mut icon_id = 0u32;
    let got = unsafe {
        PrivateExtractIconsW(&path_buf, info.iIcon, size, size, Some(&mut icons), Some(&mut icon_id), 0)
    };
    if got < 1 || icons[0].is_invalid() {
        return None;
    }
    let png = crate::icon_cache::render_hicon_to_png(icons[0], size);
    unsafe { let _ = DestroyIcon(icons[0]); }
    png
}

/// Flat folder glyph drawn by hand, used only if `stock_folder_icon_png`
/// fails, so the feature still degrades to something folder-shaped.
#[cfg(windows)]
fn fallback_folder_canvas(size: u32) -> image::RgbaImage {
    let s = size as f32;
    let mut canvas = image::RgbaImage::new(size, size);
    const DARK: [u8; 4] = [214, 152, 42, 255];
    const LIGHT: [u8; 4] = [255, 200, 87, 255];
    fill_rounded_rect(&mut canvas, s * 0.07, s * 0.19, s * 0.45, s * 0.34, s * 0.05, DARK);
    fill_rounded_rect(&mut canvas, s * 0.07, s * 0.29, s * 0.93, s * 0.87, s * 0.07, DARK);
    fill_rounded_rect(&mut canvas, s * 0.07, s * 0.45, s * 0.93, s * 0.87, s * 0.07, LIGHT);
    canvas
}

/// The system folder icon (or `fallback_folder_canvas`) at `size`×`size`,
/// with the game's icon centered on top as a badge.
#[cfg(windows)]
fn folder_with_badge(app_icon_bytes: &[u8], size: u32) -> Option<image::RgbaImage> {
    let stock = stock_folder_icon_png(size as i32).and_then(|png| image::load_from_memory(&png).ok());
    if stock.is_none() && size == 256 {
        log::warn!("folder_icon: couldn't get the system folder icon (SHGetStockIconInfo/PrivateExtractIconsW) — using the hand-drawn fallback shape instead");
    }
    let mut canvas = stock.map(|img| img.to_rgba8()).unwrap_or_else(|| fallback_folder_canvas(size));

    let s = size as f32;
    let app_img = match image::load_from_memory(app_icon_bytes) {
        Ok(img) => img.to_rgba8(),
        Err(e) => {
            log::warn!("folder_icon: couldn't decode the game's icon bytes as an image: {e}");
            return None;
        }
    };
    let badge_size = (s * 0.5).round().max(1.0) as u32;
    let badge = image::imageops::resize(&app_img, badge_size, badge_size, image::imageops::FilterType::Lanczos3);
    let bx = ((s - badge_size as f32) / 2.0).round() as i64;
    let by = (s * 0.62 - badge_size as f32 / 2.0).round() as i64;
    image::imageops::overlay(&mut canvas, &badge, bx, by);
    Some(canvas)
}

/// Builds the multi-resolution `.ico` that `desktop.ini`'s `IconResource`
/// needs, drawing each frame fresh so corner radii stay crisp at every size.
#[cfg(windows)]
fn to_ico(app_icon_bytes: &[u8]) -> Option<Vec<u8>> {
    use image::codecs::ico::{IcoEncoder, IcoFrame};
    use image::ExtendedColorType;

    let mut frames = Vec::new();
    for size in [16u32, 32, 48, 256] {
        let frame = folder_with_badge(app_icon_bytes, size)?;
        frames.push(IcoFrame::as_png(frame.as_raw(), size, size, ExtendedColorType::Rgba8).ok()?);
    }
    let mut out = Vec::new();
    IcoEncoder::new(&mut out).encode_images(&frames).ok()?;
    Some(out)
}

/// Writes `.icon.ico` + `desktop.ini` into `folder` and flags both (plus the
/// folder itself) so Explorer picks it up. Skips all of it if `desktop.ini`
/// already exists, so it's cheap to call on every recording start.
#[cfg(windows)]
pub fn ensure_folder_icon(folder: &Path, icon_bytes: &[u8]) {
    let ini_path = folder.join("desktop.ini");
    if ini_path.exists() {
        return;
    }
    log::info!("folder_icon: setting a custom icon on {}", folder.display());
    let Some(ico_bytes) = to_ico(icon_bytes) else {
        log::warn!("folder_icon: encoding {} failed (bad/undecodable source icon?)", folder.display());
        return;
    };
    let ico_path = folder.join(".icon.ico");
    if let Err(e) = std::fs::write(&ico_path, &ico_bytes) {
        log::warn!("folder_icon: writing {} failed: {e}", ico_path.display());
        return;
    }
    if let Err(e) = std::fs::write(&ini_path, "[.ShellClassInfo]\r\nIconResource=.icon.ico,0\r\n") {
        log::warn!("folder_icon: writing {} failed: {e}", ini_path.display());
        return;
    }

    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{
        SetFileAttributesW, FILE_ATTRIBUTE_HIDDEN, FILE_ATTRIBUTE_READONLY, FILE_ATTRIBUTE_SYSTEM,
    };
    fn wide(p: &Path) -> Vec<u16> {
        p.as_os_str().encode_wide().chain(std::iter::once(0)).collect()
    }
    unsafe {
        let ico_w = wide(&ico_path);
        let _ = SetFileAttributesW(PCWSTR(ico_w.as_ptr()), FILE_ATTRIBUTE_HIDDEN | FILE_ATTRIBUTE_SYSTEM);
        let ini_w = wide(&ini_path);
        let _ = SetFileAttributesW(PCWSTR(ini_w.as_ptr()), FILE_ATTRIBUTE_HIDDEN | FILE_ATTRIBUTE_SYSTEM);
        // Explorer keys off Read-Only as the "customized" marker for folders
        // (harmless — it doesn't actually make the directory read-only).
        let folder_w = wide(folder);
        let _ = SetFileAttributesW(PCWSTR(folder_w.as_ptr()), FILE_ATTRIBUTE_READONLY);
    }

    // Without this, Explorer can keep showing the old/default icon for a
    // folder that's already open or cached until it's reopened.
    use windows::Win32::UI::Shell::{SHChangeNotify, SHCNE_UPDATEITEM, SHCNF_PATHW};
    unsafe {
        let folder_w = wide(folder);
        SHChangeNotify(SHCNE_UPDATEITEM, SHCNF_PATHW, Some(folder_w.as_ptr() as *const _), None);
    }
    log::info!("folder_icon: done — {} now has a custom icon", folder.display());
}

#[cfg(not(windows))]
pub fn ensure_folder_icon(_folder: &Path, _icon_bytes: &[u8]) {}
