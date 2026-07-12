// Purely visual, shown/hidden/sized by the backend (recording/hud.rs). Plain JS, no
// React/Tailwind, to avoid `color-scheme: dark` breaking window transparency.
import { invoke, listen } from "../lib/tauri.js";
import { HUD_ICONS } from "../lib/hudIcons.js";

function setBadge(id, on, iconKey) {
  const el = document.getElementById(id);
  el.classList.toggle("on", Boolean(on));
  const svg = el.querySelector("svg");
  const markup = HUD_ICONS[iconKey] ?? "";
  if (svg.innerHTML !== markup) svg.innerHTML = markup;
}

function applyBadges(b) {
  setBadge("badge-recording", b?.recording, b?.recording_icon);
  setBadge("badge-buffer", b?.buffer, b?.buffer_icon);
  setBadge("badge-mic", b?.mic, b?.mic_icon);
}

// Initial paint pulls state directly instead of waiting for a push, avoiding
// a race where the backend emits before this window's listener is registered.
invoke("get_hud_badges").then(applyBadges).catch(() => {});
listen("hud-badges", (e) => applyBadges(e.payload));
