// withGlobalTauri:true injects the bridge into the window; using the global
// API guarantees compatibility with the running Tauri version.
const tauri = window.__TAURI__ ?? {};

// The native WebView2 context menu (Back/Refresh/Save as/Print/Inspect/...)
// has no place in a desktop app UI — every right-click menu here is either
// custom-built (CardMenu, the breadcrumb's explorer menu, the wheel's own
// dismiss) or simply not offered. Every window imports this module, so one
// listener here covers all of them instead of needing it added per-window.
// Doesn't affect devtools — F12 opens them regardless, at the WebView2 host
// level, independent of this page-level listener.
if (typeof document !== "undefined") {
  document.addEventListener("contextmenu", (e) => e.preventDefault());
}

export const invoke = (cmd, args) => tauri.core.invoke(cmd, args);
export const listen = (event, handler) => tauri.event.listen(event, handler);
export const emit = (event, payload) => tauri.event.emit(event, payload);
export const convertFileSrc = (path) => tauri.core.convertFileSrc(path);
