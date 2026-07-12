import { invoke } from "./tauri.js";

// Routes markdown links through the system browser instead of the webview.
export function openExternalLinks(e) {
  const link = e.target.closest("a");
  if (!link) return;
  e.preventDefault();
  invoke("open_url", { url: link.href });
}
