export const MS_STORE_URL = "https://apps.microsoft.com/detail/9NXZDV6V9HRX";

const MS_STORE_BADGE_LANG = { en: "en-us", tr: "tr" };

export function msStoreBadgeUrl(lang) {
  const code = MS_STORE_BADGE_LANG[lang] ?? MS_STORE_BADGE_LANG.en;
  return `https://get.microsoft.com/images/${code}%20dark.svg`;
}

export function detectPlatform() {
  if (typeof navigator === "undefined") return "windows";
  const ua = navigator.userAgent;
  if (/Mac/.test(ua)) return "macos";
  if (/Linux/.test(ua) && !/Android/.test(ua)) return "linux";
  return "windows";
}

// Best-effort, Chromium-only; returns null when undetectable, in which
// case callers should default to x64, never arm64.
export async function detectArch() {
  if (typeof navigator === "undefined" || !navigator.userAgentData?.getHighEntropyValues) return null;
  try {
    const { architecture } = await navigator.userAgentData.getHighEntropyValues(["architecture"]);
    if (architecture === "arm") return "arm64";
    if (architecture === "x86") return "x64";
  } catch {
    // ignore — falls through to null
  }
  return null;
}

// x64/universal first, arm64 last — picking the first match for a platform
// (e.g. the Hero CTA) should never land on arm64 by accident.
export function sortByArch(assets) {
  return [...assets].sort((a, b) => (a.arch === "arm64" ? 1 : 0) - (b.arch === "arm64" ? 1 : 0));
}

export function formatBytes(bytes) {
  if (!bytes) return "";
  const mb = bytes / (1024 * 1024);
  return mb >= 1 ? `${mb.toFixed(1)} MB` : `${(bytes / 1024).toFixed(0)} KB`;
}

const ASSET_LABELS = {
  ".exe": "Installer (.exe)",
  ".msi": "MSI (.msi)",
  ".dmg": "Disk image (.dmg)",
  ".deb": "Debian package (.deb)",
  ".rpm": "RPM package (.rpm)",
  ".appimage": "AppImage",
};

export function assetLabel(name) {
  const lower = name.toLowerCase();
  for (const [ext, label] of Object.entries(ASSET_LABELS)) {
    if (lower.endsWith(ext)) return label;
  }
  return name;
}
