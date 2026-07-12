import { useEffect, useState } from "react";
import { invoke } from "../lib/tauri.js";

// Module-level cache so each app name is resolved at most once per session.
// Value: data URL string, or null once known-missing.
const cache = new Map();
const pending = new Map();

export function useAppIcon(appName) {
  const [icon, setIcon] = useState(() => (appName ? cache.get(appName) ?? null : null));

  useEffect(() => {
    // Clear the icon when appName goes back to null, instead of leaving the
    // last resolved one on screen.
    if (!appName) { setIcon(null); return; }
    if (cache.has(appName)) { setIcon(cache.get(appName)); return; }
    let p = pending.get(appName);
    if (!p) {
      p = invoke("get_app_icon", { appName })
        // Backend returns a complete data URL; older bare-base64 responses get the png prefix.
        .then((v) => (v.startsWith("data:") ? v : `data:image/png;base64,${v}`))
        .catch(() => null)
        .then((v) => { cache.set(appName, v); pending.delete(appName); return v; });
      pending.set(appName, p);
    }
    let cancelled = false;
    p.then((v) => { if (!cancelled) setIcon(v); });
    return () => { cancelled = true; };
  }, [appName]);

  return icon;
}

// Wide catalog cover art keyed by app display name, using the `<name>__cover`
// disk-cache key. Separate cache/pending maps from useAppIcon's.
const coverCache = new Map();
const coverPending = new Map();

export function useAppCover(appName) {
  const [cover, setCover] = useState(() => (appName ? coverCache.get(appName) ?? null : null));

  useEffect(() => {
    // Same reset as `useAppIcon`.
    if (!appName) { setCover(null); return; }
    if (coverCache.has(appName)) { setCover(coverCache.get(appName)); return; }
    let p = coverPending.get(appName);
    if (!p) {
      p = invoke("get_app_icon", { appName: `${appName}__cover` })
        .then((v) => (v.startsWith("data:") ? v : `data:image/png;base64,${v}`))
        .catch(() => null)
        .then((v) => { coverCache.set(appName, v); coverPending.delete(appName); return v; });
      coverPending.set(appName, p);
    }
    let cancelled = false;
    p.then((v) => { if (!cancelled) setCover(v); });
    return () => { cancelled = true; };
  }, [appName]);

  return cover;
}
