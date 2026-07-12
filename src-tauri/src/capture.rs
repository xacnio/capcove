use anyhow::{anyhow, Context, Result};
use image::RgbaImage;
use xcap::Monitor;

/// Captures a monitor's image for the picker overlay backdrop only (not a
/// saved product), so the plain `xcap` path is fine on every platform.
fn monitor_capture_image(monitor: &Monitor) -> Result<RgbaImage> {
    monitor.capture_image().context("failed to capture monitor image")
}

/// Full image of a monitor + position/scale information.
#[allow(dead_code)]
pub struct MonitorShot {
    pub image: RgbaImage,
    pub scale: f32,
    /// Monitor position in screen coordinates (physical pixels)
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

pub fn capture_monitor_at(px: i32, py: i32) -> Result<MonitorShot> {
    let monitor = monitor_at(px, py)?;
    let image = monitor_capture_image(&monitor)?;
    let scale = monitor.scale_factor().unwrap_or(1.0);
    let x = monitor.x().unwrap_or(0);
    let y = monitor.y().unwrap_or(0);
    let width = monitor.width().unwrap_or_else(|_| image.width());
    let height = monitor.height().unwrap_or_else(|_| image.height());
    Ok(MonitorShot {
        image,
        scale,
        x,
        y,
        width,
        height,
    })
}

pub fn list_monitors() -> Vec<crate::MonitorInfo> {
    let mut monitors: Vec<crate::MonitorInfo> = Monitor::all().unwrap_or_default().into_iter().map(|m| crate::MonitorInfo {
        x: m.x().unwrap_or(0),
        y: m.y().unwrap_or(0),
        w: m.width().unwrap_or(1920),
        h: m.height().unwrap_or(1080),
        scale: m.scale_factor().unwrap_or(1.0),
    }).collect();
    // Sort left-to-right, top-to-bottom so Monitor 1 is always the leftmost monitor.
    monitors.sort_by_key(|m| (m.x, m.y));
    monitors
}

pub fn monitor_at(px: i32, py: i32) -> Result<Monitor> {
    if let Ok(m) = Monitor::from_point(px, py) {
        return Ok(m);
    }
    Monitor::all()
        .context("failed to list monitors")?
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("no monitor found"))
}

/// Information about a visible, top-level window (physical pixel coordinates).
#[derive(Clone)]
pub struct WinInfo {
    pub id: u32,
    pub title: String,
    pub app: String,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// Returns the executable name (without extension) for a process id.
#[cfg(windows)]
pub fn exe_for_pid(pid: u32) -> Option<String> {
    let path = exe_path_for_pid(pid)?;
    std::path::Path::new(&path)
        .file_stem()
        .and_then(|s| s.to_str())
        .map(|s| s.to_string())
}

/// Full image path of a process — game detection needs it to match the
/// catalog's path-qualified executable entries ("dir/name.exe"), which
/// can't be checked against a bare stem.
#[cfg(windows)]
pub fn exe_path_for_pid(pid: u32) -> Option<String> {
    use windows::core::PWSTR;
    use windows::Win32::Foundation::{CloseHandle, FALSE};
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };
    if pid == 0 {
        return None;
    }
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, pid).ok()?;
        let mut buf = [0u16; 512];
        let mut size = buf.len() as u32;
        let res = QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut size);
        let _ = CloseHandle(handle);
        res.ok()?;
        Some(String::from_utf16_lossy(&buf[..size as usize]))
    }
}

/// Full image path of a window's owning process — see `exe_path_for_pid`.
#[cfg(windows)]
pub fn window_exe_path(hwnd_u32: u32) -> Option<String> {
    exe_path_for_pid(pid_for_hwnd(hwnd_u32)?)
}

/// Owning process id of a window.
#[cfg(windows)]
pub fn pid_for_hwnd(hwnd_u32: u32) -> Option<u32> {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;
    unsafe {
        let mut pid = 0u32;
        GetWindowThreadProcessId(HWND(hwnd_u32 as usize as *mut _), Some(&mut pid));
        (pid != 0).then_some(pid)
    }
}

/// The process id that actually produces audio for `hwnd_u32`, for per-process
/// capture. Prefers `pid_for_hwnd`'s direct answer, but for a window hosted by
/// `ApplicationFrameHost.exe` (every UWP/MSIX-packaged app — the host owns the
/// visible top-level window, but runs the app itself as a *separate*, unrelated
/// process, not a child of the host, so `INCLUDE_TARGET_PROCESS_TREE` loopback
/// capture on the host's pid never sees the app's real audio) it instead
/// resolves the inner `Windows.UI.Core.CoreWindow` child, which belongs to the
/// actual app process.
#[cfg(windows)]
pub fn audio_pid_for_hwnd(hwnd_u32: u32) -> Option<u32> {
    let direct_pid = pid_for_hwnd(hwnd_u32)?;
    if exe_for_pid(direct_pid).as_deref() != Some("ApplicationFrameHost") {
        return Some(direct_pid);
    }
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::FindWindowExW;
    unsafe {
        let hwnd = HWND(hwnd_u32 as usize as *mut _);
        let core_window = FindWindowExW(hwnd, None, windows::core::w!("Windows.UI.Core.CoreWindow"), None).ok()?;
        pid_for_hwnd(core_window.0 as usize as u32).or(Some(direct_pid))
    }
}

#[cfg(not(windows))]
pub fn audio_pid_for_hwnd(hwnd_u32: u32) -> Option<u32> {
    pid_for_hwnd(hwnd_u32)
}

/// Returns the executable name (without extension) of a window.
#[cfg(windows)]
fn window_exe(hwnd: windows::Win32::Foundation::HWND) -> Option<String> {
    use windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;
    unsafe {
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        exe_for_pid(pid)
    }
}

#[cfg(windows)]
fn window_title(hwnd: windows::Win32::Foundation::HWND) -> Option<String> {
    use windows::Win32::UI::WindowsAndMessaging::{GetWindowTextLengthW, GetWindowTextW};
    unsafe {
        let len = GetWindowTextLengthW(hwnd);
        if len <= 0 {
            return None;
        }
        let mut buf = vec![0u16; len as usize + 1];
        let n = GetWindowTextW(hwnd, &mut buf);
        let t = String::from_utf16_lossy(&buf[..n as usize]);
        if t.trim().is_empty() {
            None
        } else {
            Some(t)
        }
    }
}

/// (title, exe-derived app name) for a raw HWND — u32 flavor of the private
/// helpers above, for callers outside this module (game detection).
#[cfg(windows)]
pub fn window_info(hwnd_u32: u32) -> (Option<String>, Option<String>) {
    let hwnd = windows::Win32::Foundation::HWND(hwnd_u32 as usize as *mut _);
    (window_title(hwnd), window_exe(hwnd))
}

/// Whether the window is currently minimized (iconic). Used by the encoder
/// writer loops to choose between repeating the last frame (WGC only
/// delivers frames on change) and showing the "minimized" placeholder card.
#[cfg(windows)]
pub fn is_window_minimized(hwnd_u32: u32) -> bool {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::IsIconic;
    unsafe { IsIconic(HWND(hwnd_u32 as usize as *mut _)).as_bool() }
}

/// Whether the window is the current foreground window — the writer loops'
/// "alt-tabbed" privacy card keys off this.
#[cfg(windows)]
pub fn is_window_foreground(hwnd_u32: u32) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
    unsafe { GetForegroundWindow().0 as usize as u32 == hwnd_u32 }
}

/// Whether the current foreground window belongs to Capcove itself (the
/// shortcut wheel, the gallery, …). Our own transient overlays taking focus
/// must not count as the game being alt-tabbed.
#[cfg(windows)]
pub fn is_foreground_own_process() -> bool {
    use windows::Win32::System::Threading::GetCurrentProcessId;
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_invalid() {
            return false;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        pid == GetCurrentProcessId()
    }
}

/// Returns a window's visual bounds (x, y, width, height) in physical pixels,
/// using DWM's extended frame bounds to strip the invisible shadow margin
/// `GetWindowRect` includes.
#[cfg(windows)]
pub fn window_frame_rect(hwnd: u32) -> Option<(i32, i32, u32, u32)> {
    use windows::Win32::Foundation::{HWND, RECT};
    use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS};
    let hwnd = HWND(hwnd as usize as *mut _);
    let mut rect = RECT::default();
    unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut rect as *mut _ as *mut _,
            std::mem::size_of::<RECT>() as u32,
        )
        .ok()?;
    }
    Some((rect.left, rect.top, (rect.right - rect.left) as u32, (rect.bottom - rect.top) as u32))
}

/// Captures a snapshot of a specific window's current on-screen content, for
/// the recorder's Window-mode picker grid. `xcap::Window::id()` returns the
/// same raw HWND value as `WinInfo::id` on Windows, so matching by id lines
/// up with `list_windows()`'s native-enumeration results even though this
/// capture itself goes through the cross-platform `xcap` path.
///
/// On Windows this goes through `PrintWindow`, which — for a window that's
/// covered by others (not minimized, just occluded) — can report success
/// while actually handing back a solid-black image: some GPU-accelerated
/// apps don't repaint their off-screen surface for a `WM_PRINT` request the
/// way `PrintWindow` needs. There's no way to tell success from this failure
/// mode from the API's own return value, so a near-solid-black result is
/// treated as "capture failed" here. That matters beyond just not showing a
/// black square: the picker's background cache only re-captures a window
/// once (see `scan_window_thumbs`'s `only_new`) *unless* the previous
/// attempt came back empty — so discarding a black result is what lets a
/// later, unoccluded tick actually get a real thumbnail instead of being
/// stuck with the black one forever.
pub fn capture_window_thumbnail(hwnd_u32: u32) -> Option<RgbaImage> {
    let windows = xcap::Window::all().ok()?;
    let win = windows.into_iter().find(|w| w.id().ok() == Some(hwnd_u32))?;
    let img = win.capture_image().ok()?;
    if is_blank_capture(&img) {
        return None;
    }
    Some(img)
}

/// Whether `img` is (near-)solid black across a sampled subset of its
/// pixels — cheap enough to run on every capture, only needs to catch
/// "PrintWindow silently gave us nothing", not do real image analysis.
pub(crate) fn is_blank_capture(img: &RgbaImage) -> bool {
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 {
        return true;
    }
    let total = (w as u64) * (h as u64);
    let step = (total / 2000).max(1) as usize;
    for (i, p) in img.pixels().enumerate() {
        if i % step != 0 {
            continue;
        }
        let [r, g, b, _] = p.0;
        if r.max(g).max(b) >= 6 {
            return false; // found a non-black pixel — this is a real capture
        }
    }
    true
}

/// Lists visible, top-level windows in z-order (topmost first).
/// Used for highlighting in the window-picker overlay.
pub fn list_windows() -> Vec<WinInfo> {
    #[cfg(windows)]
    {
        use windows::Win32::Foundation::{BOOL, HWND, LPARAM, RECT, TRUE};
        use windows::Win32::System::Threading::GetCurrentProcessId;
        use windows::Win32::UI::WindowsAndMessaging::{
            EnumWindows, GetWindowLongW, GetWindowRect, GetWindowThreadProcessId, IsIconic, IsWindowVisible, GWL_EXSTYLE,
        };

        extern "system" fn cb(hwnd: HWND, lparam: LPARAM) -> BOOL {
            unsafe {
                let out = &mut *(lparam.0 as *mut Vec<WinInfo>);
                if !IsWindowVisible(hwnd).as_bool() {
                    return TRUE;
                }
                // Minimizing doesn't clear WS_VISIBLE, so a minimized window
                // still passes the check above — kept pickable (it's already
                // restorable: `bring_window_to_foreground` calls `SW_RESTORE`
                // when picked). Its rect/frame is a meaningless off-screen
                // placeholder while iconic, though, so it's excluded from the
                // size filter below rather than being wrongly judged "too small".
                let minimized = IsIconic(hwnd).as_bool();
                // Never list Capcove's own windows (control bar, gallery, …) —
                // recording our own UI makes no sense, and several of them
                // share the plain "Capcove" title, which read as confusing
                // duplicates in the picker.
                let mut owner_pid = 0u32;
                GetWindowThreadProcessId(hwnd, Some(&mut owner_pid));
                if owner_pid == GetCurrentProcessId() {
                    return TRUE;
                }
                let ex = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
                // Skip floating toolbars and non-activatable system UI (e.g. input panel)
                use windows::Win32::UI::WindowsAndMessaging::{WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW};
                if ex & WS_EX_TOOLWINDOW.0 != 0 || ex & WS_EX_NOACTIVATE.0 != 0 {
                    return TRUE;
                }
                // An empty native title isn't reason to drop the window —
                // browser "app mode" windows (a website installed as a
                // standalone app, e.g. Chrome/Edge `--app-id=`) mirror the
                // page's <title> into the caption, which is blank during
                // navigation/load, making them intermittently vanish from
                // the picker for no visible reason. Keep the window with a
                // blank title; the picker's own UI already falls back to
                // the app/exe name when the title is empty.
                let title = window_title(hwnd).unwrap_or_default();
                let mut rect = RECT::default();
                if GetWindowRect(hwnd, &mut rect).is_err() {
                    return TRUE;
                }
                use windows::Win32::Graphics::Dwm::{
                    DwmGetWindowAttribute, DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS,
                };
                // Skip cloaked windows: virtual desktops, shell-managed hidden windows
                let mut cloaked: u32 = 0;
                let _ = DwmGetWindowAttribute(
                    hwnd,
                    DWMWA_CLOAKED,
                    &mut cloaked as *mut _ as *mut _,
                    std::mem::size_of::<u32>() as u32,
                );
                if cloaked != 0 {
                    return TRUE;
                }
                // Prefer DWM frame bounds — strips the invisible shadow margin that
                // GetWindowRect includes, giving accurate visual coordinates.
                let mut frame = rect;
                let _ = DwmGetWindowAttribute(
                    hwnd,
                    DWMWA_EXTENDED_FRAME_BOUNDS,
                    &mut frame as *mut _ as *mut _,
                    std::mem::size_of::<RECT>() as u32,
                );
                let (w, h) = (frame.right - frame.left, frame.bottom - frame.top);
                if !minimized && (w < 48 || h < 48) {
                    return TRUE;
                }
                out.push(WinInfo {
                    id: hwnd.0 as u32,
                    title,
                    app: window_exe(hwnd).unwrap_or_default(),
                    x: frame.left,
                    y: frame.top,
                    w,
                    h,
                });
                TRUE
            }
        }

        let mut out: Vec<WinInfo> = Vec::new();
        unsafe {
            let _ = EnumWindows(Some(cb), LPARAM(&mut out as *mut _ as isize));
        }
        return out;
    }
    #[cfg(not(windows))]
    {
        return list_windows_xcap();
    }
    #[allow(unreachable_code)]
    Vec::new()
}

/// Cross-platform window enumeration via `xcap::Window` (macOS, Linux/X11+XWayland).
/// Returns an empty list on native Wayland sessions, where per-window introspection
/// isn't available — callers should fall back to monitor-level capture there.
#[cfg(not(windows))]
fn list_windows_xcap() -> Vec<WinInfo> {
    /// Returns true for system/desktop windows that should never appear in the picker.
    fn is_system_window(title: &str, app: &str) -> bool {
        // Desktop icon overlays (GNOME, Nemo, KDE Plasma, etc.)
        let title_lc = title.to_lowercase();
        let app_lc   = app.to_lowercase();
        if title_lc.starts_with("desktop icons") {
            return true;
        }
        // Taskbars / panels / docks
        if app_lc.contains("panel")
            || app_lc.contains("plank")
            || app_lc.contains("xfce4-panel")
            || app_lc.contains("lxpanel")
            || app_lc.contains("tint2")
            || app_lc.contains("polybar")
            || app_lc.contains("waybar")
        {
            return true;
        }
        // macOS system chrome that shows up in the window list but isn't a
        // real pickable window.
        if app_lc == "dock"
            || app_lc == "window server"
            || app_lc == "control center"
            || app_lc == "control centre"
            || app_lc == "notification center"
            || app_lc == "notification centre"
            || app_lc == "spotlight"
            || app_lc == "loginwindow"
        {
            return true;
        }
        false
    }

    let Ok(windows) = xcap::Window::all() else { return Vec::new() };
    windows
        .into_iter()
        .filter_map(|w| {
            if w.is_minimized().unwrap_or(false) {
                return None;
            }
            let id = w.id().ok()?;
            let mut x = w.x().ok()?;
            let mut y = w.y().ok()?;
            let mut width  = w.width().ok()? as i32;
            let mut height = w.height().ok()? as i32;
            if width < 48 || height < 48 {
                return None;
            }
            if let Some((left, right, top, bottom)) = gtk_frame_extents(id) {
                x += left;
                y += top;
                width -= left + right;
                height -= top + bottom;
            }
            let app_name = w.app_name().unwrap_or_default();
            // Many macOS apps never set a CGWindowName, so fall back to the
            // app name rather than dropping the window entirely.
            let title = w.title().ok().filter(|t| !t.trim().is_empty()).unwrap_or_else(|| app_name.clone());
            if title.is_empty() && app_name.is_empty() {
                return None;
            }
            if is_system_window(&title, &app_name) {
                return None;
            }
            Some(WinInfo {
                id,
                title,
                app: app_name,
                x,
                y,
                w: width,
                h: height,
            })
        })
        .collect()
}

#[cfg(windows)]
fn cursor_position_fallback() -> (i32, i32) {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;
    let mut p = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut p);
    }
    (p.x, p.y)
}

#[cfg(not(windows))]
fn cursor_position_fallback() -> (i32, i32) {
    use mouse_position::mouse_position::Mouse;
    match Mouse::get_mouse_position() {
        Mouse::Position { x, y } => (x, y),
        Mouse::Error => (0, 0),
    }
}

pub fn cursor_position() -> (i32, i32) {
    cursor_position_fallback()
}

#[cfg(not(windows))]
fn gtk_frame_extents(window_id: u32) -> Option<(i32, i32, i32, i32)> {
    let output = std::process::Command::new("xprop")
        .args(&["-id", &window_id.to_string(), "_GTK_FRAME_EXTENTS"])
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
    let values_str = parts[1].trim();
    let values: Vec<i32> = values_str
        .split(',')
        .map(|s| s.trim().parse::<i32>().unwrap_or(0))
        .collect();
    if values.len() == 4 {
        Some((values[0], values[1], values[2], values[3]))
    } else {
        None
    }
}
