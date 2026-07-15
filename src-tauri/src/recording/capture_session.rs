//! Windows Graphics Capture frame source for a single window or monitor,
//! with optional per-frame cropping for area recording. Frames arrive on a
//! dedicated thread that must never block, so `FrameMailbox` is single-slot
//! and always overwrites unconsumed frames.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use windows_capture::capture::{Context, GraphicsCaptureApiHandler};
use windows_capture::frame::Frame;
use windows_capture::graphics_capture_api::InternalCaptureControl;
use windows_capture::monitor::Monitor;
use windows_capture::settings::{
    ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings,
    MinimumUpdateIntervalSettings, SecondaryWindowSettings, Settings,
};
use windows_capture::window::Window;

pub struct FrameMsg {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}

pub enum PopResult {
    Frame(FrameMsg),
    TimedOut,
    Closed,
}

pub struct FrameMailbox {
    slot: Mutex<Option<FrameMsg>>,
    cond: Condvar,
    closed: AtomicBool,
}

impl FrameMailbox {
    pub fn new() -> Arc<Self> {
        Arc::new(Self { slot: Mutex::new(None), cond: Condvar::new(), closed: AtomicBool::new(false) })
    }

    fn push(&self, msg: FrameMsg) {
        let mut slot = self.slot.lock().unwrap();
        *slot = Some(msg);
        self.cond.notify_one();
    }

    /// Like `pop`, but gives up after `timeout` — lets writer loops notice
    /// the source going quiet (e.g. a minimized window) so they can
    /// synthesize placeholder frames and keep timeline/audio in sync.
    pub fn pop_timeout(&self, timeout: Duration) -> PopResult {
        let deadline = std::time::Instant::now() + timeout;
        let mut slot = self.slot.lock().unwrap();
        loop {
            if let Some(msg) = slot.take() {
                return PopResult::Frame(msg);
            }
            if self.closed.load(Ordering::Acquire) {
                return PopResult::Closed;
            }
            let now = std::time::Instant::now();
            if now >= deadline {
                return PopResult::TimedOut;
            }
            let (guard, _) = self.cond.wait_timeout(slot, deadline - now).unwrap();
            slot = guard;
        }
    }

    pub fn close(&self) {
        self.closed.store(true, Ordering::Release);
        self.cond.notify_all();
    }
}

type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// `(x, y, w, h)` in the captured surface's own pixel coordinates.
pub type CropRect = (u32, u32, u32, u32);

pub struct FrameSourceFlags {
    pub mailbox: Arc<FrameMailbox>,
    pub crop: Option<CropRect>,
}

pub struct FrameSource {
    mailbox: Arc<FrameMailbox>,
    crop: Option<CropRect>,
}

impl GraphicsCaptureApiHandler for FrameSource {
    type Flags = FrameSourceFlags;
    type Error = BoxError;

    fn new(ctx: Context<Self::Flags>) -> Result<Self, Self::Error> {
        Ok(Self { mailbox: ctx.flags.mailbox, crop: ctx.flags.crop })
    }

    fn on_frame_arrived(
        &mut self,
        frame: &mut Frame,
        _capture_control: InternalCaptureControl,
    ) -> Result<(), Self::Error> {
        let mut buffer = frame.buffer()?;
        let full_w = buffer.width();
        let full_h = buffer.height();
        let data = buffer.as_nopadding_buffer()?;

        // `Window::title_bar_height()` double-applies DPI scaling on a
        // per-monitor-aware caller, which can wrap negative into a huge
        // `u32` — skip a crop that would eat half the frame or more.
        let crop = self.crop.filter(|&(_, cy, _, _)| cy < full_h / 2);

        if let Some((cx, cy, cw, ch)) = crop {
            let cx = cx.min(full_w.saturating_sub(1));
            let cy = cy.min(full_h.saturating_sub(1));
            let cw = cw.min(full_w.saturating_sub(cx)).max(1);
            let ch = ch.min(full_h.saturating_sub(cy)).max(1);
            let stride = (full_w * 4) as usize;
            let mut out = Vec::with_capacity((cw * ch * 4) as usize);
            for row in 0..ch {
                let src_y = (cy + row) as usize;
                let start = src_y * stride + (cx * 4) as usize;
                let end = start + (cw * 4) as usize;
                out.extend_from_slice(&data[start..end]);
            }
            self.mailbox.push(FrameMsg { width: cw, height: ch, bgra: out });
        } else {
            self.mailbox.push(FrameMsg { width: full_w, height: full_h, bgra: data.to_vec() });
        }
        Ok(())
    }

    fn on_closed(&mut self) -> Result<(), Self::Error> {
        self.mailbox.close();
        Ok(())
    }
}

pub type CaptureHandle = windows_capture::capture::CaptureControl<FrameSource, BoxError>;

/// `WithoutBorder` only when granted — otherwise it fails the capture session
/// on a packaged build; `Default` always starts (see `borderless_capture_granted`).
fn border_settings() -> DrawBorderSettings {
    if crate::win_util::borderless_capture_granted() {
        DrawBorderSettings::WithoutBorder
    } else {
        DrawBorderSettings::Default
    }
}

/// Starts capturing `hwnd` on a dedicated thread. `fps` caps the update rate
/// WGC will deliver frames at (it never delivers faster than the window
/// actually redraws, so this is a ceiling, not a guarantee).
pub fn start_window_capture(
    hwnd: u32,
    fps: u32,
    capture_cursor: bool,
    exclude_overlay_windows: bool,
    crop_titlebar: bool,
    mailbox: Arc<FrameMailbox>,
) -> Result<CaptureHandle, String> {
    let window = Window::from_raw_hwnd(hwnd as usize as *mut std::ffi::c_void);
    let cursor = if capture_cursor { CursorCaptureSettings::WithCursor } else { CursorCaptureSettings::WithoutCursor };
    // "Secondary" windows are ones layered on the captured window; some
    // overlays render this way, so excluding them keeps them out of frame.
    let secondary = if exclude_overlay_windows { SecondaryWindowSettings::Exclude } else { SecondaryWindowSettings::Default };
    // Crop off the OS title bar so window recordings show just the app
    // content; computed once since it only changes with DPI, not a resize.
    let title_bar_h = if crop_titlebar { window.title_bar_height().unwrap_or(0) } else { 0 };
    let crop = (title_bar_h > 0).then_some((0, title_bar_h, u32::MAX, u32::MAX));
    let settings = Settings::new(
        window,
        cursor,
        border_settings(),
        secondary,
        MinimumUpdateIntervalSettings::Custom(Duration::from_millis(1000 / fps.max(1) as u64)),
        DirtyRegionSettings::Default,
        ColorFormat::Bgra8,
        FrameSourceFlags { mailbox, crop },
    );
    FrameSource::start_free_threaded(settings).map_err(|e| e.to_string())
}

/// Starts capturing a monitor. `index` is 1-based; `None` captures the
/// primary monitor directly. `crop`, if set, is applied per-frame for area
/// recording.
pub fn start_monitor_capture(
    index: Option<usize>,
    fps: u32,
    capture_cursor: bool,
    crop: Option<CropRect>,
    mailbox: Arc<FrameMailbox>,
) -> Result<CaptureHandle, String> {
    let monitor = match index {
        Some(i) => Monitor::from_index(i).map_err(|e| e.to_string())?,
        None => Monitor::primary().map_err(|e| e.to_string())?,
    };
    let cursor = if capture_cursor { CursorCaptureSettings::WithCursor } else { CursorCaptureSettings::WithoutCursor };
    let settings = Settings::new(
        monitor,
        cursor,
        border_settings(),
        SecondaryWindowSettings::Default,
        MinimumUpdateIntervalSettings::Custom(Duration::from_millis(1000 / fps.max(1) as u64)),
        DirtyRegionSettings::Default,
        ColorFormat::Bgra8,
        FrameSourceFlags { mailbox, crop },
    );
    FrameSource::start_free_threaded(settings).map_err(|e| e.to_string())
}

/// Finds the 1-based monitor index and physical-pixel origin of the display
/// containing point `(x, y)`, by querying `GetMonitorInfoW` on each entry's
/// own `HMONITOR` since enumeration order isn't guaranteed to match `capture::list_monitors()`.
pub fn monitor_index_and_origin_at(x: i32, y: i32) -> Result<(usize, i32, i32), String> {
    use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, HMONITOR, MONITORINFO};

    let monitors = Monitor::enumerate().map_err(|e| e.to_string())?;
    for (i, mon) in monitors.iter().enumerate() {
        let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        let hmon = HMONITOR(mon.as_raw_hmonitor());
        if !unsafe { GetMonitorInfoW(hmon, &mut info) }.as_bool() {
            continue;
        }
        let r = info.rcMonitor;
        if x >= r.left && x < r.right && y >= r.top && y < r.bottom {
            return Ok((i + 1, r.left, r.top)); // from_index is 1-based
        }
    }
    Err("no monitor contains that point".into())
}
