import { useCallback, useEffect, useRef, useState } from "react";
import { MdPlayArrow, MdPause, MdVolumeUp, MdVolumeDown, MdVolumeOff, MdFullscreen, MdFullscreenExit, MdReplay10, MdForward10 } from "react-icons/md";

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

// A live recording's playable end keeps growing, so `buffered` (finite from
// the first packet) is used instead of `seekable`, which stays `Infinity`
// until the file finishes.
function readPlayableEnd(v) {
  try {
    if (v?.buffered && v.buffered.length > 0) {
      const end = v.buffered.end(v.buffered.length - 1);
      if (Number.isFinite(end)) return end;
    }
    if (v?.seekable && v.seekable.length > 0) {
      const end = v.seekable.end(v.seekable.length - 1);
      if (Number.isFinite(end)) return end;
    }
  } catch { /* not ready yet */ }
  return 0;
}

// How close to the live edge still counts as "live" — tolerant enough to
// absorb the ~2s keyframe snap after a jump-to-live without flickering.
const LIVE_EDGE_THRESHOLD_SECS = 4;

// "Jump to live" lands slightly behind the true edge, like a DVR-style
// player, giving the next standby swap headroom to prep before it's needed.
const LIVE_LAG_SECS = 8;

// Two `<video>` elements alternate active/standby: the standby preloads and
// catches up in the background so the visible element never blanks or
// resets when picking up file growth.
const SWAP_MARGIN_SECS = 5;
// Prevents a tight retry storm; ordering is handled by the already-swapping guard.
const SWAP_COOLDOWN_MS = 800;
// Cap on how long a swap waits for standby to catch up before abandoning the attempt.
const SWAP_READY_TIMEOUT_MS = 10000;

// Scrubber hover-preview thumbnails, kept in memory only: a hidden `<video>`
// seeks to the hovered time and draws the frame to an offscreen canvas.
const PREVIEW_BUCKET_SECS = 2; // groups nearby hover positions onto one cached frame
const PREVIEW_CACHE_MAX = 120; // caps memory for a long scrubbing session
// Inline pixel dimensions, since Tailwind's JIT can't generate arbitrary values here.
const PREVIEW_BOX_W = 128;
const PREVIEW_BOX_H = 72;

function cacheBusted(url) {
  try {
    const u = new URL(url);
    u.searchParams.set("_r", Date.now().toString());
    return u.toString();
  } catch {
    return `${url}${url.includes("?") ? "&" : "?"}_r=${Date.now()}`;
  }
}

/// DVR-style player for a recording still being written: scrubs against a growing
/// wall-clock total, shows buffered-ahead progress, and turns the live badge into
/// a "jump to live" button once scrubbed backward. Plays the raw growing MKV directly.
export default function LiveRecordingPlayer({ src, className = "", autoPlay = false, onClose, t, recordingStartedAt, frozenSecs }) {
  const videoRef0 = useRef(null);
  const videoRef1 = useRef(null);
  const getEl = (i) => (i === 0 ? videoRef0.current : videoRef1.current);

  const containerRef = useRef(null);
  const barRef = useRef(null);
  const controlBarRef = useRef(null);

  const [activeIndex, setActiveIndex] = useState(0);
  // Synchronous mirror of `activeIndex`, flipped before React re-renders —
  // event handlers must read this, not state, since the old element's queued
  // `pause` event can otherwise fire against a stale closure.
  const activeIndexRef = useRef(0);
  const [playing, setPlaying] = useState(false);
  const [current, setCurrent] = useState(0);
  // The active element's own buffered-range end; drives swap timing and the
  // buffer bar, not fit to show as "the duration".
  const [bufferedSecs, setBufferedSecs] = useState(0);
  const [nowMs, setNowMs] = useState(() => Date.now());
  // True while playback is stalled waiting for data; drives the spinner overlay.
  const [buffering, setBuffering] = useState(false);
  const [volume, setVolume] = useState(1);
  const [muted, setMuted] = useState(false);
  const [fullscreen, setFullscreen] = useState(false);
  const [dragging, setDragging] = useState(false);
  // Preview position while dragging the scrubber; null when not dragging.
  const [dragPos, setDragPos] = useState(null);

  // Scrubber hover-preview state.
  const previewVideoRef = useRef(null); // hover-driven, on-demand captures
  const prefetchVideoRef = useRef(null); // idle background fill
  const previewCanvasRef = useRef(null); // lazy offscreen canvas, never in the DOM
  const previewCacheRef = useRef(new Map());
  const previewSeqRef = useRef(0); // discards a stale capture superseded by a newer hover
  const scrubDebounceRef = useRef(null);
  const scrubTimeRef = useRef(null); // mirrors `scrubTime` for the prefetch loop, which can't depend on it directly
  const prefetchAnchorRef = useRef(0); // where to prioritize prefetching around — last hover, else playback position
  const [scrubTime, setScrubTime] = useState(null); // hovered/dragged time, or null when not scrubbing
  const [scrubClientX, setScrubClientX] = useState(0); // raw viewport X — see the tooltip's positioning comment
  const [previewUrl, setPreviewUrl] = useState(null);

  const requestPreviewFrame = useCallback((time) => {
    const bucket = Math.round(time / PREVIEW_BUCKET_SECS);
    const cached = previewCacheRef.current.get(bucket);
    if (cached) { setPreviewUrl(cached); return; }
    const v = previewVideoRef.current;
    if (!v) return;
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
    // Reload if there's no src yet or the requested time is outside this
    // connection's known range.
    if (!v.src || time > readPlayableEnd(v) || time < 0) {
      v.src = cacheBusted(src);
      v.addEventListener("loadedmetadata", seekNow, { once: true });
      v.load();
    } else {
      seekNow();
    }
  }, [src]);

  const clearScrubPreview = useCallback(() => {
    clearTimeout(scrubDebounceRef.current);
    setScrubTime(null);
    scrubTimeRef.current = null;
    setPreviewUrl(null);
  }, []);

  // Fills the thumbnail cache in the background via a separate hidden `<video>`,
  // targeting the nearest un-cached bucket, bounded by `bufferedSecs`. Depends on
  // bucket count rather than raw `bufferedSecs` to avoid restarting constantly.
  const bufferedBucketCount = bufferedSecs > 0 ? Math.ceil(bufferedSecs / PREVIEW_BUCKET_SECS) : 0;
  useEffect(() => {
    const totalBuckets = bufferedBucketCount;
    if (totalBuckets <= 0) return;
    const v = prefetchVideoRef.current;
    if (!v) return;
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
          if (Math.round((scrubTimeRef.current ?? -1) / PREVIEW_BUCKET_SECS) === bucket) setPreviewUrl(url);
        } catch { cancelled = true; clearInterval(intervalId); }
      };
      v.addEventListener("seeked", onSeeked, { once: true });
      v.currentTime = bucket * PREVIEW_BUCKET_SECS;
    };

    const startTicking = () => {
      intervalId = setInterval(() => {
        if (cancelled || busy) return;
        const b = findNearestUncached();
        if (b == null) return; // fully covered so far
        captureBucket(b);
      }, 120);
    };

    if (!v.src) {
      v.addEventListener("loadedmetadata", startTicking, { once: true });
      v.src = cacheBusted(src);
      v.load();
    } else if (Math.ceil(readPlayableEnd(v) / PREVIEW_BUCKET_SECS) < totalBuckets) {
      // Recording grew past this connection's ceiling — reload with a fresh
      // connection before continuing to prefetch.
      v.addEventListener("loadedmetadata", startTicking, { once: true });
      v.src = cacheBusted(src);
      v.load();
    } else {
      startTicking();
    }
    return () => { cancelled = true; clearInterval(intervalId); };
  }, [bufferedBucketCount, src]);

  // Shown duration tracks wall-clock elapsed time rather than buffering
  // state, ticking every second like the gallery card's own counter.
  useEffect(() => {
    if (frozenSecs != null || recordingStartedAt == null) return;
    const id = setInterval(() => setNowMs(Date.now()), 1000);
    return () => clearInterval(id);
  }, [frozenSecs, recordingStartedAt]);

  const displayEnd = frozenSecs != null
    ? frozenSecs
    : recordingStartedAt != null
      ? Math.max(0, nowMs / 1000 - recordingStartedAt)
      : bufferedSecs;

  const liveEdgeTarget = Math.max(0, displayEnd - LIVE_LAG_SECS);
  const atLiveEdge = liveEdgeTarget > 0 && liveEdgeTarget - current <= LIVE_EDGE_THRESHOLD_SECS;

  const swappingRef = useRef(false);
  const lastSwapAtRef = useRef(0);
  // Lets a user-initiated jump cancel an in-flight background swap instead
  // of being silently dropped by the already-swapping guard.
  const abandonRef = useRef(null);
  // One-shot "open at live edge" — see `slotHandlers`'s `onLoadedMetadata`.
  const didInitialLiveSeekRef = useRef(false);

  // `seekTo` is an explicit target (jump-to-live or seeking past the
  // buffer), or omitted to hand off at wherever the active element is when
  // standby finishes prepping.
  const beginSwap = useCallback((seekTo, opts = {}) => {
    if (swappingRef.current) {
      if (opts.preempt && abandonRef.current) {
        abandonRef.current("preempted by user action");
      } else {
        return;
      }
    }
    if (!opts.force && Date.now() - lastSwapAtRef.current < SWAP_COOLDOWN_MS) return;
    const activeEl = getEl(activeIndexRef.current);
    const standbyEl = getEl(activeIndexRef.current === 0 ? 1 : 0);
    if (!activeEl || !standbyEl) return;
    swappingRef.current = true;
    lastSwapAtRef.current = Date.now();
    const fixedTarget = typeof seekTo === "number" ? seekTo : null;

    // Give immediate feedback for a user-initiated jump, since the actual
    // seek can take a couple seconds: freeze the outgoing picture, snap the
    // scrubber to the destination, and spin until standby lands.
    if (opts.preempt && fixedTarget != null) {
      activeEl.pause();
      setCurrent(fixedTarget);
      setBuffering(true);
    }

    standbyEl.muted = true;
    standbyEl.playbackRate = 1;
    standbyEl.src = cacheBusted(src);
    standbyEl.load();

    let settled = false;
    let timeoutId;
    let catchUpId;
    let catchUpDeadlineId;
    let catchingUp = false;
    const cleanup = () => {
      clearTimeout(timeoutId);
      clearInterval(catchUpId);
      clearTimeout(catchUpDeadlineId);
      standbyEl.playbackRate = 1;
      standbyEl.removeEventListener("loadedmetadata", onMeta);
      standbyEl.removeEventListener("canplay", onCanPlay);
      standbyEl.removeEventListener("playing", doSwap);
    };
    // Gives up on this attempt without touching the active element; resets
    // state so the next watchdog tick can retry.
    const abandon = () => {
      if (settled) return;
      settled = true;
      cleanup();
      standbyEl.pause();
      // Undo the user-jump feedback on failure: unfreeze the old picture and
      // restore the real scrubber position.
      if (opts.preempt && fixedTarget != null) {
        setBuffering(false);
        setCurrent(activeEl.currentTime);
        if (wantPlayingRef.current) activeEl.play().catch(() => {});
      }
      abandonRef.current = null;
      swappingRef.current = false;
    };
    abandonRef.current = abandon;
    // Seek the fresh connection to the target as soon as its metadata loads
    // — otherwise Chromium only buffers around position 0 and never reaches
    // a target further in.
    const onMeta = () => {
      const wanted = fixedTarget ?? activeEl.currentTime;
      standbyEl.currentTime = wanted;
    };
    const onCanPlay = () => {
      if (settled) return;
      standbyEl.volume = activeEl.volume;
      standbyEl.play().catch(() => abandon());
    };
    const finalize = () => {
      if (settled) return;
      settled = true;
      cleanup();
      standbyEl.muted = muted;
      // The standby always plays briefly (muted, hidden) to prove it
      // decodes, but if the user had paused, the swap must still land
      // paused. `jumpToLive` is the deliberate exception.
      if (!wantPlayingRef.current) standbyEl.pause();
      // Flip the active index before pausing the old element so its queued
      // pause event doesn't see itself as still active.
      const next = activeIndexRef.current === 0 ? 1 : 0;
      activeIndexRef.current = next;
      activeEl.pause();
      setActiveIndex(next);
      setCurrent(standbyEl.currentTime);
      setBufferedSecs(readPlayableEnd(standbyEl));
      setBuffering(false);
      // The newly-active element was already playing before hand-off, so no
      // fresh `play` event fires — update the UI state explicitly.
      setPlaying(!standbyEl.paused);
      abandonRef.current = null;
      swappingRef.current = false;
    };
    // A growing MKV has no seek index yet, so the hand-off seek lands at the keyframe
    // before the target; the hidden standby fast-forwards to catch up before swapping.
    const doSwap = () => {
      if (settled || catchingUp) return;
      const behind = fixedTarget == null ? activeEl.currentTime - standbyEl.currentTime : 0;
      // Active is now behind the captured hand-off point — the user
      // scrubbed backward mid-swap. Abandon rather than jump them forward again.
      if (fixedTarget == null && behind < -0.75) {
        abandon();
        return;
      }
      if (behind <= 0.25) { finalize(); return; }
      catchingUp = true;
      standbyEl.playbackRate = 4;
      catchUpId = setInterval(() => {
        if (settled) { clearInterval(catchUpId); return; }
        if (standbyEl.currentTime >= activeEl.currentTime - 0.05) {
          standbyEl.playbackRate = 1;
          finalize();
        }
      }, 80);
      // If catch-up drags, hand off anyway rather than never landing.
      catchUpDeadlineId = setTimeout(() => {
        if (!settled) { standbyEl.playbackRate = 1; finalize(); }
      }, 3000);
    };
    standbyEl.addEventListener("loadedmetadata", onMeta, { once: true });
    standbyEl.addEventListener("canplay", onCanPlay);
    standbyEl.addEventListener("playing", doSwap);
    timeoutId = setTimeout(abandon, SWAP_READY_TIMEOUT_MS);
  }, [activeIndex, muted, src]);

  // Polls the active element's real buffered end directly rather than
  // relying on `timeupdate`, which stops firing during a stall and only
  // ticks a few times a second otherwise.
  useEffect(() => {
    const id = setInterval(() => {
      if (swappingRef.current || !wantPlayingRef.current) return;
      const el = getEl(activeIndexRef.current);
      if (!el) return;
      const end = readPlayableEnd(el);
      if (end > 0 && el.currentTime >= end - SWAP_MARGIN_SECS) beginSwap();
    }, 300);
    return () => clearInterval(id);
  }, [activeIndex, beginSwap]);

  // Whether the user wants playback running, distinct from the element's own
  // `paused` (which also flips on stalls). Lets the watchdog tell a real
  // pause apart from playback dying on its own.
  const wantPlayingRef = useRef(autoPlay);

  // Last-resort recovery: event-driven paths here can each be silenced once,
  // so this polls every second and forces a swap if the user wants playback
  // but the playhead hasn't advanced.
  const lastWatchdogPosRef = useRef(-1);
  useEffect(() => {
    const id = setInterval(() => {
      const el = getEl(activeIndexRef.current);
      if (!el || swappingRef.current || !wantPlayingRef.current) return;
      const pos = el.currentTime;
      const advanced = pos > lastWatchdogPosRef.current + 0.05;
      lastWatchdogPosRef.current = pos;
      if (advanced) return;
      beginSwap(undefined, { force: true });
    }, 1000);
    return () => clearInterval(id);
  }, [activeIndex, beginSwap]);

  const togglePlay = useCallback(() => {
    const el = getEl(activeIndexRef.current);
    if (!el) return;
    if (el.paused) { wantPlayingRef.current = true; el.play(); }
    else { wantPlayingRef.current = false; el.pause(); }
  }, [activeIndex]);

  const jumpToLive = useCallback(() => {
    wantPlayingRef.current = true;
    beginSwap(liveEdgeTarget, { force: true, preempt: true });
  }, [beginSwap, liveEdgeTarget]);

  // A target beyond what's buffered starts a swap aimed at that point
  // instead of clamping back to the buffered range.
  const seekOrReload = useCallback((target) => {
    const el = getEl(activeIndexRef.current);
    if (!el) return;
    // Any user seek makes an in-flight swap's target stale (it would
    // otherwise land later and yank playback back) — cancel it; the runway
    // check restarts one if needed.
    if (swappingRef.current) abandonRef.current?.();
    const clamped = Math.max(0, target);
    if (clamped <= readPlayableEnd(el)) {
      el.currentTime = clamped;
      setCurrent(clamped);
    } else {
      beginSwap(clamped, { force: true, preempt: true });
    }
  }, [activeIndex, beginSwap]);

  const seekBy = useCallback((delta) => {
    const el = getEl(activeIndexRef.current);
    if (!el) return;
    seekOrReload(Math.min(el.currentTime + delta, displayEnd || el.currentTime + delta));
  }, [activeIndex, displayEnd, seekOrReload]);

  const timeFromClientX = useCallback((clientX) => {
    const bar = barRef.current;
    if (!bar || !displayEnd) return null;
    const rect = bar.getBoundingClientRect();
    const frac = Math.min(1, Math.max(0, (clientX - rect.left) / rect.width));
    return frac * displayEnd;
  }, [displayEnd]);

  // Debounces hover so sweeping across the bar doesn't fire a seek+capture
  // per pixel. Only previews up to `bufferedSecs` since later footage
  // doesn't exist on disk yet.
  const updateScrubPreview = useCallback((clientX) => {
    const t = timeFromClientX(clientX);
    if (t == null) return;
    setScrubTime(t);
    setScrubClientX(clientX);
    scrubTimeRef.current = t;
    prefetchAnchorRef.current = Math.min(t, bufferedSecs);
    clearTimeout(scrubDebounceRef.current);
    // Show a cached frame immediately instead of waiting through the debounce.
    const bucket = Math.round(Math.min(t, bufferedSecs) / PREVIEW_BUCKET_SECS);
    const cached = previewCacheRef.current.get(bucket);
    if (cached) { setPreviewUrl(cached); return; }
    scrubDebounceRef.current = setTimeout(() => requestPreviewFrame(Math.min(t, bufferedSecs)), 80);
  }, [timeFromClientX, requestPreviewFrame, bufferedSecs]);

  // Only a visual preview moves while dragging; the seek commits once on
  // release, since seeking on every pointermove made the bar fight the user.
  useEffect(() => {
    if (!dragging) return;
    const onMove = (e) => {
      const t = timeFromClientX(e.clientX);
      if (t != null) setDragPos(t);
      updateScrubPreview(e.clientX);
    };
    const onUp = (e) => {
      const t = timeFromClientX(e.clientX);
      setDragging(false);
      setDragPos(null);
      clearScrubPreview();
      if (t != null) seekOrReload(t);
    };
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
    return () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
    };
  }, [dragging, timeFromClientX, seekOrReload, updateScrubPreview, clearScrubPreview]);

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
    const el = getEl(activeIndexRef.current);
    if (!el) return;
    el.muted = !el.muted;
    setMuted(el.muted);
  };

  const onVolumeChange = (e) => {
    const el = getEl(activeIndexRef.current);
    const next = Number(e.target.value);
    if (el) { el.volume = next; el.muted = next === 0; }
    setVolume(next);
    setMuted(next === 0);
  };

  const onKeyDown = (e) => {
    if (e.code === "Space") { e.preventDefault(); togglePlay(); }
    else if (e.code === "ArrowRight") seekBy(10);
    else if (e.code === "ArrowLeft") seekBy(-10);
    else if (e.code === "KeyF") toggleFullscreen();
    else if (e.code === "KeyM") toggleMute();
    else if (e.code === "Escape" && onClose) onClose();
  };

  // Each handler checks `activeIndexRef.current === slot` so only the
  // currently-visible element's events drive shared UI state; the hidden
  // standby's prep never leaks in.
  const slotHandlers = (slot) => ({
    onClick: togglePlay,
    onDoubleClick: toggleFullscreen,
    // Opens a live view near the live edge instead of wherever the recording
    // happens to be. Only applies to the initial metadata load; standby
    // loads are positioned by `beginSwap`, and a frozen snapshot opens at 0.
    onLoadedMetadata: (e) => {
      if (activeIndexRef.current !== slot || didInitialLiveSeekRef.current) return;
      didInitialLiveSeekRef.current = true;
      if (frozenSecs != null || recordingStartedAt == null) return;
      const target = Math.max(0, Date.now() / 1000 - recordingStartedAt - LIVE_LAG_SECS);
      e.currentTarget.currentTime = target;
      setCurrent(target);
    },
    onPlay: () => { if (activeIndexRef.current === slot) setPlaying(true); },
    onPlaying: () => { if (activeIndexRef.current === slot) setBuffering(false); },
    onPause: () => { if (activeIndexRef.current === slot) setPlaying(false); },
    onTimeUpdate: (e) => {
      if (activeIndexRef.current !== slot) return;
      setCurrent(e.currentTarget.currentTime);
      if (scrubTimeRef.current == null) prefetchAnchorRef.current = e.currentTarget.currentTime;
      setBufferedSecs((prev) => Math.max(prev, readPlayableEnd(e.currentTarget)));
      setBuffering(false);
    },
    onProgress: (e) => { if (activeIndexRef.current === slot) setBufferedSecs((prev) => Math.max(prev, readPlayableEnd(e.currentTarget))); },
    onVolumeChange: (e) => { if (activeIndexRef.current === slot) { setVolume(e.currentTarget.volume); setMuted(e.currentTarget.muted); } },
    // Safety nets for when the active element hits its ceiling before the
    // runway check lands a swap in time; also raise the spinner.
    onEnded: () => {
      if (activeIndexRef.current !== slot) return;
      if (wantPlayingRef.current) setBuffering(true);
      beginSwap(undefined, { force: true });
    },
    onStalled: () => {
      if (activeIndexRef.current !== slot) return;
      if (wantPlayingRef.current) setBuffering(true);
      beginSwap();
    },
    onWaiting: () => {
      if (activeIndexRef.current !== slot) return;
      if (wantPlayingRef.current) setBuffering(true);
      beginSwap();
    },
  });

  const VolumeIcon = muted || volume === 0 ? MdVolumeOff : volume < 0.5 ? MdVolumeDown : MdVolumeUp;
  const shownCurrent = dragPos ?? current;
  const pct = displayEnd > 0 ? Math.min(100, (shownCurrent / displayEnd) * 100) : 0;
  const bufferedPct = displayEnd > 0 ? Math.min(100, (bufferedSecs / displayEnd) * 100) : 0;

  return (
    <div
      ref={containerRef}
      tabIndex={0}
      onKeyDown={onKeyDown}
      className={`relative flex min-h-0 flex-col bg-black outline-none ${className}`}
    >
      {/* min-h-0 overrides flexbox's default min-height so this can shrink
          below the video's natural aspect-ratio height instead of pushing
          the control bar out of view. object-contain keeps the video
          letterboxed once resized. */}
      <div className="relative aspect-video min-h-0 bg-black pb-2">
        {/* Both elements stay opaque; hand-off only flips z-order —
            cross-fading caused a visible darkening as both sat
            semi-transparent mid-transition. */}
        <video
          ref={videoRef0}
          src={src}
          autoPlay={autoPlay}
          className={`absolute inset-0 h-full w-full object-contain ${activeIndex === 0 ? "z-10" : "z-0 pointer-events-none"}`}
          {...slotHandlers(0)}
        />
        <video
          ref={videoRef1}
          className={`absolute inset-0 h-full w-full object-contain ${activeIndex === 1 ? "z-10" : "z-0 pointer-events-none"}`}
          {...slotHandlers(1)}
        />

        {/* Scrub-preview capture sources: kept visible at 1x1px (not
            display:none, which can stop decoding) but never rendered;
            frames are drawn to an offscreen canvas instead. */}
        <video ref={previewVideoRef} muted crossOrigin="anonymous" className="pointer-events-none absolute left-0 top-0 h-px w-px opacity-0" />
        <video ref={prefetchVideoRef} muted crossOrigin="anonymous" className="pointer-events-none absolute left-0 top-0 h-px w-px opacity-0" />

        {/* Buffering spinner, shown instead of the play button during a rebuffer. */}
        {buffering && (
          <div className="pointer-events-none absolute inset-0 z-20 flex items-center justify-center">
            <div className="h-12 w-12 animate-spin rounded-full border-[3px] border-white/25 border-t-white/90" />
          </div>
        )}

        {!playing && !buffering && (
          <button onClick={togglePlay} className="absolute inset-0 z-20 flex items-center justify-center">
            <span className="flex h-16 w-16 items-center justify-center rounded-full bg-black/50 text-white backdrop-blur-sm transition hover:scale-105 hover:bg-black/60">
              <MdPlayArrow size={34} />
            </span>
          </button>
        )}

        {/* Scrub hover-preview tooltip, clamped to the player's bounding
            rect and offset above the control bar's real height. */}
        {scrubTime != null && (() => {
          const rect = containerRef.current?.getBoundingClientRect();
          const boxHalf = PREVIEW_BOX_W / 2;
          const idealLeft = rect ? scrubClientX - rect.left : 0;
          const left = rect ? Math.min(Math.max(idealLeft, boxHalf), rect.width - boxHalf) : idealLeft;
          const bottomOffset = (controlBarRef.current?.offsetHeight ?? 64) + 10;
          return (
            <div className="pointer-events-none absolute z-30 -translate-x-1/2" style={{ left, bottom: bottomOffset }}>
              {previewUrl && (
                <div
                  className="overflow-hidden rounded-md border border-white/15 bg-black shadow-lg"
                  style={{ width: PREVIEW_BOX_W, height: PREVIEW_BOX_H, marginBottom: 4 }}
                >
                  <img src={previewUrl} alt="" style={{ display: "block", width: PREVIEW_BOX_W, height: PREVIEW_BOX_H, objectFit: "cover" }} />
                </div>
              )}
              <div className="rounded bg-black/85 px-1.5 py-0.5 text-center text-[10px] font-mono text-white">{fmt(scrubTime)}</div>
            </div>
          );
        })()}

        {/* Same "still recording" indicator as the gallery card. */}
        <span className="absolute left-3 top-3 z-20 flex items-center gap-1 rounded bg-red-600/90 px-2 py-1 text-[10px] font-bold uppercase tracking-wider text-white">
          <span className="relative flex h-1.5 w-1.5">
            <span className="absolute inline-flex h-full w-full animate-ping rounded-full bg-white opacity-70" />
            <span className="relative inline-flex h-1.5 w-1.5 rounded-full bg-white" />
          </span>
          {t ? t("gallery.card.recordingInProgress") : "Recording"}
        </span>
      </div>

      <div ref={controlBarRef} className="shrink-0 bg-stone-900 px-4 pb-3 pt-3.5">
        <div
          ref={barRef}
          onPointerDown={(e) => { setDragging(true); const t = timeFromClientX(e.clientX); if (t != null) setDragPos(t); }}
          onMouseMove={(e) => { if (!dragging) updateScrubPreview(e.clientX); }}
          onMouseLeave={() => { if (!dragging) clearScrubPreview(); }}
          className="group relative mb-2.5 h-2.5 cursor-pointer rounded-full bg-white/10"
        >
          {/* Buffered range: how far the active element has downloaded ahead of playback. */}
          <div className="absolute inset-y-0 left-0 rounded-full bg-white/30" style={{ width: `${bufferedPct}%` }} />
          <div className="absolute inset-y-0 left-0 rounded-full bg-accent-400" style={{ width: `${pct}%` }} />
          <div
            className="absolute top-1/2 h-3.5 w-3.5 -translate-y-1/2 -translate-x-1/2 rounded-full bg-accent-400 opacity-0 transition-opacity group-hover:opacity-100"
            style={{ left: `${pct}%` }}
          />
        </div>

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

          <button onClick={toggleMute} className="flex h-8 w-8 items-center justify-center rounded-full hover:bg-white/10 transition">
            <VolumeIcon size={18} />
          </button>
          <input
            type="range" min={0} max={1} step={0.01}
            value={muted ? 0 : volume}
            onChange={onVolumeChange}
            className="w-16 accent-accent-400"
          />

          <span className="ml-1.5 text-xs font-mono text-white/80 tabular-nums">{fmt(shownCurrent)} / {fmt(displayEnd)}</span>

          <div className="flex-1" />

          {/* Live pill: solid and pulsing at the live edge, becomes a
              clickable "back to live" button once scrubbed behind it. Not
              `disabled`, since that swallows clicks silently — the guard
              lives in the handler instead. */}
          <button
            onClick={() => { if (!atLiveEdge) jumpToLive(); }}
            className={`mr-1 flex items-center gap-1.5 rounded-full px-2.5 py-1 text-[11px] font-bold uppercase tracking-wide transition ${
              atLiveEdge ? "bg-red-600/90 cursor-default" : "bg-white/15 hover:bg-white/25 cursor-pointer"
            }`}
          >
            <span className="relative flex h-1.5 w-1.5">
              {atLiveEdge && <span className="absolute inline-flex h-full w-full animate-ping rounded-full bg-white opacity-70" />}
              <span className={`relative inline-flex h-1.5 w-1.5 rounded-full ${atLiveEdge ? "bg-white" : "bg-stone-400"}`} />
            </span>
            {atLiveEdge
              ? (t ? t("gallery.card.liveNow") : "LIVE")
              : (t ? t("gallery.card.backToLive") : "Back to live")}
          </button>

          <button onClick={toggleFullscreen} className="flex h-8 w-8 items-center justify-center rounded-full hover:bg-white/10 transition">
            {fullscreen ? <MdFullscreenExit size={18} /> : <MdFullscreen size={18} />}
          </button>
        </div>
      </div>
    </div>
  );
}
