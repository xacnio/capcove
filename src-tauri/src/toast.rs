//! In-app toast notifications — a small always-on-top notification card,
//! positioned per `VideoSettings::toast_corner`.
//!
//! Two implementations exist side by side:
//! - `webview`: the original, a WebviewWindow (`src/toast/App.jsx`) owning
//!   queueing/animation/dismiss. Full CSS fidelity, but pays a whole WebView2
//!   instance's RAM just to show a small card.
//! - `crate::toast_native`: an experimental GDI-drawn layered window with no
//!   WebView2 involved at all — meaningfully less background RAM, at the
//!   cost of some visual/animation polish (no backdrop blur, approximate
//!   text layout).
//!
//! `USE_NATIVE` below is the only thing that picks between them; every
//! caller elsewhere in the app only ever sees this file's four public
//! functions and never knows which backend is actually running.

use std::sync::Arc;

use tauri::{AppHandle, Manager};

use crate::config::ConfigStore;

/// Flip to `false` to go back to the original WebView2-hosted overlay — both
/// implementations are kept fully intact, so this is the only line that
/// needs to change to switch back. `toast_native` only exists on Windows
/// (`#[cfg(windows)]` in lib.rs), so every dispatcher below gates the native
/// call behind the same cfg — a plain `bool` alone wouldn't stop the
/// `crate::toast_native` path from being name-resolved (and failing to
/// compile) on other platforms.
#[cfg(windows)]
const USE_NATIVE: bool = true;

/// What a toast is about — lets the user silence one category of event
/// without silencing all of them. `General` always shows regardless of settings.
#[derive(Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToastCategory {
    Recording,
    Session,
    Stream,
    Buffer,
    Clip,
    General,
}

impl ToastCategory {
    pub(crate) fn enabled(self, app: &AppHandle) -> bool {
        let cats = app.state::<Arc<ConfigStore>>().get().video.toast_categories;
        match self {
            ToastCategory::Recording => cats.recording,
            ToastCategory::Session => cats.session,
            ToastCategory::Stream => cats.stream,
            ToastCategory::Buffer => cats.buffer,
            ToastCategory::Clip => cats.clip,
            ToastCategory::General => true,
        }
    }
}

/// Builds the toast window at app startup so it's loaded and listening
/// before anything needs to be announced.
pub fn preload(app: &AppHandle) {
    #[cfg(windows)]
    if USE_NATIVE {
        crate::toast_native::preload(app);
        return;
    }
    webview::preload(app);
}

/// Re-applies the capture-hidden preference to the toast window if it
/// already exists — see `wheel::apply_capture_hidden` for why.
pub fn apply_capture_hidden(app: &AppHandle, hidden: bool) {
    #[cfg(windows)]
    if USE_NATIVE {
        crate::toast_native::apply_capture_hidden(app, hidden);
        return;
    }
    webview::apply_capture_hidden(app, hidden);
}

/// Queues an in-app toast. `kind` is "info" or "error" (styling only), gated
/// by `category`. Safe to call from any thread — hops to the main thread.
pub fn show(app: &AppHandle, kind: &str, category: ToastCategory, title: &str, body: &str) {
    show_with_icon(app, kind, category, title, body, None);
}

/// Same as `show`, plus a specific icon data URL (already resolved by the
/// caller — see `show_for_game` for the common "look it up by game name"
/// case).
pub fn show_with_icon(app: &AppHandle, kind: &str, category: ToastCategory, title: &str, body: &str, icon: Option<String>) {
    #[cfg(windows)]
    if USE_NATIVE {
        crate::toast_native::show_with_icon(app, kind, category, title, body, icon);
        return;
    }
    webview::show_with_icon(app, kind, category, title, body, icon);
}

/// Convenience for the common case: a toast about a specific game, whose
/// icon (if already cached locally) should show alongside it.
pub fn show_for_game(app: &AppHandle, kind: &str, category: ToastCategory, title: &str, body: &str, game_name: &str) {
    #[cfg(windows)]
    if USE_NATIVE {
        crate::toast_native::show_for_game(app, kind, category, title, body, game_name);
        return;
    }
    webview::show_for_game(app, kind, category, title, body, game_name);
}

/// Only meaningful for the `webview` backend — a no-op call from the native
/// frontend never happens since there's no page to call it. Still needs to
/// stay registered as a command regardless of `USE_NATIVE`.
#[tauri::command]
pub fn toast_ready(app: AppHandle) {
    webview::toast_ready(app);
}

// `USE_NATIVE = true` makes the compiler correctly (but harmlessly) flag some
// of this as unreachable — that's a snapshot of the current toggle, not a
// sign anything here is actually dead; flip `USE_NATIVE` and this becomes
// the live path again.
#[allow(dead_code)]
mod webview {
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};

    use tauri::{
        window::Color, AppHandle, Emitter, Manager, PhysicalPosition, WebviewUrl, WebviewWindowBuilder,
    };

    use crate::config::ConfigStore;
    use super::ToastCategory;

    const LABEL: &str = "toast";

    #[derive(Clone, serde::Serialize)]
    struct ToastPayload {
        id: u64,
        kind: String,
        category: ToastCategory,
        title: String,
        body: String,
        /// `data:image/...;base64,...` — a game's icon, when the toast concerns
        /// one and it's already cached locally (never fetched over the network
        /// just for this: a toast has to be instant or not bother).
        icon: Option<String>,
    }

    static NEXT_ID: AtomicU64 = AtomicU64::new(1);

    // Toasts fired before the page finishes loading are queued here until
    // `toast_ready` fires. WINDOW_READY is always accessed under the PENDING lock
    // so "check ready, else queue" can't race "mark ready, take queue".
    static WINDOW_READY: AtomicBool = AtomicBool::new(false);
    static PENDING: Mutex<Vec<ToastPayload>> = Mutex::new(Vec::new());

    /// Called by the toast frontend right after its `show-toast` listener is
    /// registered. See `WINDOW_READY`'s doc comment for why this handshake
    /// exists instead of relying on the page-load event.
    pub fn toast_ready(app: AppHandle) {
        let pending = {
            let mut queue = PENDING.lock().unwrap();
            WINDOW_READY.store(true, Ordering::SeqCst);
            std::mem::take(&mut *queue)
        };
        if let Some(win) = app.get_webview_window(LABEL) {
            for p in pending {
                let _ = win.emit("show-toast", p);
            }
        }
    }

    pub fn preload(app: &AppHandle) {
        let app2 = app.clone();
        let _ = app.run_on_main_thread(move || {
            if let Err(e) = ensure_window(&app2) {
                log::warn!("toast overlay could not be preloaded: {e}");
            }
        });
    }

    fn ensure_window(app: &AppHandle) -> Result<(), String> {
        if app.get_webview_window(LABEL).is_some() {
            return Ok(());
        }
        let monitor = app
            .primary_monitor()
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "no primary monitor".to_string())?;

        let win = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("pages/toast.html".into()))
            .title("Capcove")
            .decorations(false)
            .shadow(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .resizable(false)
            .transparent(true)
            .background_color(Color(0, 0, 0, 0))
            .focused(false)
            .visible(false)
            .build()
            .map_err(|e| e.to_string())?;
        // Covers the whole monitor so any of the 4 corners can be picked purely
        // via CSS, without repositioning when the user changes `toast_corner`.
        let _ = win.set_position(PhysicalPosition::new(monitor.position().x, monitor.position().y));
        let _ = win.set_size(*monitor.size());
        let _ = win.set_ignore_cursor_events(true);
        // Ghost window: no DWM open animation, and (unless opted out) hidden
        // from capture/streaming software entirely.
        if let Ok(raw) = win.hwnd() {
            let hwnd_u32 = raw.0 as usize as u32;
            crate::win_util::make_overlay_ghost(hwnd_u32);
            let hide = app.state::<Arc<ConfigStore>>().get().hide_overlays_from_capture;
            crate::win_util::set_capture_hidden(hwnd_u32, hide);
        }
        let _ = win.show();
        Ok(())
    }

    /// Re-applies the capture-hidden preference to the toast window if it
    /// already exists — see `wheel::apply_capture_hidden` for why.
    pub fn apply_capture_hidden(app: &AppHandle, hidden: bool) {
        if let Some(win) = app.get_webview_window(LABEL) {
            if let Ok(raw) = win.hwnd() {
                crate::win_util::set_capture_hidden(raw.0 as usize as u32, hidden);
            }
        }
    }

    /// Best-effort, local-only icon lookup for `show_for_game` — never fetches over
    /// the network. Checks the disk cache, then the embedded icon pack.
    fn game_icon_data_url(app: &AppHandle, name: &str) -> Option<String> {
        let ic = app.state::<Arc<crate::icon_cache::IconCache>>();
        if let Some(b64) = ic.get_base64(name) {
            return Some(format!("data:image/png;base64,{b64}"));
        }
        crate::games_db::embedded_icon_data_url(app, name)
    }

    /// Queues an in-app toast. `kind` is "info" or "error" (styling only), gated
    /// by `category`. Safe to call from any thread — hops to the main thread.
    pub fn show(app: &AppHandle, kind: &str, category: ToastCategory, title: &str, body: &str) {
        show_with_icon(app, kind, category, title, body, None);
    }

    /// Same as `show`, plus a specific icon data URL (already resolved by the
    /// caller — see `show_for_game` for the common "look it up by game name"
    /// case).
    pub fn show_with_icon(app: &AppHandle, kind: &str, category: ToastCategory, title: &str, body: &str, icon: Option<String>) {
        if !category.enabled(app) {
            return;
        }
        let app2 = app.clone();
        let payload = ToastPayload {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            kind: kind.to_string(),
            category,
            title: title.to_string(),
            body: body.to_string(),
            icon,
        };
        let _ = app.run_on_main_thread(move || {
            if let Err(e) = ensure_window(&app2) {
                log::warn!("toast overlay could not be created: {e}");
                return;
            }
            // Lock held across the ready-check AND the push — see WINDOW_READY's
            // doc comment for the stranded-toast race this prevents.
            let queue = PENDING.lock().unwrap();
            if WINDOW_READY.load(Ordering::SeqCst) {
                drop(queue);
                let _ = app2.emit("show-toast", payload);
            } else {
                let mut queue = queue;
                queue.push(payload);
            }
        });
    }

    /// Convenience for the common case: a toast about a specific game, whose
    /// icon (if already cached locally) should show alongside it.
    pub fn show_for_game(app: &AppHandle, kind: &str, category: ToastCategory, title: &str, body: &str, game_name: &str) {
        let icon = game_icon_data_url(app, game_name);
        show_with_icon(app, kind, category, title, body, icon);
    }
}
