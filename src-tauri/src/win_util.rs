/// Makes an overlay window pop rather than animate in: disables DWM open/
/// close transition animations.
#[cfg(windows)]
pub fn make_overlay_ghost(hwnd_u32: u32) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_TRANSITIONS_FORCEDISABLED};

    let hwnd = HWND(hwnd_u32 as usize as *mut _);
    let disable: i32 = 1; // BOOL TRUE
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_TRANSITIONS_FORCEDISABLED,
            &disable as *const _ as *const core::ffi::c_void,
            std::mem::size_of::<i32>() as u32,
        );
    }
}

#[cfg(not(windows))]
pub fn make_overlay_ghost(_hwnd_u32: u32) {}

/// Sets (or clears) `WDA_EXCLUDEFROMCAPTURE` (Win10 2004+) on an overlay
/// window, so no other capture/streaming software can see it either.
/// User-toggleable (`Settings::hide_overlays_from_capture`, default on).
#[cfg(windows)]
pub fn set_capture_hidden(hwnd_u32: u32, hidden: bool) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{SetWindowDisplayAffinity, WDA_EXCLUDEFROMCAPTURE, WDA_NONE};

    let hwnd = HWND(hwnd_u32 as usize as *mut _);
    let affinity = if hidden { WDA_EXCLUDEFROMCAPTURE } else { WDA_NONE };
    unsafe {
        let _ = SetWindowDisplayAffinity(hwnd, affinity);
    }
}

#[cfg(not(windows))]
pub fn set_capture_hidden(_hwnd_u32: u32, _hidden: bool) {}

/// Restores (if minimized) and brings a window to the foreground — used
/// when the recorder's Window-mode picker hands back a target, so the
/// picked window is what the user actually sees while recording.
#[cfg(windows)]
pub fn bring_window_to_foreground(hwnd_u32: u32) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{IsIconic, SetForegroundWindow, ShowWindow, SW_RESTORE};

    let hwnd = HWND(hwnd_u32 as usize as *mut _);
    unsafe {
        if IsIconic(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_RESTORE);
        }
        let _ = SetForegroundWindow(hwnd);
    }
}

#[cfg(not(windows))]
pub fn bring_window_to_foreground(_hwnd_u32: u32) {}

/// Waits up to `timeout_ms` for a process to exit on its own; force-kills it
/// if it hasn't. Safety net for ffmpeg's graceful stdin-close stop.
#[cfg(windows)]
pub fn wait_or_kill_process(pid: u32, timeout_ms: u32) {
    use windows::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
    use windows::Win32::System::Threading::{OpenProcess, TerminateProcess, WaitForSingleObject, PROCESS_ACCESS_RIGHTS, PROCESS_TERMINATE};

    const SYNCHRONIZE: PROCESS_ACCESS_RIGHTS = PROCESS_ACCESS_RIGHTS(0x0010_0000);
    unsafe {
        let Ok(handle) = OpenProcess(PROCESS_TERMINATE | SYNCHRONIZE, false, pid) else { return };
        if WaitForSingleObject(handle, timeout_ms) != WAIT_OBJECT_0 {
            log::warn!("process {pid} didn't exit within {timeout_ms}ms of a graceful stop — force-killing it");
            let _ = TerminateProcess(handle, 1);
        }
        let _ = CloseHandle(handle);
    }
}

#[cfg(not(windows))]
pub fn wait_or_kill_process(_pid: u32, _timeout_ms: u32) {}

// 2 = unresolved (default until a check/request completes at least once).
#[cfg(windows)]
static BORDERLESS_GRANTED: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(2);

#[cfg(windows)]
fn set_borderless_granted(granted: bool) {
    BORDERLESS_GRANTED.store(granted as u8, std::sync::atomic::Ordering::Relaxed);
}

/// Whether the OS has granted borderless capture. Non-blocking: capture
/// callers gate their `DrawBorderSettings` on this, so it must never block.
/// Until a status check has resolved it at least once, unpackaged builds
/// report `true` (they can always drop the border) and packaged builds
/// report `false` (record bordered rather than risk a failed/hung start).
#[cfg(windows)]
pub fn borderless_capture_granted() -> bool {
    match BORDERLESS_GRANTED.load(std::sync::atomic::Ordering::Relaxed) {
        0 => false,
        1 => true,
        _ => !is_packaged(),
    }
}

#[cfg(not(windows))]
pub fn borderless_capture_granted() -> bool {
    true
}

/// Current state of an OS consent capability, for the settings-icon warning
/// and its explainer modal in the frontend.
#[derive(serde::Serialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityStatus {
    /// Unpackaged build — the capability doesn't apply.
    NotApplicable,
    /// Packaged, never decided yet — requesting access will show the OS prompt.
    NeedsPrompt,
    Granted,
    /// Packaged and previously denied (by the user or by policy) — Windows
    /// won't show the prompt again; only Settings can change this now.
    Denied,
}

/// The consent-gated capabilities Capcove tracks, unified under one
/// "requested permissions" UI in the frontend instead of separate one-off
/// modals per capability.
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityKind {
    /// `graphicsCaptureWithoutBorder` — suppresses the OS's yellow capture
    /// border from ending up in recordings/screenshots.
    BorderlessCapture,
    /// `microphone` — capturing the mic as an audio track.
    Microphone,
}

impl CapabilityKind {
    pub const ALL: [CapabilityKind; 2] = [CapabilityKind::BorderlessCapture, CapabilityKind::Microphone];

    /// The name Windows tracks this capability's consent under — matches the
    /// manifest's `<rescap:Capability>`/`<DeviceCapability>` `Name` attribute.
    fn capability_name(self) -> &'static str {
        match self {
            Self::BorderlessCapture => "graphicsCaptureWithoutBorder",
            Self::Microphone => "microphone",
        }
    }

    /// Deep link to this capability's page in Windows Settings, for when
    /// Windows won't show the consent prompt again after a denial.
    pub fn settings_uri(self) -> &'static str {
        match self {
            Self::BorderlessCapture => "ms-settings:privacy-graphicscapturewithoutborder",
            Self::Microphone => "ms-settings:privacy-microphone",
        }
    }
}

/// Checks the current OS consent status for `kind` without showing any
/// prompt. Blocks on a WinRT call, so it must run off the UI thread.
#[cfg(windows)]
pub fn capability_status(kind: CapabilityKind) -> CapabilityStatus {
    if !is_packaged() {
        return CapabilityStatus::NotApplicable;
    }
    use windows::Security::Authorization::AppCapabilityAccess::{AppCapability, AppCapabilityAccessStatus};
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
    use windows_core::HSTRING;

    unsafe { let _ = CoInitializeEx(None, COINIT_MULTITHREADED); }
    match AppCapability::Create(&HSTRING::from(kind.capability_name())).and_then(|cap| cap.CheckAccess()) {
        Ok(AppCapabilityAccessStatus::Allowed) => {
            if kind == CapabilityKind::BorderlessCapture { set_borderless_granted(true); }
            CapabilityStatus::Granted
        }
        Ok(AppCapabilityAccessStatus::UserPromptRequired) => CapabilityStatus::NeedsPrompt,
        Ok(status) => {
            log::info!("{kind:?}: denied (status {})", status.0);
            if kind == CapabilityKind::BorderlessCapture { set_borderless_granted(false); }
            CapabilityStatus::Denied
        }
        Err(e) => {
            log::warn!("{kind:?} status check failed: {e}");
            CapabilityStatus::NeedsPrompt
        }
    }
}

#[cfg(not(windows))]
pub fn capability_status(_kind: CapabilityKind) -> CapabilityStatus {
    CapabilityStatus::NotApplicable
}

/// Actually shows the OS consent prompt for `kind` (only does so if Windows
/// hasn't recorded a decision yet — otherwise it just returns the cached
/// status, or in microphone's case briefly opens the device for nothing).
/// Blocks on a WinRT/WASAPI call, so it must run off the UI thread.
///
/// Borderless capture and microphone need genuinely different APIs to
/// trigger their prompt: `GraphicsCaptureAccess.RequestAccessAsync` works
/// directly. Microphone consent isn't tied to a request API at all — Windows
/// shows it the first time an app actually starts a capture stream, so
/// `audio_capture::request_microphone_consent` just does exactly that,
/// briefly, and tears it back down.
#[cfg(windows)]
pub fn request_capability(kind: CapabilityKind) -> CapabilityStatus {
    if !is_packaged() {
        return CapabilityStatus::NotApplicable;
    }
    match kind {
        CapabilityKind::BorderlessCapture => request_borderless_capture_access(),
        CapabilityKind::Microphone => match crate::recording::audio_capture::request_microphone_consent() {
            Ok(()) => CapabilityStatus::Granted,
            Err(e) => {
                log::warn!("microphone consent request failed: {e}");
                CapabilityStatus::Denied
            }
        },
    }
}

#[cfg(not(windows))]
pub fn request_capability(_kind: CapabilityKind) -> CapabilityStatus {
    CapabilityStatus::NotApplicable
}

/// Runs the request on an MTA thread on purpose: a blocking `.get()` on an STA
/// thread deadlocks, since the async completion needs the STA message pump this
/// thread isn't pumping — which previously hung the whole capture on packaged
/// builds.
#[cfg(windows)]
fn request_borderless_capture_access() -> CapabilityStatus {
    use windows::Graphics::Capture::{GraphicsCaptureAccess, GraphicsCaptureAccessKind};
    use windows::Security::Authorization::AppCapabilityAccess::AppCapabilityAccessStatus;
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};

    unsafe { let _ = CoInitializeEx(None, COINIT_MULTITHREADED); }
    match GraphicsCaptureAccess::RequestAccessAsync(GraphicsCaptureAccessKind::Borderless).and_then(|op| op.get()) {
        Ok(AppCapabilityAccessStatus::Allowed) => {
            log::info!("borderless screen capture: granted");
            set_borderless_granted(true);
            CapabilityStatus::Granted
        }
        Ok(status) => {
            log::info!("borderless screen capture: not granted (status {}) — recording with the OS capture border", status.0);
            set_borderless_granted(false);
            CapabilityStatus::Denied
        }
        Err(e) => {
            log::warn!("borderless screen capture request failed: {e} — recording with the OS capture border");
            set_borderless_granted(false);
            CapabilityStatus::Denied
        }
    }
}

/// Sets whole-window opacity (0-255) via a layered-window attribute. Needs
/// `WS_EX_LAYERED` set once before `SetLayeredWindowAttributes` takes effect.
#[cfg(windows)]
pub fn set_window_opacity(hwnd_u32: u32, alpha: u8) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindowLongPtrW, SetLayeredWindowAttributes, SetWindowLongPtrW, GWL_EXSTYLE, LWA_ALPHA, WS_EX_LAYERED,
    };

    let hwnd = HWND(hwnd_u32 as usize as *mut _);
    unsafe {
        let ex_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        if ex_style & (WS_EX_LAYERED.0 as isize) == 0 {
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex_style | WS_EX_LAYERED.0 as isize);
        }
        let _ = SetLayeredWindowAttributes(hwnd, windows::Win32::Foundation::COLORREF(0), alpha, LWA_ALPHA);
    }
}

#[cfg(not(windows))]
pub fn set_window_opacity(_hwnd_u32: u32, _alpha: u8) {}


/// Returns true if the current process is running with elevated (administrator) privileges.
#[cfg(windows)]
pub fn is_elevated() -> bool {
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::Security::{
        GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY,
    };
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
            return false;
        }
        let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
        let mut return_len = 0u32;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            Some(&mut elevation as *mut _ as *mut _),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut return_len,
        )
        .is_ok();
        let _ = CloseHandle(token);
        ok && elevation.TokenIsElevated != 0
    }
}

#[cfg(not(windows))]
pub fn is_elevated() -> bool {
    false
}

/// Re-launches the current executable with UAC elevation ("runas") then exits this process.
#[cfg(windows)]
pub fn restart_as_admin() {
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let exe = std::env::current_exe().unwrap_or_default();
    let exe_wide: Vec<u16> = exe.to_string_lossy().encode_utf16().chain([0u16]).collect();
    let verb: Vec<u16> = "runas".encode_utf16().chain([0u16]).collect();
    unsafe {
        ShellExecuteW(
            None,
            windows::core::PCWSTR(verb.as_ptr()),
            windows::core::PCWSTR(exe_wide.as_ptr()),
            windows::core::PCWSTR(std::ptr::null()),
            windows::core::PCWSTR(std::ptr::null()),
            SW_SHOWNORMAL,
        );
    }
    std::process::exit(0);
}

#[cfg(not(windows))]
pub fn restart_as_admin() {}

/// Creates a Windows Task Scheduler logon task that launches the app with highest privileges.
/// Requires the caller to already be running elevated.
#[cfg(windows)]
pub fn create_admin_autostart() -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let exe_str = exe.to_string_lossy();
    let status = std::process::Command::new("schtasks")
        .args([
            "/create",
            "/f",
            "/tn",
            "Capcove",
            "/tr",
            &format!("\"{}\"", exe_str),
            "/sc",
            "onlogon",
            "/rl",
            "highest",
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .status()
        .map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err("schtasks /create failed".into())
    }
}

#[cfg(not(windows))]
pub fn create_admin_autostart() -> Result<(), String> {
    Ok(())
}

/// Removes the Capcove Task Scheduler logon task if it exists.
#[cfg(windows)]
pub fn remove_admin_autostart() {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let _ = std::process::Command::new("schtasks")
        .args(["/delete", "/f", "/tn", "Capcove"])
        .creation_flags(CREATE_NO_WINDOW)
        .status();
}

#[cfg(not(windows))]
pub fn remove_admin_autostart() {}

/// Returns true if the process is running from an installed MSIX/AppX package
/// (e.g. Microsoft Store). Such installs already get a Start Menu entry from
/// their manifest and are updated by the Store, not by our own logic.
#[cfg(windows)]
pub fn is_packaged() -> bool {
    extern "system" {
        fn GetCurrentPackageFullName(length: *mut u32, full_name: *mut u16) -> u32;
    }
    const APPMODEL_ERROR_NO_PACKAGE: u32 = 15700;

    let mut length: u32 = 0;
    let result = unsafe { GetCurrentPackageFullName(&mut length, std::ptr::null_mut()) };
    result != APPMODEL_ERROR_NO_PACKAGE
}

#[cfg(not(windows))]
pub fn is_packaged() -> bool {
    false
}

/// Package family name (e.g. `Alperenetin.Capcove_w4kcn2812j35r`) when running
/// from an MSIX/AppX package, else `None`. Used to locate the package's real
/// writable folders, whose paths differ from the plain `%LOCALAPPDATA%` ones a
/// packaged process's file writes get redirected into.
#[cfg(windows)]
pub fn package_family_name() -> Option<String> {
    extern "system" {
        fn GetCurrentPackageFamilyName(length: *mut u32, name: *mut u16) -> u32;
    }
    const ERROR_INSUFFICIENT_BUFFER: u32 = 122;

    let mut length: u32 = 0;
    // First call reports the required buffer length (including the null).
    if unsafe { GetCurrentPackageFamilyName(&mut length, std::ptr::null_mut()) } != ERROR_INSUFFICIENT_BUFFER
        || length == 0
    {
        return None; // not packaged, or no name
    }
    let mut buf = vec![0u16; length as usize];
    if unsafe { GetCurrentPackageFamilyName(&mut length, buf.as_mut_ptr()) } != 0 {
        return None;
    }
    Some(String::from_utf16_lossy(&buf[..(length as usize).saturating_sub(1)]))
}

#[cfg(not(windows))]
pub fn package_family_name() -> Option<String> {
    None
}

#[cfg(all(windows, not(debug_assertions)))]
pub fn register_app_shortcut(run_as_admin: bool) -> Result<(), Box<dyn std::error::Error>> {
    use std::env;
    use std::path::PathBuf;

    if is_packaged() {
        log::info!("Running as a packaged app (MSIX); skipping Start Menu shortcut creation");
        return Ok(());
    }
    use windows::Win32::Storage::EnhancedStorage::PKEY_AppUserModel_ID;
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
        IPersistFile,
    };
    use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};
    use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
    use windows::core::{Interface, PCWSTR, PROPVARIANT};

    let current_exe = env::current_exe()?;
    let appdata = env::var("APPDATA")?;
    let shortcut_dir = PathBuf::from(appdata).join("Microsoft\\Windows\\Start Menu\\Programs");
    let shortcut_path = shortcut_dir.join("Capcove.lnk");

    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let shell_link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
        let exe_path_u16: Vec<u16> = current_exe
            .to_string_lossy()
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        shell_link.SetPath(PCWSTR(exe_path_u16.as_ptr()))?;
        if let Some(parent) = current_exe.parent() {
            let working_dir_u16: Vec<u16> = parent
                .to_string_lossy()
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();
            shell_link.SetWorkingDirectory(PCWSTR(working_dir_u16.as_ptr()))?;
        }
        let prop_store: IPropertyStore = shell_link.cast()?;
        let app_id = PROPVARIANT::from("dev.xacnio.capcove");
        prop_store.SetValue(&PKEY_AppUserModel_ID, &app_id)?;
        prop_store.Commit()?;
        let persist_file: IPersistFile = shell_link.cast()?;
        let shortcut_path_u16: Vec<u16> = shortcut_path
            .to_string_lossy()
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        persist_file.Save(PCWSTR(shortcut_path_u16.as_ptr()), true)?;
    }

    // Set/clear the SLDF_RUNAS_USER (0x2000) bit in the .lnk LinkFlags field at offset 0x14.
    // This controls "Run as administrator" on the Start Menu shortcut.
    if let Ok(mut data) = std::fs::read(&shortcut_path) {
        if data.len() >= 0x18 {
            let flags = u32::from_le_bytes([data[0x14], data[0x15], data[0x16], data[0x17]]);
            let new_flags = if run_as_admin { flags | 0x2000 } else { flags & !0x2000 };
            if new_flags != flags {
                data[0x14..0x18].copy_from_slice(&new_flags.to_le_bytes());
                std::fs::write(&shortcut_path, &data)?;
            }
        }
    }

    Ok(())
}

/// Creates or updates the Start Menu shortcut with the current exe path and admin flag.
/// No-op in debug builds or on non-Windows.
pub fn update_start_menu_shortcut(run_as_admin: bool) {
    let _ = run_as_admin;
    #[cfg(all(windows, not(debug_assertions)))]
    if let Err(e) = register_app_shortcut(run_as_admin) {
        log::warn!("failed to update start menu shortcut: {e}");
    }
}

/// Loopback port used to signal a running instance to open its gallery window.
/// Picked away from a value an earlier build of this codebase also used.
const SINGLE_INSTANCE_PORT: u16 = 58312;

/// Tries to claim a process-wide named mutex. Returns `true` if first/only instance;
/// otherwise signals the running instance (via [`start_single_instance_listener`])
/// to open its gallery window and returns `false`.
#[cfg(windows)]
pub fn acquire_single_instance() -> bool {
    use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
    use windows::Win32::System::Threading::CreateMutexW;

    // Reverse-DNS app id, not just "Capcove" — avoids colliding with a
    // differently-branded build of the same codebase.
    let Ok(handle) = (unsafe { CreateMutexW(None, true, windows::core::w!("Local\\dev.xacnio.capcove.SingleInstanceMutex")) }) else {
        return true; // couldn't create the mutex — don't block startup over it
    };
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        notify_running_instance();
        return false;
    }
    // Never closed — the mutex stays held for the lifetime of this process;
    // Windows releases it automatically on process exit.
    let _ = handle;
    true
}

#[cfg(not(windows))]
pub fn acquire_single_instance() -> bool {
    true
}

/// Connects to the running instance's listener to ask it to open its gallery window.
/// A few short retries cover the brief startup window before the listener binds its socket.
#[cfg(windows)]
fn notify_running_instance() {
    use std::net::{SocketAddr, TcpStream};
    use std::time::Duration;

    let addr: SocketAddr = ([127, 0, 0, 1], SINGLE_INSTANCE_PORT).into();
    for _ in 0..5 {
        if TcpStream::connect_timeout(&addr, Duration::from_millis(200)).is_ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(150));
    }
}

/// Starts a background listener that opens the gallery window (creating it if needed)
/// whenever a second instance signals via [`notify_running_instance`].
pub fn start_single_instance_listener(app: tauri::AppHandle) {
    std::thread::spawn(move || {
        use std::net::{Ipv4Addr, TcpListener};
        let Ok(listener) = TcpListener::bind((Ipv4Addr::LOCALHOST, SINGLE_INSTANCE_PORT)) else {
            return; // port unavailable — best effort only, doesn't affect normal operation
        };
        for stream in listener.incoming() {
            if stream.is_err() {
                continue;
            }
            let app2 = app.clone();
            let _ = app.run_on_main_thread(move || {
                crate::tray::show_main(&app2);
            });
        }
    });
}

/// Explicitly sets this process's AppUserModelID, used for taskbar grouping/jump lists.
/// Does *not* affect toast notifications — those resolve their displayed name/icon via
/// [`register_notification_aumid`] instead (see its doc comment for why).
#[cfg(windows)]
pub fn set_app_user_model_id() {
    use windows::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID;
    unsafe {
        let _ = SetCurrentProcessExplicitAppUserModelID(windows::core::w!("dev.xacnio.capcove"));
    }
}

#[cfg(not(windows))]
pub fn set_app_user_model_id() {}

/// Registers the "dev.xacnio.capcove" AppUserModelID in the registry with a display
/// name and icon, so Windows toast notifications show "Capcove" instead of the
/// launching host process. `icon_path` must point to a real `.ico` file on disk.
#[cfg(windows)]
pub fn register_notification_aumid(icon_path: &std::path::Path) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let key = r"HKCU\Software\Classes\AppUserModelId\dev.xacnio.capcove";
    let run = |args: &[&str]| {
        let _ = std::process::Command::new("reg")
            .args(args)
            .creation_flags(CREATE_NO_WINDOW)
            .status();
    };
    run(&["add", key, "/v", "DisplayName", "/t", "REG_SZ", "/d", "Capcove", "/f"]);
    if let Some(icon_str) = icon_path.to_str() {
        run(&["add", key, "/v", "IconUri", "/t", "REG_SZ", "/d", icon_str, "/f"]);
    }
}

#[cfg(not(windows))]
pub fn register_notification_aumid(_icon_path: &std::path::Path) {}
