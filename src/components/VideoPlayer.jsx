import { useCallback, useEffect, useRef, useState } from "react";
import { MdPlayArrow, MdPause, MdVolumeUp, MdVolumeDown, MdVolumeOff, MdFullscreen, MdFullscreenExit, MdReplay10, MdForward10, MdContentCut, MdTune, MdKeyboardArrowDown } from "react-icons/md";
import { invoke, convertFileSrc, listen } from "../lib/tauri.js";
import AdvancedExportModal from "./AdvancedExportModal.jsx";

// Keyed by `name` (the recording's relative filename) — the waveform PNG
// never changes once finished, so reopening the file reuses this cache.
const waveformCache = new Map();

function fmt(t) {
  if (!Number.isFinite(t)) return "0:00";
  const total = Math.max(0, Math.floor(t));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const mm = h > 0 ? String(m).padStart(2, "0") : String(m);
  const ss = String(s).padStart(2, "0");
  return h > 0 ? `${h}:${mm}:${ss}` : `${mm}:${ss}`;
}

// Same as `fmt`, plus the exact frame number — used while the trim tool is
// zoomed in, where individual frames are actually distinguishable.
function fmtFrame(t, fps) {
  if (!Number.isFinite(t) || !(fps > 0)) return fmt(t);
  const frame = Math.round(t * fps);
  return `${fmt(t)} · f${frame}`;
}

// Scrubber hover-preview thumbnails. Deliberately in-memory only — a hidden
// `<video>` seeks to the hovered time, draws a frame to an offscreen
// `<canvas>`, and caches the data URL; nothing ever touches disk.
const PREVIEW_BUCKET_SECS = 2; // groups nearby hover positions onto one cached frame
const PREVIEW_CACHE_MAX = 120; // caps memory for a long scrubbing session
// Inline pixel dimensions, not a Tailwind arbitrary-value class — those
// depend on the JIT scanner having generated the rule at build time.
const PREVIEW_BOX_W = 128;
const PREVIEW_BOX_H = 72;

/// Custom video player with an always-visible control bar (play/pause, ±10s,
/// volume, timeline, fullscreen), replacing the native `<video controls>` UI.
const VOLUME_KEY = "capcove.playerVolume";
const MUTED_KEY = "capcove.playerMuted";
const DISPLAY_MODE_KEY = "capcove.playerDisplayMode";

export default function VideoPlayer({ src, name, path, t, className = "", autoPlay = false, onClose, onClipSaved, extraControls }) {
  const videoRef = useRef(null);
  const containerRef = useRef(null);
  const barRef = useRef(null);
  const controlBarRef = useRef(null);
  const volumeBarRef = useRef(null);

  const [playing, setPlaying] = useState(false);
  const [current, setCurrent] = useState(0);
  const [duration, setDuration] = useState(0);
  const [volume, setVolume] = useState(() => {
    const saved = parseFloat(localStorage.getItem(VOLUME_KEY));
    return Number.isFinite(saved) ? Math.min(1, Math.max(0, saved)) : 0.5;
  });
  const [muted, setMuted] = useState(() => localStorage.getItem(MUTED_KEY) === "1");
  const [fullscreen, setFullscreen] = useState(false);
  const [dragging, setDragging] = useState(false);
  const [volumeDragging, setVolumeDragging] = useState(false);
  // "fit" scales up to fill the available box (can upscale past the source's
  // real resolution on a big/maximized window); "original" shows it at its
  // actual pixel size instead, scrolling if that's bigger than the box.
  const [displayMode, setDisplayMode] = useState(() => localStorage.getItem(DISPLAY_MODE_KEY) || "fit");
  const [videoNativeSize, setVideoNativeSize] = useState(null); // {w, h} once metadata loads

  // Grabs keyboard focus as soon as a video opens, so space/arrows/etc.
  // (see `onKeyDown` below) work immediately without an extra click.
  useEffect(() => { containerRef.current?.focus(); }, []);

  // Inline trim tool — start/end handles on the same bar. `trimEnd === null`
  // means "not yet initialized", so it can pick up `duration` once known.
  const [trimStart, setTrimStart] = useState(0);
  const [trimEnd, setTrimEnd] = useState(null);
  const [trimDragging, setTrimDragging] = useState(null); // "start" | "end" | null
  const [trimZoomWindow, setTrimZoomWindow] = useState(null); // { center, halfWindow } while holding still
  const [savingClip, setSavingClip] = useState(false);
  const [saveClipError, setSaveClipError] = useState("");
  const [saveProgress, setSaveProgress] = useState(null); // 0-100 while savingClip, else null
  const [advancedExportOpen, setAdvancedExportOpen] = useState(false);
  const [saveMenuOpen, setSaveMenuOpen] = useState(false);
  const saveMenuRef = useRef(null);
  useEffect(() => {
    if (!saveMenuOpen) return undefined;
    const onClickOutside = (e) => { if (saveMenuRef.current && !saveMenuRef.current.contains(e.target)) setSaveMenuOpen(false); };
    document.addEventListener("mousedown", onClickOutside);
    return () => document.removeEventListener("mousedown", onClickOutside);
  }, [saveMenuOpen]);
  const trimStartRef = useRef(0);
  const trimEndRef = useRef(0);
  const trimDragRef = useRef(null);
  // "full" plays normally, ignoring the trim range. "selection" always
  // (re)starts playback from the trim start and auto-stops at its end —
  // touching either handle switches into it automatically (see
  // `onTrimHandleDown`); the switch below lets it be picked by hand too.
  const [playMode, setPlayMode] = useState("full");
  const playModeRef = useRef("full");
  useEffect(() => { playModeRef.current = playMode; }, [playMode]);

  // Frame rate, for frame-accurate stepping once the trim tool is zoomed in
  // (see `timeFromClientXZoomAware`). `probe_video` is the same call the
  // full editor uses.
  const [videoFps, setVideoFps] = useState(0);
  useEffect(() => {
    setVideoFps(0);
    if (!path) return undefined;
    let cancelled = false;
    invoke("probe_video", { path })
      .then((info) => { if (!cancelled && info?.fps > 0) setVideoFps(info.fps); })
      .catch(() => {});
    return () => { cancelled = true; };
  }, [path]);

  // `ensure_playable_video` (video_thumb.rs) probes the real container and remuxes
  // if needed; `resolvedSrc` starts null so we never point at a broken source.
  const [resolvedSrc, setResolvedSrc] = useState(null);
  useEffect(() => {
    let cancelled = false;
    setResolvedSrc(null);
    if (!name) { setResolvedSrc(src); return undefined; }
    invoke("ensure_playable_video", { name })
      .then((p) => { if (!cancelled) setResolvedSrc(p ? convertFileSrc(p) : src); })
      .catch(() => { if (!cancelled) setResolvedSrc(src); });
    return () => { cancelled = true; };
  }, [src, name]);

  // Scrub hover-preview — see `PREVIEW_BUCKET_SECS`'s doc comment.
  const previewVideoRef = useRef(null); // hover-driven, on-demand captures
  const prefetchVideoRef = useRef(null); // idle background fill — see the prefetch effect below
  const previewCanvasRef = useRef(null); // lazy offscreen canvas, never in the DOM
  const previewCacheRef = useRef(new Map());
  const previewSeqRef = useRef(0); // discards a stale capture superseded by a newer hover
  const scrubDebounceRef = useRef(null);
  const scrubTimeRef = useRef(null); // mirrors `scrubTime` for the prefetch loop
  const prefetchAnchorRef = useRef(0); // last hover position, else playback position
  const [scrubTime, setScrubTime] = useState(null); // hovered/dragged time, or null when not scrubbing
  const [scrubClientX, setScrubClientX] = useState(0); // raw viewport X for the tooltip
  const [previewUrl, setPreviewUrl] = useState(null);

  // Static waveform of the whole audio track, generated server-side
  // (ffmpeg's `showwavespic`, see `get_video_waveform` in `video_thumb.rs`)
  // rather than decoding the file in the browser. `name` is optional.
  const [waveformUrl, setWaveformUrl] = useState(() => (name ? waveformCache.get(name) ?? null : null));
  useEffect(() => {
    if (!name || waveformCache.has(name)) return;
    let cancelled = false;
    invoke("get_video_waveform", { name })
      .then((b64) => `data:image/png;base64,${b64}`)
      .catch(() => null)
      .then((url) => {
        waveformCache.set(name, url);
        if (!cancelled) setWaveformUrl(url);
      });
    return () => { cancelled = true; };
  }, [name]);

  // Precise waveform for the trim tool's zoomed-in view — the full-track
  // image above is a fixed 1000px wide regardless of duration, so stretching
  // a sliver of it to fill the bar just blows up a handful of source pixels
  // into a blank smear. Re-rendered once per zoom (the window doesn't move
  // during a single drag, only the handle inside it does — see
  // `onTrimHandleDown`), not on every pointer move.
  const [zoomWaveformUrl, setZoomWaveformUrl] = useState(null);
  useEffect(() => {
    if (!trimZoomWindow || !name) { setZoomWaveformUrl(null); return undefined; }
    let cancelled = false;
    const vs = Math.max(0, trimZoomWindow.center - trimZoomWindow.halfWindow);
    const ve = Math.min(duration, trimZoomWindow.center + trimZoomWindow.halfWindow);
    invoke("get_video_waveform_range", { name, startMs: Math.round(vs * 1000), endMs: Math.round(ve * 1000) })
      .then((b64) => { if (!cancelled) setZoomWaveformUrl(`data:image/png;base64,${b64}`); })
      .catch(() => {});
    return () => { cancelled = true; };
  }, [trimZoomWindow, name, duration]);

  // Tracks a deliberate pause (the button, spacebar, or letting it play
  // through to the end) — the trim tool's drag-release preview (see the
  // trim-drag effect below) skips auto-playing when this is set, so
  // adjusting a handle never overrides a pause the user asked for.
  const userPausedRef = useRef(false);

  const togglePlay = useCallback(() => {
    const v = videoRef.current;
    if (!v) return;
    userPausedRef.current = !v.paused;
    if (v.paused) {
      // Selection mode always (re)starts from the trim start — including
      // right after it just auto-stopped at the end, so hitting play again
      // replays the same selection instead of being stuck at its end.
      if (playModeRef.current === "selection") {
        v.currentTime = trimStartRef.current;
        setCurrent(trimStartRef.current);
      }
      v.play();
    } else {
      v.pause();
    }
  }, []);

  const seekBy = useCallback((delta) => {
    const v = videoRef.current;
    if (!v) return;
    v.currentTime = Math.min(Math.max(0, v.currentTime + delta), v.duration || 0);
  }, []);

  const seekToClientX = useCallback((clientX) => {
    const v = videoRef.current;
    const bar = barRef.current;
    if (!v || !bar || !duration) return;
    const rect = bar.getBoundingClientRect();
    const pct = Math.min(1, Math.max(0, (clientX - rect.left) / rect.width));
    v.currentTime = pct * duration;
    setCurrent(pct * duration);
  }, [duration]);

  const timeFromClientX = useCallback((clientX) => {
    const bar = barRef.current;
    if (!bar || !duration) return null;
    const rect = bar.getBoundingClientRect();
    const frac = Math.min(1, Math.max(0, (clientX - rect.left) / rect.width));
    return frac * duration;
  }, [duration]);

  const requestPreviewFrame = useCallback((time) => {
    const bucket = Math.round(time / PREVIEW_BUCKET_SECS);
    const cached = previewCacheRef.current.get(bucket);
    if (cached) { setPreviewUrl(cached); return; }
    const v = previewVideoRef.current;
    if (!v || !resolvedSrc) return;
    const seq = ++previewSeqRef.current;
    const onSeeked = () => {
      v.removeEventListener("seeked", onSeeked);
      if (previewSeqRef.current !== seq) return; // a later hover already moved on
      if (!previewCanvasRef.current) previewCanvasRef.current = document.createElement("canvas");
      const canvas = previewCanvasRef.current;
      canvas.width = v.videoWidth || 160;
      canvas.height = v.videoHeight || 90;
      canvas.getContext("2d").drawImage(v, 0, 0, canvas.width, canvas.height);
      let url;
      try { url = canvas.toDataURL("image/jpeg", 0.6); } catch { return; }
      const cache = previewCacheRef.current;
      cache.set(bucket, url);
      if (cache.size > PREVIEW_CACHE_MAX) cache.delete(cache.keys().next().value);
      setPreviewUrl(url);
    };
    const seekNow = () => {
      v.removeEventListener("loadedmetadata", seekNow);
      v.addEventListener("seeked", onSeeked, { once: true });
      v.currentTime = time;
    };
    if (!v.src) {
      v.src = resolvedSrc;
      v.addEventListener("loadedmetadata", seekNow, { once: true });
      v.load();
    } else {
      seekNow();
    }
  }, [resolvedSrc]);

  // Shared by hover-scrub and trim-handle dragging — both just need to feed
  // in an already-resolved time (trim dragging maps clientX through its own
  // zoom-aware calc, not `timeFromClientX`).
  const updateScrubPreviewForTime = useCallback((t, clientX) => {
    setScrubTime(t);
    setScrubClientX(clientX);
    scrubTimeRef.current = t;
    prefetchAnchorRef.current = t;
    clearTimeout(scrubDebounceRef.current);
    // A cache hit shows immediately; only an actual miss still gets debounced.
    const bucket = Math.round(t / PREVIEW_BUCKET_SECS);
    const cached = previewCacheRef.current.get(bucket);
    if (cached) { setPreviewUrl(cached); return; }
    scrubDebounceRef.current = setTimeout(() => requestPreviewFrame(t), 80);
  }, [requestPreviewFrame]);

  // Hover-intent debounce — without it, sweeping the mouse across the bar
  // would fire a seek+capture for every pixel crossed.
  const updateScrubPreview = useCallback((clientX) => {
    const t = timeFromClientX(clientX);
    if (t == null) return;
    updateScrubPreviewForTime(t, clientX);
  }, [timeFromClientX, updateScrubPreviewForTime]);

  const clearScrubPreview = useCallback(() => {
    clearTimeout(scrubDebounceRef.current);
    setScrubTime(null);
    scrubTimeRef.current = null;
    setPreviewUrl(null);
  }, []);

  // Fills the thumbnail cache in the background via a separate hidden `<video>`,
  // targeting the un-cached bucket nearest `prefetchAnchorRef` each tick.
  useEffect(() => {
    const totalBuckets = duration > 0 ? Math.ceil(duration / PREVIEW_BUCKET_SECS) : 0;
    if (totalBuckets <= 0) return undefined;
    const v = prefetchVideoRef.current;
    if (!v || !resolvedSrc) return undefined;
    let cancelled = false;
    let intervalId;

    const findNearestUncached = () => {
      const anchor = Math.round(prefetchAnchorRef.current / PREVIEW_BUCKET_SECS);
      for (let r = 0; r <= totalBuckets; r++) {
        for (const cand of (r === 0 ? [anchor] : [anchor + r, anchor - r])) {
          if (cand < 0 || cand >= totalBuckets) continue;
          if (!previewCacheRef.current.has(cand)) return cand;
        }
      }
      return null;
    };

    let busy = false;
    const captureBucket = (bucket) => {
      busy = true;
      const onSeeked = () => {
        v.removeEventListener("seeked", onSeeked);
        busy = false;
        if (cancelled) return;
        if (!previewCanvasRef.current) previewCanvasRef.current = document.createElement("canvas");
        const canvas = previewCanvasRef.current;
        canvas.width = v.videoWidth || 160;
        canvas.height = v.videoHeight || 90;
        canvas.getContext("2d").drawImage(v, 0, 0, canvas.width, canvas.height);
        try {
          const url = canvas.toDataURL("image/jpeg", 0.55);
          const cache = previewCacheRef.current;
          cache.set(bucket, url);
          if (cache.size > PREVIEW_CACHE_MAX) cache.delete(cache.keys().next().value);
          // Already hovering exactly this spot when it lands — show it.
          if (Math.round((scrubTimeRef.current ?? -1) / PREVIEW_BUCKET_SECS) === bucket) setPreviewUrl(url);
        } catch { /* unsupported canvas read — stop prefetching */ cancelled = true; clearInterval(intervalId); }
      };
      v.addEventListener("seeked", onSeeked, { once: true });
      v.currentTime = bucket * PREVIEW_BUCKET_SECS;
    };

    const startTicking = () => {
      intervalId = setInterval(() => {
        if (cancelled || busy) return;
        const b = findNearestUncached();
        if (b == null) return; // fully covered
        captureBucket(b);
      }, 120);
    };

    if (!v.src) {
      v.addEventListener("loadedmetadata", startTicking, { once: true });
      v.src = resolvedSrc;
      v.load();
    } else {
      startTicking();
    }
    return () => { cancelled = true; clearInterval(intervalId); };
  }, [duration, resolvedSrc]);

  useEffect(() => {
    if (!dragging) return;
    const onMove = (e) => { seekToClientX(e.clientX); updateScrubPreview(e.clientX); };
    const onUp = (e) => { seekToClientX(e.clientX); setDragging(false); clearScrubPreview(); };
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
    return () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
    };
  }, [dragging, seekToClientX, updateScrubPreview, clearScrubPreview]);

  // Trim range resets for each newly opened video, then picks up the full
  // duration once it's known.
  // `duration`/`current` only otherwise update once the new video's own
  // `loadedmetadata`/`timeupdate` fire — until then the previous video's
  // values stick around, so the progress bar (`toViewPct` clamps to 100)
  // reads as "stuck"/clipped at the old duration once playback moves past
  // it, even though the new video is actually playing normally underneath.
  useEffect(() => {
    setTrimStart(0); setTrimEnd(null); setSaveClipError(""); setPlayMode("full"); setVideoNativeSize(null);
    setDuration(0); setCurrent(0);
    userPausedRef.current = false;
  }, [resolvedSrc]);
  useEffect(() => {
    if (duration > 0 && trimEnd == null) setTrimEnd(duration);
  }, [duration, trimEnd]);
  const effectiveTrimEnd = trimEnd == null ? duration : trimEnd;
  useEffect(() => { trimStartRef.current = trimStart; }, [trimStart]);
  useEffect(() => { trimEndRef.current = effectiveTrimEnd; }, [effectiveTrimEnd]);

  const MIN_TRIM_GAP_S = 0.3;
  const isTrimmed = trimStart > 0.05 || effectiveTrimEnd < duration - 0.05;

  // While holding a handle still (see `onTrimHandleDown`), the *whole bar*
  // — waveform, fill, handles, dim overlay — rescales to this narrow window
  // instead of the full duration, so the same drag distance covers far less
  // time. `toViewPct` is what every position in the bar renders through.
  const viewStart = trimZoomWindow ? Math.max(0, trimZoomWindow.center - trimZoomWindow.halfWindow) : 0;
  const viewEnd = trimZoomWindow ? Math.min(duration, trimZoomWindow.center + trimZoomWindow.halfWindow) : duration;
  const viewSpan = viewEnd - viewStart || 1;
  const toViewPct = useCallback((t) => Math.min(100, Math.max(0, ((t - viewStart) / viewSpan) * 100)), [viewStart, viewSpan]);
  const inView = (t) => t >= viewStart - 0.001 && t <= viewEnd + 0.001;

  const trimStartPct = duration > 0 ? toViewPct(trimStart) : 0;
  const trimEndPct = duration > 0 ? toViewPct(effectiveTrimEnd) : 100;

  // Same idea as `timeFromClientX`, but maps the bar's full width to a
  // narrow window around `zoom.center` instead of the whole duration —
  // holding a handle still zooms in for finer control (see `onTrimHandleDown`).
  // Once zoomed, the result also snaps to the nearest real video frame
  // (when the frame rate is known) so dragging steps frame by frame instead
  // of by arbitrary fractions of a second.
  const timeFromClientXZoomAware = useCallback((clientX, zoom) => {
    const bar = barRef.current;
    if (!bar || !duration) return null;
    const rect = bar.getBoundingClientRect();
    const frac = Math.min(1, Math.max(0, (clientX - rect.left) / rect.width));
    if (!zoom) return frac * duration;
    const t = Math.min(duration, Math.max(0, zoom.center + (frac - 0.5) * 2 * zoom.halfWindow));
    return videoFps > 0 ? Math.round(t * videoFps) / videoFps : t;
  }, [duration, videoFps]);

  const TRIM_HOLD_MS = 380;
  const TRIM_MOVE_CANCEL_PX = 5;
  const onTrimHandleDown = useCallback((which) => (e) => {
    e.stopPropagation();
    e.preventDefault();
    setPlayMode("selection");
    const anchorTime = which === "start" ? trimStartRef.current : trimEndRef.current;
    // A plain click (no drag) still jumps playback to that handle's time.
    const v = videoRef.current;
    if (v) { v.currentTime = anchorTime; setCurrent(anchorTime); }
    const drag = { which, startClientX: e.clientX, moved: false, zoom: null, holdTimer: null };
    drag.holdTimer = setTimeout(() => {
      if (drag.moved) return;
      // Re-read the handle's position now, not the one from the moment the
      // press started — `onMove` still nudges it (via the full, unzoomed
      // mapping) for movement under the cancel threshold, so by the time
      // this fires the handle can have drifted well past `anchorTime` (a
      // single pixel is many seconds on a long video). Centering the zoom
      // on stale data put it somewhere the handle no longer was.
      const liveTime = which === "start" ? trimStartRef.current : trimEndRef.current;
      const halfWindow = Math.min(duration / 2, Math.max(1.5, duration * 0.05));
      drag.zoom = { center: liveTime, halfWindow };
      setTrimZoomWindow(drag.zoom);
    }, TRIM_HOLD_MS);
    trimDragRef.current = drag;
    setTrimDragging(which);
  }, [duration]);

  useEffect(() => {
    if (!trimDragging) return undefined;
    const onMove = (e) => {
      const drag = trimDragRef.current;
      if (!drag) return;
      if (!drag.moved && Math.abs(e.clientX - drag.startClientX) > TRIM_MOVE_CANCEL_PX) {
        drag.moved = true;
        clearTimeout(drag.holdTimer);
      }
      const t = timeFromClientXZoomAware(e.clientX, drag.zoom);
      if (t == null) return;
      if (drag.which === "start") {
        setTrimStart(Math.max(0, Math.min(t, trimEndRef.current - MIN_TRIM_GAP_S)));
      } else {
        setTrimEnd(Math.min(duration, Math.max(t, trimStartRef.current + MIN_TRIM_GAP_S)));
      }
      const v = videoRef.current;
      if (v) { v.currentTime = t; setCurrent(t); }
      updateScrubPreviewForTime(t, e.clientX);
    };
    const onUp = () => {
      const drag = trimDragRef.current;
      if (drag) clearTimeout(drag.holdTimer);
      trimDragRef.current = null;
      setTrimDragging(null);
      setTrimZoomWindow(null);
      clearScrubPreview();
      // Releasing either handle after an actual drag previews the selection:
      // play from its start, auto-stopping at its end (see the `<video>`'s
      // onTimeUpdate). A plain click with no movement just seeks (handled
      // in `onTrimHandleDown`) — it shouldn't also start playback. Neither
      // should this, if the video is paused because the user asked for
      // that — adjusting a handle shouldn't override a deliberate pause.
      if (drag?.moved && !userPausedRef.current) {
        const v = videoRef.current;
        if (v) {
          v.currentTime = trimStartRef.current;
          setCurrent(trimStartRef.current);
          v.play().catch(() => {});
        }
      }
    };
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
    return () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
    };
  }, [trimDragging, duration, timeFromClientXZoomAware, updateScrubPreviewForTime, clearScrubPreview]);

  useEffect(() => {
    if (!savingClip) { setSaveProgress(null); return undefined; }
    setSaveProgress(0);
    let unlisten;
    (async () => {
      unlisten = await listen("trim-export-progress", (e) => {
        const { done_ms, total_ms } = e.payload;
        setSaveProgress(total_ms > 0 ? Math.min(100, Math.round((done_ms / total_ms) * 100)) : 0);
      });
    })();
    return () => unlisten?.();
  }, [savingClip]);

  const saveClip = useCallback(async () => {
    if (!path || savingClip) return;
    setSavingClip(true);
    setSaveClipError("");
    try {
      const clip = await invoke("export_trim_clip", {
        path,
        startMs: Math.round(trimStart * 1000),
        endMs: Math.round(effectiveTrimEnd * 1000),
        advanced: null,
      });
      onClipSaved?.(clip);
    } catch (e) {
      setSaveClipError(String(e));
    } finally {
      setSavingClip(false);
    }
  }, [path, savingClip, trimStart, effectiveTrimEnd, onClipSaved]);

  useEffect(() => {
    const onFsChange = () => setFullscreen(!!document.fullscreenElement);
    document.addEventListener("fullscreenchange", onFsChange);
    return () => document.removeEventListener("fullscreenchange", onFsChange);
  }, []);

  const toggleFullscreen = () => {
    if (!containerRef.current) return;
    if (document.fullscreenElement) document.exitFullscreen();
    else containerRef.current.requestFullscreen();
  };

  const toggleMute = () => {
    const v = videoRef.current;
    if (!v) return;
    v.muted = !v.muted;
    setMuted(v.muted);
    localStorage.setItem(MUTED_KEY, v.muted ? "1" : "0");
  };

  const changeDisplayMode = (mode) => {
    setDisplayMode(mode);
    localStorage.setItem(DISPLAY_MODE_KEY, mode);
  };

  const applyVolume = useCallback((next) => {
    const clamped = Math.min(1, Math.max(0, next));
    const v = videoRef.current;
    if (v) { v.volume = clamped; v.muted = clamped === 0; }
    setVolume(clamped);
    setMuted(clamped === 0);
    localStorage.setItem(VOLUME_KEY, String(clamped));
    localStorage.setItem(MUTED_KEY, clamped === 0 ? "1" : "0");
  }, []);

  const setVolumeFromClientX = useCallback((clientX) => {
    const bar = volumeBarRef.current;
    if (!bar) return;
    const rect = bar.getBoundingClientRect();
    applyVolume((clientX - rect.left) / rect.width);
  }, [applyVolume]);

  // Scrolling always sets an explicit level (and un-mutes to it) rather than
  // stepping from 0 while muted — same convention as most players' volume.
  const VOLUME_WHEEL_STEP = 0.05;
  const onVolumeWheel = useCallback((e) => {
    e.preventDefault();
    applyVolume(volume + (e.deltaY < 0 ? VOLUME_WHEEL_STEP : -VOLUME_WHEEL_STEP));
  }, [applyVolume, volume]);

  useEffect(() => {
    if (!volumeDragging) return undefined;
    const onMove = (e) => setVolumeFromClientX(e.clientX);
    const onUp = () => setVolumeDragging(false);
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
    return () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
    };
  }, [volumeDragging, setVolumeFromClientX]);

  // Applies the saved volume/mute once the element for this src actually
  // exists — the `<video>` only mounts once `resolvedSrc` resolves, and its
  // own `volume`/`muted` properties otherwise just default to 1/false.
  useEffect(() => {
    const v = videoRef.current;
    if (!v || !resolvedSrc) return;
    v.volume = volume;
    v.muted = muted;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [resolvedSrc]);

  const onKeyDown = (e) => {
    if (e.code === "Space") { e.preventDefault(); togglePlay(); }
    else if (e.code === "ArrowRight") seekBy(10);
    else if (e.code === "ArrowLeft") seekBy(-10);
    else if (e.code === "KeyF") toggleFullscreen();
    else if (e.code === "KeyM") toggleMute();
    else if (e.code === "Escape" && onClose) onClose();
  };

  const VolumeIcon = muted || volume === 0 ? MdVolumeOff : volume < 0.5 ? MdVolumeDown : MdVolumeUp;
  const pct = duration > 0 ? toViewPct(current) : 0;

  return (
    <div
      ref={containerRef}
      tabIndex={0}
      onKeyDown={onKeyDown}
      className={`relative flex min-h-0 flex-col bg-black outline-none ${className}`}
    >
      {/* `flex-1` (not a fixed `aspect-video` box) so this always fills
          whatever room the control bar leaves, in both modes — otherwise a
          short/small video (fit) or a small source resolution (original)
          just shrinks to its own content size and leaves dead space below
          instead of the control bar staying pinned to the bottom. This
          outer box never scrolls itself — only the inner layer below does,
          in "original size" mode — so the play button/overlays stay
          anchored to the visible viewport instead of scrolling out of view
          with an oversized video. */}
      <div className={`relative min-h-0 flex-1 bg-black ${displayMode === "original" ? "" : "pb-2"}`}>
        {/* `object-contain` keeps "fit" letterboxed to this box's actual
            shape. "Original size" instead pins the video to its real pixel
            dimensions and centers it with `grid place-items-center` —
            unlike flex centering, that doesn't clip the start of the
            content once it's bigger than the box and has to scroll. */}
        <div className={displayMode === "original" ? "absolute inset-0 overflow-auto grid place-items-center" : "absolute inset-0"}>
          {resolvedSrc ? (
            <video
              ref={videoRef}
              src={resolvedSrc}
              autoPlay={autoPlay}
              style={displayMode === "original" && videoNativeSize ? { width: videoNativeSize.w, height: videoNativeSize.h } : undefined}
              className={displayMode === "original" ? "block max-w-none" : "h-full w-full object-contain"}
              onClick={togglePlay}
              onDoubleClick={toggleFullscreen}
              onPlay={() => setPlaying(true)}
              onPause={() => setPlaying(false)}
              onTimeUpdate={(e) => {
                const t = e.currentTarget.currentTime;
                if (playModeRef.current === "selection" && t >= trimEndRef.current - 0.02) {
                  e.currentTarget.pause();
                  e.currentTarget.currentTime = trimEndRef.current;
                  setCurrent(trimEndRef.current);
                  return;
                }
                setCurrent(t);
                if (scrubTimeRef.current == null) prefetchAnchorRef.current = t;
              }}
              onLoadedMetadata={(e) => {
                setDuration(e.currentTarget.duration);
                setVideoNativeSize({ w: e.currentTarget.videoWidth, h: e.currentTarget.videoHeight });
              }}
              onVolumeChange={(e) => { setVolume(e.currentTarget.volume); setMuted(e.currentTarget.muted); }}
            />
          ) : (
            <div className="flex h-full w-full items-center justify-center">
              <div className="h-8 w-8 animate-spin rounded-full border-2 border-stone-600 border-t-stone-200" />
            </div>
          )}
        </div>

        {/* Scrub-preview capture sources — kept technically visible
            (opacity-0, not display:none) since display:none video can stop
            decoding in some engines. Never played, only seeked. */}
        <video ref={previewVideoRef} muted crossOrigin="anonymous" className="pointer-events-none absolute left-0 top-0 h-px w-px opacity-0" />
        <video ref={prefetchVideoRef} muted crossOrigin="anonymous" className="pointer-events-none absolute left-0 top-0 h-px w-px opacity-0" />

        {!playing && (
          // Hit area is just the circle itself, not the whole box — a
          // full-cover button here would sit on top of (and swallow clicks
          // meant for) the inner layer's scrollbar in "original size" mode.
          <button
            onClick={togglePlay}
            className="absolute left-1/2 top-1/2 flex h-16 w-16 -translate-x-1/2 -translate-y-1/2 items-center justify-center rounded-full bg-black/50 text-white backdrop-blur-sm transition hover:scale-105 hover:bg-black/60"
          >
            <MdPlayArrow size={34} />
          </button>
        )}

        {/* Scrub hover-preview tooltip — horizontally clamped against the
            player's bounding rect. This box lives inside the video area
            itself (not the outer container that also holds the control
            bar), so a small constant gap is enough — its own bottom edge
            already sits right where the control bar begins. */}
        {scrubTime != null && (() => {
          const rect = containerRef.current?.getBoundingClientRect();
          const boxHalf = PREVIEW_BOX_W / 2;
          const idealLeft = rect ? scrubClientX - rect.left : 0;
          const left = rect ? Math.min(Math.max(idealLeft, boxHalf), rect.width - boxHalf) : idealLeft;
          return (
            <div className="pointer-events-none absolute z-30 -translate-x-1/2" style={{ left, bottom: 10 }}>
              {previewUrl && (
                <div
                  className="overflow-hidden rounded-md border border-white/15 bg-black shadow-lg"
                  style={{ width: PREVIEW_BOX_W, height: PREVIEW_BOX_H, marginBottom: 4 }}
                >
                  <img src={previewUrl} alt="" style={{ display: "block", width: PREVIEW_BOX_W, height: PREVIEW_BOX_H, objectFit: "cover" }} />
                </div>
              )}
              <div className="rounded bg-black/85 px-1.5 py-0.5 text-center text-[10px] font-mono text-white">
                {trimZoomWindow ? fmtFrame(scrubTime, videoFps) : fmt(scrubTime)}
              </div>
            </div>
          );
        })()}
      </div>

      <div ref={controlBarRef} className="shrink-0 bg-stone-900 px-4 pb-3 pt-3.5">
        <div
          ref={barRef}
          onPointerDown={(e) => { setDragging(true); seekToClientX(e.clientX); }}
          onMouseMove={(e) => { if (!dragging) updateScrubPreview(e.clientX); }}
          onMouseLeave={() => { if (!dragging) clearScrubPreview(); }}
          className={`group relative mb-2.5 cursor-pointer overflow-hidden bg-stone-950 ${
            name ? "h-10 rounded-lg" : "h-2.5 rounded-full"
          }`}
        >
          {/* Sized off the `name` prop, not `waveformUrl` — the bar is
              already at its final height on first paint; only the image
              itself fades in once it arrives. Hidden while zoomed: the
              source PNG is a fixed 1000px wide regardless of duration, so
              stretching a sliver of it to fill the bar would just blow up a
              handful of source pixels into a misleading smear rather than
              anything resembling the real waveform there. */}
          {name && (
            <img
              src={waveformUrl || undefined}
              alt=""
              className={`pointer-events-none absolute inset-y-0 left-0 h-full w-full transition-opacity duration-300 ${waveformUrl && !trimZoomWindow ? "opacity-100" : "opacity-0"}`}
              style={{ objectFit: "fill", mixBlendMode: "screen" }}
            />
          )}
          {/* Real render of just the zoomed window, fetched fresh per zoom
              (see `get_video_waveform_range`) — crossfades in once it
              arrives, which is also what reads as the "zoom" animating. */}
          {name && trimZoomWindow && (
            <img
              src={zoomWaveformUrl || undefined}
              alt=""
              className={`pointer-events-none absolute inset-y-0 left-0 h-full w-full scale-100 transition-all duration-300 ease-out ${zoomWaveformUrl ? "opacity-100" : "scale-95 opacity-0"}`}
              style={{ objectFit: "fill", mixBlendMode: "screen" }}
            />
          )}
          {/* Selected trim range — a different hue than the accent-colored
              playback fill below, so "kept" and "already played" don't
              blend into the same color where they overlap. */}
          {isTrimmed && (
            <div
              className="pointer-events-none absolute inset-y-0 bg-amber-400/25 ring-1 ring-inset ring-amber-400/50"
              style={{ left: `${trimStartPct}%`, width: `${trimEndPct - trimStartPct}%` }}
            />
          )}
          <div className={`absolute inset-y-0 left-0 ${name ? "bg-accent-400/40" : "rounded-full bg-accent-400"}`} style={{ width: `${pct}%` }} />
          <div
            className="absolute inset-y-0 w-1 -translate-x-1/2 bg-accent-400 opacity-0 transition-opacity group-hover:opacity-100"
            style={{ left: `${pct}%` }}
          />

          {/* Trim tool: dims the parts that won't be kept, plus a start/end
              handle pair. Handles stop propagation so they don't also
              trigger the bar's own seek-drag. Positions are all in the
              same zoomed view-space as the waveform above, so they stay
              lined up; a handle currently outside that view just hides
              instead of pinning to an edge it isn't really at. */}
          <div className="pointer-events-none absolute inset-y-0 left-0 bg-black/70" style={{ width: `${trimStartPct}%` }} />
          <div className="pointer-events-none absolute inset-y-0 right-0 bg-black/70" style={{ width: `${100 - trimEndPct}%` }} />
          {inView(trimStart) && (
            <div
              onPointerDown={onTrimHandleDown("start")}
              className="group/handle absolute inset-y-0 z-20 flex w-4 -translate-x-1/2 items-center justify-center cursor-ew-resize"
              style={{ left: `${trimStartPct}%` }}
            >
              <div className="h-full w-1.5 rounded-full bg-white shadow-[0_0_0_1px_rgba(0,0,0,0.7)] ring-2 ring-accent-400 transition group-hover/handle:ring-[3px]" />
            </div>
          )}
          {inView(effectiveTrimEnd) && (
            <div
              onPointerDown={onTrimHandleDown("end")}
              className="group/handle absolute inset-y-0 z-20 flex w-4 -translate-x-1/2 items-center justify-center cursor-ew-resize"
              style={{ left: `${trimEndPct}%` }}
            >
              <div className="h-full w-1.5 rounded-full bg-white shadow-[0_0_0_1px_rgba(0,0,0,0.7)] ring-2 ring-accent-400 transition group-hover/handle:ring-[3px]" />
            </div>
          )}
        </div>

        {saveClipError && (
          <div className="mb-1.5 text-[11px] text-red-400">{t?.("videoEditor.trim.error")}{saveClipError}</div>
        )}

        <div className="flex items-center gap-1 text-white">
          <button onClick={togglePlay} className="flex h-8 w-8 items-center justify-center rounded-full hover:bg-white/10 transition">
            {playing ? <MdPause size={20} /> : <MdPlayArrow size={20} />}
          </button>
          <button onClick={() => seekBy(-10)} className="flex h-8 w-8 items-center justify-center rounded-full hover:bg-white/10 transition">
            <MdReplay10 size={18} />
          </button>
          <button onClick={() => seekBy(10)} className="flex h-8 w-8 items-center justify-center rounded-full hover:bg-white/10 transition">
            <MdForward10 size={18} />
          </button>

          <div onWheel={onVolumeWheel} className="flex items-center">
            <button onClick={toggleMute} className="flex h-8 w-8 items-center justify-center rounded-full hover:bg-white/10 transition">
              <VolumeIcon size={18} />
            </button>
            <div
              ref={volumeBarRef}
              onPointerDown={(e) => { setVolumeDragging(true); setVolumeFromClientX(e.clientX); }}
              className="group/vol relative flex h-8 w-16 shrink-0 cursor-pointer items-center"
            >
              <div className="h-1 w-full overflow-hidden rounded-full bg-white/20">
                <div className="h-full rounded-full bg-white" style={{ width: `${(muted ? 0 : volume) * 100}%` }} />
              </div>
              <div
                className={`absolute top-1/2 h-2.5 w-2.5 -translate-x-1/2 -translate-y-1/2 rounded-full bg-white shadow transition-opacity ${
                  volumeDragging ? "opacity-100" : "opacity-0 group-hover/vol:opacity-100"
                }`}
                style={{ left: `${(muted ? 0 : volume) * 100}%` }}
              />
            </div>
          </div>

          <span className="ml-1.5 text-xs font-mono text-white/80 tabular-nums">
            {fmt(current)} / {fmt(duration)}
          </span>

          <div className="flex-1" />

          {path && (
            <>
              <button
                onClick={() => setTrimStart(Math.max(0, Math.min(current, effectiveTrimEnd - MIN_TRIM_GAP_S)))}
                title={t?.("videoEditor.trim.setStart")}
                className="flex h-8 w-8 items-center justify-center rounded-full font-mono text-sm font-bold text-white/80 transition hover:bg-white/10 hover:text-white"
              >
                [
              </button>
              <button
                onClick={() => setTrimEnd(Math.min(duration, Math.max(current, trimStart + MIN_TRIM_GAP_S)))}
                title={t?.("videoEditor.trim.setEnd")}
                className="flex h-8 w-8 items-center justify-center rounded-full font-mono text-sm font-bold text-white/80 transition hover:bg-white/10 hover:text-white"
              >
                ]
              </button>
            </>
          )}

          {isTrimmed && path && (
            <div className="flex items-center rounded-full bg-white/10 p-0.5 text-[11px] font-medium">
              <button
                onClick={() => setPlayMode("full")}
                className={`rounded-full px-2 py-1 transition ${playMode === "full" ? "bg-white/90 text-stone-950" : "text-white/60 hover:text-white/90"}`}
              >
                {t?.("videoEditor.trim.playFull")}
              </button>
              <button
                onClick={() => setPlayMode("selection")}
                className={`rounded-full px-2 py-1 transition ${playMode === "selection" ? "bg-amber-400 text-stone-950" : "text-white/60 hover:text-white/90"}`}
              >
                {t?.("videoEditor.trim.playSelection")}
              </button>
            </div>
          )}

          {extraControls}

          {path && (
            // Outer wrapper stays overflow-visible (the dropdown menu below
            // positions against it) — only the pill itself clips, for its
            // rounded corners plus the progress bar's rounded bottom edge.
            <div ref={saveMenuRef} className="relative flex h-8 items-stretch">
              <div className="flex items-stretch overflow-hidden rounded-full bg-accent-400/90 text-stone-950 transition hover:bg-accent-400">
                <button
                  onClick={isTrimmed ? saveClip : () => setAdvancedExportOpen(true)}
                  disabled={savingClip}
                  title={t?.(savingClip ? "videoEditor.trim.saving" : isTrimmed ? "videoEditor.trim.saveClip" : "videoEditor.trim.compress")}
                  className={`flex items-center gap-1.5 pl-3 pr-2 text-xs font-medium disabled:opacity-60 ${isTrimmed ? "rounded-l-full" : "rounded-full pr-3"}`}
                >
                  {savingClip ? (
                    <span className="h-3.5 w-3.5 animate-spin rounded-full border-2 border-stone-900/40 border-t-stone-950" />
                  ) : (
                    <MdContentCut size={15} />
                  )}
                  {savingClip
                    ? `${t?.("videoEditor.trim.saving")} ${saveProgress ?? 0}%`
                    : t?.(isTrimmed ? "videoEditor.trim.saveClip" : "videoEditor.trim.compress")}
                </button>
                {isTrimmed && (
                  <button
                    onClick={() => setSaveMenuOpen((o) => !o)}
                    disabled={savingClip}
                    className="flex items-center rounded-r-full border-l border-stone-950/20 px-1.5 disabled:opacity-60"
                  >
                    <MdKeyboardArrowDown size={16} />
                  </button>
                )}
              </div>
              {savingClip && (
                <div className="absolute inset-x-0 bottom-0 h-0.5 overflow-hidden rounded-full bg-stone-950/30">
                  <div className="h-full bg-stone-950/70 transition-[width]" style={{ width: `${saveProgress ?? 0}%` }} />
                </div>
              )}
              {saveMenuOpen && isTrimmed && (
                <div className="absolute bottom-full right-0 z-30 mb-1.5 w-44 rounded-xl border border-white/10 bg-stone-900 p-1 text-white shadow-xl">
                  <button
                    onClick={() => { setSaveMenuOpen(false); setAdvancedExportOpen(true); }}
                    className="flex w-full items-center gap-2 rounded-lg px-2.5 py-2 text-left text-xs transition hover:bg-white/10"
                  >
                    <MdTune size={15} className="shrink-0 text-stone-400" />
                    {t?.("videoEditor.trim.advancedExport.button")}
                  </button>
                </div>
              )}
            </div>
          )}

          <div className="flex items-center rounded-full bg-white/10 p-0.5 text-[11px] font-medium">
            <button
              onClick={() => changeDisplayMode("fit")}
              className={`rounded-full px-2 py-1 transition ${displayMode === "fit" ? "bg-white/90 text-stone-950" : "text-white/60 hover:text-white/90"}`}
            >
              {t?.("videoEditor.displayFit")}
            </button>
            <button
              onClick={() => changeDisplayMode("original")}
              className={`rounded-full px-2 py-1 transition ${displayMode === "original" ? "bg-white/90 text-stone-950" : "text-white/60 hover:text-white/90"}`}
            >
              {t?.("videoEditor.displayOriginal")}
            </button>
          </div>

          <button onClick={toggleFullscreen} className="flex h-8 w-8 items-center justify-center rounded-full hover:bg-white/10 transition">
            {fullscreen ? <MdFullscreenExit size={18} /> : <MdFullscreen size={18} />}
          </button>
        </div>
      </div>

      {advancedExportOpen && (
        <AdvancedExportModal
          path={path}
          startMs={trimStart * 1000}
          endMs={effectiveTrimEnd * 1000}
          t={t}
          onClose={() => setAdvancedExportOpen(false)}
          onExported={(clip) => onClipSaved?.(clip)}
        />
      )}
    </div>
  );
}
