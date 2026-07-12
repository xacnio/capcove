// Side-effect import: tags <html> with `is-macos` early so global CSS can
// disable backdrop-filter there — unstable on macOS's transparent windows,
// causing rendering glitches or crashes under sustained blur.
export const isMac = typeof navigator !== "undefined" && /Mac/.test(navigator.userAgent);

if (isMac && typeof document !== "undefined") {
  document.documentElement.classList.add("is-macos");
}
