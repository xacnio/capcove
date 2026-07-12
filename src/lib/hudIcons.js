// Shared between the on-screen HUD and its Settings picker so the preview matches.
// Each value is raw inner SVG markup (0 0 24 24 viewBox), fills only, for `fill-*` tinting.
export const HUD_ICONS = {
  // Recording
  dot: '<circle cx="12" cy="12" r="7"/>',
  camera: '<path d="M17 10.5V7c0-.55-.45-1-1-1H4c-.55 0-1 .45-1 1v10c0 .55.45 1 1 1h12c.55 0 1-.45 1-1v-3.5l4 4v-11l-4 4z"/>',
  square: '<rect x="6" y="6" width="12" height="12" rx="2"/>',
  controller: '<rect x="2" y="9" width="20" height="8" rx="4"/><circle cx="6" cy="17" r="3"/><circle cx="18" cy="17" r="3"/>',
  flame: '<path d="M12 2c-1.5 3.5-5 5-5 10a5 5 0 0 0 10 0c0-1.2-.4-2.2-1-3 .2 2-.9 3.5-2 3.5-1.2 0-2-1-2-2.2 0-2 1.8-3 1.8-6.3-.9 1-1.8 2.3-1.8 4.3z"/>',
  target: '<path fill-rule="evenodd" d="M12 2a10 10 0 1 0 0.01 0z M12 5a7 7 0 1 0 0.01 0z"/><circle cx="12" cy="12" r="3"/>',
  // Replay buffer
  history: '<path d="M13 3a9 9 0 0 0-9 9H1l3.89 3.89.07.14L9 12H6c0-3.87 3.13-7 7-7s7 3.13 7 7-3.13 7-7 7c-1.93 0-3.68-.79-4.94-2.06l-1.42 1.42A8.954 8.954 0 0 0 13 21a9 9 0 0 0 0-18zm-1 5v5l4.28 2.54.72-1.21-3.5-2.08V8H12z"/>',
  rewind: '<path d="M11 18V6l-8.5 6 8.5 6zm.5-6 8.5 6V6l-8.5 6z"/>',
  bolt: '<path d="M7 2v11h3v9l7-12h-4l4-8z"/>',
  sparkle: '<path d="M12 2l1.8 6.2L20 10l-6.2 1.8L12 18l-1.8-6.2L4 10l6.2-1.8z"/>',
  hourglass: '<polygon points="6,3 18,3 12,11"/><polygon points="6,21 18,21 12,13"/><rect x="6" y="2" width="12" height="1.6"/><rect x="6" y="20.4" width="12" height="1.6"/>',
  // Microphone
  mic: '<path d="M12 14a3 3 0 0 0 3-3V5a3 3 0 0 0-6 0v6a3 3 0 0 0 3 3zm5-3a5 5 0 0 1-10 0H5a7 7 0 0 0 6 6.92V21h2v-3.08A7 7 0 0 0 19 11h-2z"/>',
  mic_alt: '<path d="M12 15c1.66 0 3-1.34 3-3V6c0-1.66-1.34-3-3-3S9 4.34 9 6v6c0 1.66 1.34 3 3 3zm6-3c0 3.31-2.69 6-6 6s-6-2.69-6-6H4c0 3.53 2.61 6.43 6 6.92V21h4v-2.08c3.39-.49 6-3.39 6-6.92h-2z"/>',
  speaker: '<path d="M4 9v6h4l5 4V5L8 9H4z"/>',
  headset: '<rect x="4" y="3" width="16" height="4" rx="2"/><rect x="3" y="6" width="3.5" height="9" rx="1.6"/><rect x="17.5" y="6" width="3.5" height="9" rx="1.6"/>',
  // Shared "fun" extras — recognizable, playful, not tied to one badge.
  star: '<path d="M12 17.27L18.18 21l-1.64-7.03L22 9.24l-7.19-.61L12 2 9.19 8.63 2 9.24l5.46 4.73L5.82 21z"/>',
  heart: '<path d="M12 21.35l-1.45-1.32C5.4 15.36 2 12.28 2 8.5 2 5.42 4.42 3 7.5 3c1.74 0 3.41.81 4.5 2.09C13.09 3.81 14.76 3 16.5 3 19.58 3 22 5.42 22 8.5c0 3.78-3.4 6.86-8.55 11.54L12 21.35z"/>',
  diamond: '<polygon points="12,3 19,9 12,21 5,9"/>',
  crown: '<path d="M4 8l3 3 5-6 5 6 3-3-2 10H6z"/>',
  shield: '<path d="M12 2l7 3v6c0 5-3.5 8.5-7 10-3.5-1.5-7-5-7-10V5z"/>',
  trophy: '<path d="M7 4h10v2h3v2a4 4 0 0 1-4 4c-.6 1.6-1.9 2.8-3.5 3.3V18h3v2H8v-2h3v-2.7C9.4 14.8 8.1 13.6 7.5 12A4 4 0 0 1 4 8V6h3V4zm-3 4a2 2 0 0 0 2 2 8.6 8.6 0 0 1-.3-2H4zm16 0h-1.7a8.6 8.6 0 0 1-.3 2 2 2 0 0 0 2-2z"/>',
  rocket: '<path d="M12 2c3 2 5 6 5 10 0 2-1 4-2 5l-1-3-2 2-2-2-1 3c-1-1-2-3-2-5 0-4 2-8 5-10zm0 5a2 2 0 1 0 0 4 2 2 0 0 0 0-4zM8 17l-3 4 4-1zm8 0l3 4-4-1z"/>',
  skull: '<path d="M12 3a7 7 0 0 0-7 7v3l1.5 2v2h2v2h2v-2h3v2h2v-2h2v-2L19 13v-3a7 7 0 0 0-7-7zM9 11a1.3 1.3 0 1 1 0 2.6A1.3 1.3 0 0 1 9 11zm6 0a1.3 1.3 0 1 1 0 2.6A1.3 1.3 0 0 1 15 11z"/>',
  ghost: '<path d="M12 2a7 7 0 0 0-7 7v11l2.5-2 2 2 2.5-2 2.5 2 2-2 2.5 2V9a7 7 0 0 0-7-7zM9.5 9a1.3 1.3 0 1 1 0 2.6A1.3 1.3 0 0 1 9.5 9zm5 0a1.3 1.3 0 1 1 0 2.6A1.3 1.3 0 0 1 14.5 9z"/>',
  swords: '<path d="M3 3l7 7-1.5 1.5-7-7zM21 3l-7 7 1.5 1.5 7-7zM10 14l-6 6 1.5 1.5 6-6zm4 0l6 6-1.5 1.5-6-6z"/>',
};

// Which icon choices are offered for each badge, in picker order.
export const HUD_ICON_CHOICES = {
  recording: ["dot", "camera", "square", "controller", "flame", "target", "star", "heart", "crown", "shield", "trophy", "skull", "ghost", "swords"],
  buffer: ["history", "rewind", "bolt", "sparkle", "hourglass", "star", "rocket", "diamond"],
  mic: ["mic", "mic_alt", "speaker", "headset"],
};
