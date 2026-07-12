use std::path::{Path, PathBuf};

pub struct IconCache {
    pub dir: PathBuf,
    /// Catalog art (fetched icon/cover) for a game not yet in this build's
    /// embedded icon/cover pack. Kept outside `dir` since it isn't user data
    /// (re-fetchable on any install) and `sync.rs`'s Drive backup only reads `dir`.
    pub catalog_dir: PathBuf,
}

fn sanitize(app: &str) -> String {
    app.chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect()
}

impl IconCache {
    pub fn new(config_dir: &Path) -> Self {
        let dir = config_dir.join("icon_cache");
        let _ = std::fs::create_dir_all(&dir);
        let catalog_dir = config_dir.join("icon_cache_catalog");
        let _ = std::fs::create_dir_all(&catalog_dir);
        Self { dir, catalog_dir }
    }

    fn cache_path(&self, app: &str) -> PathBuf {
        self.dir.join(format!("{}.png", sanitize(app)))
    }

    fn catalog_cache_path(&self, app: &str) -> PathBuf {
        self.catalog_dir.join(format!("{}.png", sanitize(app)))
    }

    pub fn get_base64(&self, app: &str) -> Option<String> {
        use base64::Engine;
        Some(base64::engine::general_purpose::STANDARD.encode(self.get_bytes(app)?))
    }

    /// Same as `get_base64`, without the base64 round-trip — for a caller
    /// that wants to re-encode the icon into something else (a `.ico`, for
    /// `folder_icon::set_folder_icon`) rather than hand it to the frontend.
    pub fn get_bytes(&self, app: &str) -> Option<Vec<u8>> {
        std::fs::read(self.cache_path(app)).ok()
    }

    pub fn has(&self, app: &str) -> bool {
        self.cache_path(app).exists()
    }

    /// Stores an externally-sourced icon (e.g. game art from the games db).
    /// Deliberately overwrites any exe-extracted fallback for the same app —
    /// catalog art is the nicer of the two.
    pub fn store_png(&self, app: &str, bytes: &[u8]) {
        if app.is_empty() || bytes.is_empty() {
            return;
        }
        let _ = std::fs::write(self.cache_path(app), bytes);
    }

    /// Same shape as `get_base64`/`has`/`store_png`, for catalog art not yet
    /// in the embedded pack — see the `catalog_dir` doc comment for why this
    /// is a separate directory instead of just another key in `dir`.
    pub fn get_catalog_base64(&self, app: &str) -> Option<String> {
        use base64::Engine;
        Some(base64::engine::general_purpose::STANDARD.encode(self.get_catalog_bytes(app)?))
    }

    /// Raw-bytes counterpart of `get_catalog_base64` — see `get_bytes`.
    pub fn get_catalog_bytes(&self, app: &str) -> Option<Vec<u8>> {
        std::fs::read(self.catalog_cache_path(app)).ok()
    }

    pub fn has_catalog(&self, app: &str) -> bool {
        self.catalog_cache_path(app).exists()
    }

    pub fn store_catalog_png(&self, app: &str, bytes: &[u8]) {
        if app.is_empty() || bytes.is_empty() {
            return;
        }
        let _ = std::fs::write(self.catalog_cache_path(app), bytes);
    }

    /// Extracts the icon from a window handle and caches it to disk.
    #[allow(unused_variables)]
    pub fn cache_from_hwnd(&self, app: &str, hwnd_u32: u32) {
        if app.is_empty() || self.has(app) {
            return;
        }
        #[cfg(windows)]
        if let Some(png) = extract_png_from_hwnd(hwnd_u32) {
            let _ = std::fs::write(self.cache_path(app), png);
        }
        #[cfg(target_os = "macos")]
        if let Some(png) = extract_png_from_macos_app(app) {
            let _ = std::fs::write(self.cache_path(app), png);
        }
        #[cfg(target_os = "linux")]
        if let Some(png) = extract_png_from_x11(hwnd_u32) {
            let _ = std::fs::write(self.cache_path(app), png);
        }
    }
}

// Windows icon extraction

#[cfg(windows)]
fn extract_png_from_hwnd(hwnd_u32: u32) -> Option<Vec<u8>> {
    use windows::Win32::Foundation::{CloseHandle, FALSE, HWND, LPARAM, WPARAM};
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GetClassLongPtrW, GetWindowThreadProcessId, SendMessageW, HICON, WM_GETICON,
    };
    use windows::core::PWSTR;

    unsafe {
        let hwnd = HWND(hwnd_u32 as usize as *mut _);

        // Ask via WM_GETICON (ICON_BIG = 1) first
        let res = SendMessageW(hwnd, WM_GETICON, WPARAM(1), LPARAM(0));
        let h = HICON(res.0 as *mut _);
        if !h.is_invalid() {
            if let Some(png) = render_hicon_to_png(h, 32) {
                return Some(png);
            }
        }

        // Class icon (GCLP_HICON = -14)
        let lp = GetClassLongPtrW(hwnd, windows::Win32::UI::WindowsAndMessaging::GET_CLASS_LONG_INDEX(-14i32));
        let h = HICON(lp as *mut _);
        if !h.is_invalid() {
            if let Some(png) = render_hicon_to_png(h, 32) {
                return Some(png);
            }
        }

        // Fall back to getting the icon from the exe via SHGetFileInfoW
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 {
            return None;
        }
        let proc = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, pid).ok()?;
        let mut buf = [0u16; 512];
        let mut size = buf.len() as u32;
        let res = QueryFullProcessImageNameW(proc, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut size);
        let _ = CloseHandle(proc);
        res.ok()?;
        let exe_path = String::from_utf16_lossy(&buf[..size as usize]);

        extract_png_from_exe_path(&exe_path)
    }
}

/// Also called directly by `commands::games::inspect_exe_file` for the
/// "add custom game" file-picker flow, ahead of any game actually being
/// added/cached.
#[cfg(windows)]
pub(crate) fn extract_png_from_exe_path(exe_path: &str) -> Option<Vec<u8>> {
    use windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_NORMAL;
    use windows::Win32::UI::Shell::{SHGetFileInfoW, SHFILEINFOW, SHGFI_FLAGS};
    use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, PrivateExtractIconsW, HICON};
    use windows::core::PCWSTR;

    let wide: Vec<u16> = exe_path.encode_utf16().chain([0u16]).collect();

    let mut hicon = unsafe {
        let mut shfi: SHFILEINFOW = std::mem::zeroed();
        let r = SHGetFileInfoW(
            PCWSTR(wide.as_ptr()),
            FILE_ATTRIBUTE_NORMAL,
            Some(&mut shfi),
            std::mem::size_of::<SHFILEINFOW>() as u32,
            SHGFI_FLAGS(0x100), // SHGFI_ICON | SHGFI_LARGEICON(0)
        );
        if r != 0 && !shfi.hIcon.is_invalid() {
            Some(shfi.hIcon)
        } else {
            None
        }
    };

    // SHGetFileInfoW goes through the shell and can silently come back empty
    // in some contexts; PrivateExtractIconsW reads the PE's icon resource
    // directly instead, bypassing the shell/COM.
    if hicon.is_none() {
        unsafe {
            let mut icons = [HICON::default()];
            let mut icon_id = 0u32;
            // Wants a fixed MAX_PATH buffer, not just a pointer.
            let mut path_buf = [0u16; 260];
            let n = wide.len().min(259);
            path_buf[..n].copy_from_slice(&wide[..n]);
            let got = PrivateExtractIconsW(&path_buf, 0, 32, 32, Some(&mut icons), Some(&mut icon_id), 0);
            if got >= 1 && !icons[0].is_invalid() {
                hicon = Some(icons[0]);
            }
        }
    }

    let hicon = hicon?;
    let result = render_hicon_to_png(hicon, 32);
    unsafe { let _ = DestroyIcon(hicon); }
    result
}

/// Renders an `HICON` to a `size`×`size` PNG. `pub(crate)` beyond this
/// module for `folder_icon::stock_folder_icon_png`, which needs a bigger
/// size (256) than anything extracted here ever asks for (32).
#[cfg(windows)]
pub(crate) fn render_hicon_to_png(hicon: windows::Win32::UI::WindowsAndMessaging::HICON, size: i32) -> Option<Vec<u8>> {
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Graphics::Gdi::{
        CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject,
        HGDIOBJ, SelectObject, BITMAPINFO, BITMAPINFOHEADER, DIB_USAGE, RGBQUAD,
    };
    use windows::Win32::UI::WindowsAndMessaging::{DI_FLAGS, DrawIconEx};

    let size = size.max(1);

    unsafe {
        let dc = CreateCompatibleDC(None);
        if dc.0.is_null() {
            return None;
        }

        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: size,
                biHeight: -size, // negative = top-down
                biPlanes: 1,
                biBitCount: 32,
                biCompression: 0, // BI_RGB
                biSizeImage: 0,
                biXPelsPerMeter: 0,
                biYPelsPerMeter: 0,
                biClrUsed: 0,
                biClrImportant: 0,
            },
            bmiColors: [RGBQUAD::default()],
        };

        let mut bits_ptr: *mut std::ffi::c_void = std::ptr::null_mut();
        let hbm = match CreateDIBSection(
            dc,
            &bmi,
            DIB_USAGE(0), // DIB_RGB_COLORS
            &mut bits_ptr,
            HANDLE::default(),
            0,
        ) {
            Ok(h) => h,
            Err(_) => {
                let _ = DeleteDC(dc);
                return None;
            }
        };

        if bits_ptr.is_null() {
            let _ = DeleteObject(HGDIOBJ(hbm.0 as *mut _));
            let _ = DeleteDC(dc);
            return None;
        }

        let old = SelectObject(dc, HGDIOBJ(hbm.0 as *mut _));
        let _ = DrawIconEx(dc, 0, 0, hicon, size, size, 0, None, DI_FLAGS(3)); // DI_NORMAL = 3

        let pixel_count = (size * size) as usize;
        let pixels_bgra = std::slice::from_raw_parts(bits_ptr as *const u8, pixel_count * 4);

        // If there is no alpha channel (legacy-style icon), make all pixels opaque
        let has_alpha = pixels_bgra.chunks_exact(4).any(|c| c[3] != 0);

        let mut pixels_rgba = vec![0u8; pixel_count * 4];
        for (i, chunk) in pixels_bgra.chunks_exact(4).enumerate() {
            pixels_rgba[i * 4] = chunk[2];     // R
            pixels_rgba[i * 4 + 1] = chunk[1]; // G
            pixels_rgba[i * 4 + 2] = chunk[0]; // B
            pixels_rgba[i * 4 + 3] = if has_alpha { chunk[3] } else { 255 };
        }

        let _ = SelectObject(dc, old);
        let _ = DeleteObject(HGDIOBJ(hbm.0 as *mut _));
        let _ = DeleteDC(dc);

        let img = image::ImageBuffer::<image::Rgba<u8>, Vec<u8>>::from_raw(
            size as u32,
            size as u32,
            pixels_rgba,
        )?;
        let mut buf = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png)
            .ok()?;
        Some(buf)
    }
}

#[cfg(not(windows))]
pub(crate) fn extract_png_from_exe_path(_exe_path: &str) -> Option<Vec<u8>> {
    None
}

/// Locates an app's `.app` bundle by name, probing standard install
/// locations before falling back to Spotlight (`mdfind`).
#[cfg(target_os = "macos")]
fn find_macos_app_bundle(app_name: &str) -> Option<String> {
    let home = std::env::var("HOME").unwrap_or_default();
    // Direct path probing first — doesn't depend on Spotlight indexing,
    // which can be disabled or incomplete (common in fresh VMs).
    let mut candidates = vec![
        format!("/Applications/{app_name}.app"),
        format!("/System/Applications/{app_name}.app"),
        format!("/System/Applications/Utilities/{app_name}.app"),
        format!("/Applications/Utilities/{app_name}.app"),
    ];
    if !home.is_empty() {
        candidates.push(format!("{home}/Applications/{app_name}.app"));
    }
    if let Some(path) = candidates.into_iter().find(|p| std::path::Path::new(p).exists()) {
        return Some(path);
    }

    // Fall back to Spotlight for apps in non-standard locations.
    let mut search_dirs = vec!["/Applications".to_string(), "/System/Applications".to_string()];
    if !home.is_empty() {
        search_dirs.push(format!("{home}/Applications"));
    }
    let mut mdfind = std::process::Command::new("mdfind");
    for dir in &search_dirs {
        mdfind.arg("-onlyin").arg(dir);
    }
    mdfind.arg("-name").arg(format!("{app_name}.app"));
    let output = mdfind.output().ok()?;
    let app_path = String::from_utf8_lossy(&output.stdout)
        .lines()
        .next()?
        .trim()
        .to_string();
    if app_path.is_empty() { None } else { Some(app_path) }
}

/// macOS: renders the app's icon via `qlmanage` (the same QuickLook service
/// Finder uses for icon previews).
#[cfg(target_os = "macos")]
fn extract_png_from_macos_app(app_name: &str) -> Option<Vec<u8>> {
    let app_path = find_macos_app_bundle(app_name)?;

    let tmp_dir = std::env::temp_dir().join(format!("capcove_icon_{}_{}", std::process::id(), app_name.len()));
    std::fs::create_dir_all(&tmp_dir).ok()?;

    let mut child = std::process::Command::new("qlmanage")
        .args(["-t", "-s", "128", "-o"])
        .arg(&tmp_dir)
        .arg(&app_path)
        .spawn()
        .ok();

    // `qlmanage` can hang indefinitely for some app bundles, so poll with a
    // deadline instead of a plain `.wait()`/`.status()`.
    let status = child.as_mut().and_then(|c| {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            match c.try_wait() {
                Ok(Some(status)) => return Some(status),
                Ok(None) if std::time::Instant::now() < deadline => {
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
                _ => {
                    let _ = c.kill();
                    let _ = c.wait();
                    return None;
                }
            }
        }
    });

    let result = status.filter(|s| s.success()).and_then(|_| {
        let base = std::path::Path::new(&app_path).file_name()?.to_str()?;
        std::fs::read(tmp_dir.join(format!("{base}.png"))).ok()
    });
    let _ = std::fs::remove_dir_all(&tmp_dir);
    result
}

#[cfg(target_os = "linux")]
fn extract_png_from_x11(window_id: u32) -> Option<Vec<u8>> {
    let output = std::process::Command::new("xprop")
        .args(&["-id", &window_id.to_string(), "-notype", "32c", "_NET_WM_ICON"])
        .output()
        .ok()?;
    
    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parts: Vec<&str> = stdout.split('=').collect();
    if parts.len() < 2 {
        return None;
    }

    let values_str = parts[1].trim().replace("\n", "");
    if values_str.is_empty() {
        return None;
    }

    let numbers: Vec<u32> = values_str
        .split(',')
        .map(|s| s.trim().parse::<i64>().unwrap_or(0) as u32)
        .collect();

    let mut best_index = None;
    let mut best_score = -1;
    let mut idx = 0;
    
    while idx < numbers.len() {
        if idx + 2 > numbers.len() {
            break;
        }
        let w = numbers[idx];
        let h = numbers[idx + 1];
        if w == 0 || h == 0 || w > 512 || h > 512 {
            break;
        }
        let pixel_count = (w * h) as usize;
        if idx + 2 + pixel_count > numbers.len() {
            break;
        }
        
        let size = w.min(h);
        let score = if size == 48 {
            100
        } else if size == 32 {
            90
        } else if size == 64 {
            85
        } else if size > 16 && size < 128 {
            70
        } else if size == 16 {
            50
        } else {
            30
        };
        
        if score > best_score {
            best_score = score;
            best_index = Some((idx, w, h));
        }
        
        idx += 2 + pixel_count;
    }

    let (start_idx, w, h) = match best_index {
        Some(v) => v,
        None => return None,
    };

    let pixel_count = (w * h) as usize;
    let mut pixels = Vec::with_capacity(pixel_count * 4);
    
    for i in 0..pixel_count {
        let argb = numbers[start_idx + 2 + i];
        let a = ((argb >> 24) & 0xFF) as u8;
        let r = ((argb >> 16) & 0xFF) as u8;
        let g = ((argb >> 8) & 0xFF) as u8;
        let b = (argb & 0xFF) as u8;
        pixels.push(r);
        pixels.push(g);
        pixels.push(b);
        pixels.push(a);
    }

    let img = image::ImageBuffer::<image::Rgba<u8>, Vec<u8>>::from_raw(w, h, pixels)?;

    let mut buf = Vec::new();
    if img.write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png).is_err() {
        return None;
    }

    Some(buf)
}
