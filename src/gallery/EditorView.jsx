import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke, listen, convertFileSrc } from "../lib/tauri.js";
import {
  MdArrowBack, MdCallSplit, MdContentCut, MdDeleteOutline, MdFileDownload, MdFolderOpen, MdMovie,
  MdPause, MdPlayArrow, MdSkipNext, MdSkipPrevious, MdVolumeOff, MdVolumeUp,
} from "react-icons/md";
import { SiYoutube } from "react-icons/si";
import YouTubeUploadModal from "../components/YouTubeUploadModal.jsx";

const clamp = (v, min, max) => Math.min(max, Math.max(min, v));
const MIN_SEG_MS = 100;

let nextSegId = 0;
const uid = () => `seg-${nextSegId++}`;

function msToClock(ms) {
  const total = Math.max(0, ms / 1000);
  const m = Math.floor(total / 60);
  const s = total % 60;
  return `${m}:${s.toFixed(2).padStart(5, "0")}`;
}

function msToTick(ms) {
  const total = Math.max(0, Math.round(ms / 1000));
  const m = Math.floor(total / 60);
  const s = total % 60;
  return `${m}:${String(s).padStart(2, "0")}`;
}

function fileNameFromPath(p) {
  return p.split(/[\\/]/).pop() || p;
}

function fmtBytes(bytes) {
  if (!bytes) return "0 B";
  const units = ["B", "KB", "MB", "GB"];
  let v = bytes, i = 0;
  while (v >= 1024 && i < units.length - 1) { v /= 1024; i++; }
  return `${v.toFixed(i === 0 ? 0 : 2)} ${units[i]}`;
}

// Splits the segment containing `ms` in two. No-op when `ms` falls in a gap
// or too close (< MIN_SEG_MS) to an existing edge to leave two valid pieces.
function splitSegs(segs, ms) {
  return segs.flatMap((s) =>
    ms > s.startMs + MIN_SEG_MS && ms < s.endMs - MIN_SEG_MS
      ? [{ ...s, id: uid(), endMs: ms }, { ...s, id: uid(), startMs: ms }]
      : [s]
  );
}

// Moves one edge of a segment, clamped by its neighbors (segments never
// overlap; the space freed up becomes a gap).
function resizeSegs(segs, id, edge, ms, durationMs) {
  const i = segs.findIndex((s) => s.id === id);
  if (i < 0) return segs;
  const s = segs[i];
  const prevEnd = i > 0 ? segs[i - 1].endMs : 0;
  const nextStart = i < segs.length - 1 ? segs[i + 1].startMs : durationMs;
  const next = [...segs];
  if (edge === "l") next[i] = { ...s, startMs: clamp(ms, prevEnd, s.endMs - MIN_SEG_MS) };
  else next[i] = { ...s, endMs: clamp(ms, s.startMs + MIN_SEG_MS, nextStart) };
  return next;
}

// The full-source waveform is rendered once per track by ffmpeg and reused
// across mounts (same pattern as the old editor).
const waveformCache = new Map();

function useWaveform(path, trackIndex) {
  const key = `${path}#${trackIndex}`;
  const [img, setImg] = useState(() => waveformCache.get(key) ?? null);
  useEffect(() => {
    if (waveformCache.has(key)) { setImg(waveformCache.get(key)); return; }
    let cancelled = false;
    invoke("render_waveform", { path, audioIndex: trackIndex, width: 1600, height: 56 })
      .then((b64) => {
        const url = `data:image/png;base64,${b64}`;
        waveformCache.set(key, url);
        if (!cancelled) setImg(url);
      })
      .catch(() => {});
    return () => { cancelled = true; };
  }, [key]);
  return img;
}

// One timeline lane: source-time scaled to full width; kept segments are
// bright blocks with drag handles on both edges, gaps show the dark base.
function TrackLane({ kind, durationMs, segs, selectedId, waveform, gapLabel, onSelect, onResize, onSeek }) {
  const laneRef = useRef(null);
  const dragRef = useRef(null);

  const msFromClientX = (clientX) => {
    const rect = laneRef.current.getBoundingClientRect();
    return clamp((clientX - rect.left) / rect.width, 0, 1) * durationMs;
  };

  const onMove = (e) => {
    const d = dragRef.current;
    if (!d) return;
    onResize(d.id, d.edge, msFromClientX(e.clientX));
  };
  const endDrag = () => {
    dragRef.current = null;
    window.removeEventListener("pointermove", onMove);
    window.removeEventListener("pointerup", endDrag);
  };
  const startEdgeDrag = (id, edge) => (e) => {
    e.preventDefault();
    e.stopPropagation();
    onSelect(id);
    dragRef.current = { id, edge };
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", endDrag);
  };
  useEffect(() => () => endDrag(), []);

  const pct = (ms) => `${durationMs > 0 ? (ms / durationMs) * 100 : 0}%`;
  const isVideo = kind === "video";

  return (
    <div
      ref={laneRef}
      className={`relative w-full overflow-hidden rounded-md bg-stone-950 ${isVideo ? "h-12" : "h-9"}`}
      onPointerDown={(e) => { if (e.target === laneRef.current) onSeek(msFromClientX(e.clientX)); }}
    >
      {/* deleted-span caption, behind everything */}
      {gapLabel && (
        <div className="pointer-events-none absolute inset-0 flex items-center justify-center text-[9px] uppercase tracking-widest text-stone-700">
          {gapLabel}
        </div>
      )}
      {waveform && (
        <img src={waveform} alt="" draggable={false}
          className="pointer-events-none absolute inset-0 h-full w-full object-fill opacity-25" />
      )}
      {segs.map((s) => {
        const selected = s.id === selectedId;
        return (
          <div
            key={s.id}
            onPointerDown={(e) => { e.stopPropagation(); onSelect(s.id); }}
            className={`absolute inset-y-0 cursor-pointer overflow-hidden border transition-colors ${
              selected
                ? "border-accent-300 bg-accent-500/40 z-10"
                : isVideo
                  ? "border-accent-500/50 bg-accent-500/20 hover:bg-accent-500/30"
                  : "border-emerald-500/40 bg-emerald-500/15 hover:bg-emerald-500/25"
            } rounded-md`}
            style={{ left: pct(s.startMs), width: pct(s.endMs - s.startMs) }}
          >
            {waveform && (
              <img src={waveform} alt="" draggable={false}
                className="pointer-events-none absolute top-0 h-full max-w-none object-fill opacity-90"
                style={{
                  width: `${durationMs > 0 ? (durationMs / (s.endMs - s.startMs)) * 100 : 100}%`,
                  left: `${s.endMs - s.startMs > 0 ? -(s.startMs / (s.endMs - s.startMs)) * 100 : 0}%`,
                }} />
            )}
            {!isVideo && s.volume !== 1 && (
              <span className="pointer-events-none absolute right-2 top-1/2 -translate-y-1/2 rounded bg-stone-950/80 px-1 text-[9px] font-semibold text-stone-300">
                {s.volume === 0 ? <MdVolumeOff size={9} className="inline" /> : `${Math.round(s.volume * 100)}%`}
              </span>
            )}
            <div onPointerDown={startEdgeDrag(s.id, "l")}
              className={`absolute inset-y-0 left-0 w-1.5 cursor-ew-resize ${selected ? "bg-accent-300" : "bg-white/20 hover:bg-white/50"}`} />
            <div onPointerDown={startEdgeDrag(s.id, "r")}
              className={`absolute inset-y-0 right-0 w-1.5 cursor-ew-resize ${selected ? "bg-accent-300" : "bg-white/20 hover:bg-white/50"}`} />
          </div>
        );
      })}
    </div>
  );
}

// Tick step targeting ~6-12 labels regardless of clip length.
function tickStepMs(durationMs) {
  for (const s of [1, 2, 5, 10, 15, 30, 60, 120, 300, 600]) {
    if (durationMs / (s * 1000) <= 12) return s * 1000;
  }
  return 1200_000;
}

function Ruler({ durationMs, onSeek }) {
  const ref = useRef(null);
  const dragging = useRef(false);

  const seekFromEvent = (e) => {
    const rect = ref.current.getBoundingClientRect();
    onSeek(clamp((e.clientX - rect.left) / rect.width, 0, 1) * durationMs);
  };
  const onMove = (e) => { if (dragging.current) seekFromEvent(e); };
  const endDrag = () => {
    dragging.current = false;
    window.removeEventListener("pointermove", onMove);
    window.removeEventListener("pointerup", endDrag);
  };

  const ticks = useMemo(() => {
    const step = tickStepMs(durationMs);
    const out = [];
    for (let ms = 0; ms <= durationMs; ms += step) out.push(ms);
    return out;
  }, [durationMs]);

  return (
    <div
      ref={ref}
      className="relative h-6 cursor-pointer select-none"
      onPointerDown={(e) => {
        dragging.current = true;
        seekFromEvent(e);
        window.addEventListener("pointermove", onMove);
        window.addEventListener("pointerup", endDrag);
      }}
    >
      {ticks.map((ms) => (
        <div key={ms} className="absolute bottom-0 top-0 flex flex-col justify-end" style={{ left: `${(ms / durationMs) * 100}%` }}>
          <span className="mb-0.5 -translate-x-1/2 text-[9px] text-stone-600">{msToTick(ms)}</span>
          <div className="h-1.5 w-px bg-stone-700" />
        </div>
      ))}
    </div>
  );
}

export default function EditorView({ t, lang, initialPath, onBack }) {
  const [srcPath, setSrcPath] = useState(initialPath ?? null);
  const [clip, setClip] = useState(null); // probe result, null = loading/none
  const [error, setError] = useState("");
  const [videoSegs, setVideoSegs] = useState([]);
  const [audioTracks, setAudioTracks] = useState([]); // [{index,label,muted,segs}]
  // Track 0 is whatever the recorder mapped first (its mix or sole primary
  // source). Collapsed view shows only that; split view shows every other
  // track and drops it, so the two views never overlap the same audio.
  const [splitAudio, setSplitAudio] = useState(false);
  const splitAudioRef = useRef(splitAudio);
  splitAudioRef.current = splitAudio;
  const [selection, setSelection] = useState(null); // {kind:"video"|"audio", track?, id}
  const [currentMs, setCurrentMs] = useState(0);
  const [playing, setPlaying] = useState(false);
  const [exporting, setExporting] = useState(false);
  const [exportProgress, setExportProgress] = useState(null); // {done_ms,total_ms}
  const [lastExportedPath, setLastExportedPath] = useState(null);
  const [showYoutubeModal, setShowYoutubeModal] = useState(false);
  const [driveConnected, setDriveConnected] = useState(false);
  const videoRef = useRef(null);
  const rafRef = useRef(0);
  // Per-track preview <audio> elements (extracted by prepare_edit_audio) —
  // the <video> itself is muted, so what you hear is exactly the edit:
  // deleted audio segments are silent, volumes apply live.
  const [previewAudio, setPreviewAudio] = useState(null); // string[] paths
  const trackElsRef = useRef([]);
  const audioTracksRef = useRef(audioTracks);
  audioTracksRef.current = audioTracks;

  useEffect(() => { setSrcPath(initialPath ?? null); }, [initialPath]);

  // Load + reset the whole edit state whenever the source changes.
  useEffect(() => {
    setClip(null);
    setError("");
    setVideoSegs([]);
    setAudioTracks([]);
    setSplitAudio(false);
    setSelection(null);
    setCurrentMs(0);
    setPlaying(false);
    setLastExportedPath(null);
    if (!srcPath) return;
    let cancelled = false;
    invoke("probe_video", { path: srcPath })
      .then((p) => {
        if (cancelled) return;
        setClip({ path: srcPath, name: fileNameFromPath(srcPath), ...p });
        setVideoSegs([{ id: uid(), startMs: 0, endMs: p.duration_ms }]);
        setAudioTracks((p.audio_tracks ?? []).map((tr) => ({
          index: tr.index,
          label: tr.label,
          muted: false,
          segs: [{ id: uid(), startMs: 0, endMs: p.duration_ms, volume: 1 }],
        })));
      })
      .catch((e) => { if (!cancelled) setError(String(e)); });
    return () => { cancelled = true; };
  }, [srcPath]);

  // Extract preview audio tracks once per source (cached backend-side).
  useEffect(() => {
    setPreviewAudio(null);
    if (!clip) return;
    let cancelled = false;
    invoke("prepare_edit_audio", { path: clip.path })
      .then((files) => { if (!cancelled) setPreviewAudio(files); })
      .catch(() => { if (!cancelled) setPreviewAudio([]); });
    return () => { cancelled = true; };
  }, [clip]);

  // Materialize the hidden audio elements for the extracted tracks.
  useEffect(() => {
    if (!previewAudio) return;
    const els = previewAudio.map((f) => {
      const el = new Audio(convertFileSrc(f));
      el.preload = "auto";
      el.volume = 0;
      return el;
    });
    trackElsRef.current = els;
    return () => {
      els.forEach((el) => { el.pause(); el.removeAttribute("src"); el.load(); });
      trackElsRef.current = [];
    };
  }, [previewAudio]);

  // Applies segment volume (capped at 100%; export still applies full gain) and mutes
  // deleted spans and tracks outside the current export basis (`visibleAudioTracks`).
  const applyTrackVolumes = useCallback((ms) => {
    const tracks = audioTracksRef.current;
    const split = splitAudioRef.current;
    trackElsRef.current.forEach((el, i) => {
      const tr = tracks[i];
      if (!tr) { el.volume = 0; return; }
      const inBasis = tracks.length <= 1 || (split ? i !== 0 : i === 0);
      if (!inBasis || tr.muted) { el.volume = 0; return; }
      const seg = tr.segs.find((s) => ms >= s.startMs && ms < s.endMs);
      el.volume = seg ? Math.min(1, seg.volume) : 0;
    });
  }, []);

  const syncTrackTimes = useCallback((seconds, force = false) => {
    trackElsRef.current.forEach((el) => {
      if (force || Math.abs(el.currentTime - seconds) > 0.12) el.currentTime = seconds;
    });
  }, []);

  const durationMs = clip?.duration_ms ?? 0;
  // Output length = first kept video span → last one (middle gaps count as
  // black; only leading/trailing cuts shorten the output).
  const outputMs = videoSegs.length ? videoSegs[videoSegs.length - 1].endMs - videoSegs[0].startMs : 0;
  // Whether the playhead sits in a removed video span (preview shows black there).
  const inVideoGap = !!clip && !videoSegs.some((s) => currentMs >= s.startMs && currentMs < s.endMs);

  // The tracks currently shown AND exported (see the split/collapsed logic above).
  const visibleAudioTracks = useMemo(() => {
    if (audioTracks.length <= 1) return audioTracks;
    return splitAudio ? audioTracks.slice(1) : audioTracks.slice(0, 1);
  }, [audioTracks, splitAudio]);

  const toggleTrackMute = useCallback((index) => {
    setAudioTracks((ts) => ts.map((tr) => (tr.index === index ? { ...tr, muted: !tr.muted } : tr)));
  }, []);

  const pickFile = async () => {
    const p = await invoke("pick_video_file").catch(() => null);
    if (p) setSrcPath(p);
  };

  // ---- playback -----------------------------------------------------------

  const seekTo = useCallback((ms) => {
    setCurrentMs(ms);
    if (videoRef.current) videoRef.current.currentTime = ms / 1000;
    syncTrackTimes(ms / 1000, true);
    applyTrackVolumes(ms);
  }, [syncTrackTimes, applyTrackVolumes]);

  // rAF-driven playhead: handles skipping deleted video spans during preview
  // (the <video> element itself knows nothing about cuts).
  useEffect(() => {
    if (!playing) return;
    const step = () => {
      const v = videoRef.current;
      if (!v) return;
      const ms = v.currentTime * 1000;
      const last = videoSegs[videoSegs.length - 1];
      if (last && ms >= last.endMs) {
        v.pause();
        trackElsRef.current.forEach((el) => el.pause());
        const first = videoSegs[0];
        if (first) v.currentTime = first.startMs / 1000;
        setCurrentMs(first ? first.startMs : 0);
        setPlaying(false);
        return;
      }
      // Keep the preview tracks glued to the video clock and sounding like
      // the current edit state.
      syncTrackTimes(v.currentTime);
      applyTrackVolumes(ms);
      setCurrentMs(ms);
      rafRef.current = requestAnimationFrame(step);
    };
    rafRef.current = requestAnimationFrame(step);
    return () => cancelAnimationFrame(rafRef.current);
  }, [playing, videoSegs, syncTrackTimes, applyTrackVolumes]);

  const togglePlay = useCallback(() => {
    const v = videoRef.current;
    if (!v || !videoSegs.length) return;
    if (playing) {
      v.pause();
      trackElsRef.current.forEach((el) => el.pause());
      setPlaying(false);
      return;
    }
    const ms = v.currentTime * 1000;
    const first = videoSegs[0];
    const last = videoSegs[videoSegs.length - 1];
    // Outside the output window — restart from the beginning of the edit.
    if (ms < first.startMs || ms >= last.endMs) {
      v.currentTime = first.startMs / 1000;
    }
    syncTrackTimes(v.currentTime, true);
    applyTrackVolumes(v.currentTime * 1000);
    v.play();
    trackElsRef.current.forEach((el) => { el.play().catch(() => {}); });
    setPlaying(true);
  }, [playing, videoSegs, syncTrackTimes, applyTrackVolumes]);

  const stepFrame = (dir) => {
    const v = videoRef.current;
    if (!v || !clip) return;
    v.pause();
    trackElsRef.current.forEach((el) => el.pause());
    setPlaying(false);
    seekTo(clamp(currentMs + dir * (1000 / (clip.fps || 30)), 0, durationMs));
  };

  // ---- edit operations ----------------------------------------------------

  const doSplit = useCallback(() => {
    const ms = currentMs;
    if (selection?.kind === "audio") {
      setAudioTracks((ts) => ts.map((tr) =>
        tr.index === selection.track ? { ...tr, segs: splitSegs(tr.segs, ms) } : tr
      ));
      setSelection(null);
    } else {
      setVideoSegs((segs) => splitSegs(segs, ms));
      setSelection(null);
    }
  }, [currentMs, selection]);

  const doDelete = useCallback(() => {
    if (!selection) return;
    if (selection.kind === "video") {
      setVideoSegs((segs) => (segs.length > 1 ? segs.filter((s) => s.id !== selection.id) : segs));
    } else {
      setAudioTracks((ts) => ts.map((tr) =>
        tr.index === selection.track ? { ...tr, segs: tr.segs.filter((s) => s.id !== selection.id) } : tr
      ));
    }
    setSelection(null);
  }, [selection]);

  const selectedAudioSeg = useMemo(() => {
    if (selection?.kind !== "audio") return null;
    const tr = audioTracks.find((tr) => tr.index === selection.track);
    return tr?.segs.find((s) => s.id === selection.id) ?? null;
  }, [selection, audioTracks]);

  const setSelectedVolume = (volume) => {
    if (selection?.kind !== "audio") return;
    setAudioTracks((ts) => ts.map((tr) =>
      tr.index === selection.track
        ? { ...tr, segs: tr.segs.map((s) => (s.id === selection.id ? { ...s, volume } : s)) }
        : tr
    ));
  };

  // Keyboard: space = play/pause, S = split, Del = delete selection.
  useEffect(() => {
    const onKey = (e) => {
      if (/^(input|textarea|select)$/i.test(e.target?.tagName ?? "")) return;
      if (e.code === "Space") { e.preventDefault(); togglePlay(); }
      else if (e.key === "s" || e.key === "S") doSplit();
      else if (e.key === "Delete" || e.key === "Backspace") doDelete();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [togglePlay, doSplit, doDelete]);

  // ---- export -------------------------------------------------------------

  useEffect(() => {
    if (!exporting) return;
    let unlisten;
    (async () => { unlisten = await listen("editor-export-progress", (e) => setExportProgress(e.payload)); })();
    return () => unlisten?.();
  }, [exporting]);

  // Exports go straight into the library as a "clip" — no save dialog; the
  // backend emits video-saved so the gallery refreshes on its own.
  const doExport = async () => {
    if (!clip || !videoSegs.length) return;
    setError("");
    try {
      setExporting(true);
      setExportProgress(null);
      setLastExportedPath(null);
      const job = {
        source_path: clip.path,
        video_segments: videoSegs.map((s) => ({ start_ms: Math.round(s.startMs), end_ms: Math.round(s.endMs) })),
        // Only the current export basis (see `visibleAudioTracks`), minus muted tracks.
        audio_tracks: visibleAudioTracks.filter((tr) => !tr.muted).map((tr) => ({
          index: tr.index,
          label: tr.label,
          segments: tr.segs.map((s) => ({ start_ms: Math.round(s.startMs), end_ms: Math.round(s.endMs), volume: s.volume })),
        })),
      };
      const savedPath = await invoke("export_edit", { job });
      setLastExportedPath(savedPath);
    } catch (e) {
      setError(String(e));
    } finally {
      setExporting(false);
      setExportProgress(null);
    }
  };

  const openYoutubeUpload = async () => {
    try {
      const status = await invoke("get_drive_status");
      setDriveConnected(!!status?.connected);
    } catch {
      setDriveConnected(false);
    }
    setShowYoutubeModal(true);
  };

  const exportPct = exportProgress?.total_ms
    ? Math.min(100, Math.round((exportProgress.done_ms / exportProgress.total_ms) * 100))
    : null;

  // ---- render -------------------------------------------------------------

  return (
    <div className="flex flex-1 min-h-0 min-w-0 flex-col bg-stone-950">
      <div className="flex shrink-0 items-center gap-3 border-b border-stone-800 px-3 py-2">
        <button onClick={onBack}
          className="flex h-8 w-8 items-center justify-center rounded-lg text-stone-400 transition hover:bg-stone-800 hover:text-stone-200">
          <MdArrowBack size={17} />
        </button>
        <div className="min-w-0 flex-1">
          <div className="truncate text-sm font-semibold text-stone-100">
            {clip ? clip.name : t("videoEditor.title")}
          </div>
          {clip && (
            <div className="flex items-center gap-2 text-[11px] text-stone-500">
              <span>{clip.width}×{clip.height} · {Math.round(clip.fps)} FPS · {fmtBytes(clip.size_bytes)}</span>
              <button onClick={() => invoke("reveal_item", { path: clip.path })}
                className="flex items-center gap-1 text-stone-500 transition hover:text-accent-300" title={t("videoEditor.revealInFolder")}>
                <MdFolderOpen size={12} />
              </button>
            </div>
          )}
        </div>
        {clip && (
          <span className="text-xs text-stone-400">
            {t("videoEditor.totalDuration")}: <span className="font-mono text-stone-200">{msToClock(outputMs)}</span>
          </span>
        )}
        {lastExportedPath && (
          <button onClick={openYoutubeUpload}
            className="flex items-center gap-1.5 rounded-lg bg-stone-800 px-3 py-1.5 text-xs font-medium text-stone-200 transition hover:bg-stone-700">
            <SiYoutube size={13} className="text-red-500" /> {t("videoEditor.uploadToYoutube")}
          </button>
        )}
        <button onClick={doExport} disabled={!clip || exporting}
          className="flex items-center gap-1.5 rounded-lg bg-accent-400 px-3 py-1.5 text-xs font-semibold text-stone-950 transition hover:bg-accent-300 disabled:opacity-50">
          <MdFileDownload size={14} />
          {exporting ? (exportPct != null ? `${exportPct}%` : t("videoEditor.exporting")) : t("videoEditor.export")}
        </button>
      </div>

      {error && <div className="border-b border-red-900/40 bg-red-950/30 px-4 py-1.5 text-xs text-red-400">{error}</div>}
      {exporting && (
        <div className="h-0.5 w-full bg-stone-800">
          <div className="h-full bg-accent-400 transition-all" style={{ width: `${exportPct ?? 0}%` }} />
        </div>
      )}

      {!srcPath || (!clip && !error) ? (
        <div className="flex flex-1 flex-col items-center justify-center gap-3 text-stone-500">
          {!srcPath ? (
            <>
              <MdMovie size={40} className="text-stone-700" />
              <span className="text-sm">{t("videoEditor.emptyHint")}</span>
              <button onClick={pickFile}
                className="rounded-lg bg-stone-800 px-4 py-2 text-xs font-medium text-stone-200 transition hover:bg-stone-700">
                {t("videoEditor.addClip")}
              </button>
            </>
          ) : (
            <span className="text-sm">{t("common.loading")}</span>
          )}
        </div>
      ) : clip && (
        <>
          {/* Preview — the <video> is muted (audio previews through the
              per-track elements) and blacked out over removed spans. */}
          <div className="relative flex flex-1 min-h-0 items-center justify-center bg-black/50 p-3">
            <video
              ref={videoRef}
              src={convertFileSrc(clip.path)}
              muted
              className="max-h-full max-w-full"
              onClick={togglePlay}
              onPause={() => { trackElsRef.current.forEach((el) => el.pause()); setPlaying(false); }}
            />
            {inVideoGap && <div className="absolute inset-0 bg-black" onClick={togglePlay} />}
          </div>

          <div className="flex shrink-0 items-center gap-1 border-t border-stone-800 px-3 py-2">
            <button onClick={() => seekTo(videoSegs[0]?.startMs ?? 0)} title={t("videoEditor.skipStart")}
              className="flex h-7 w-7 items-center justify-center rounded-full text-stone-300 transition hover:bg-stone-800">
              <MdSkipPrevious size={18} />
            </button>
            <button onClick={() => stepFrame(-1)} title={t("videoEditor.prevFrame")}
              className="flex h-7 w-7 rotate-180 items-center justify-center rounded-full text-stone-300 transition hover:bg-stone-800">
              <MdSkipNext size={14} />
            </button>
            <button onClick={togglePlay}
              className="mx-1 flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-accent-500 text-stone-950 transition hover:bg-accent-400">
              {playing ? <MdPause size={18} /> : <MdPlayArrow size={18} />}
            </button>
            <button onClick={() => stepFrame(1)} title={t("videoEditor.nextFrame")}
              className="flex h-7 w-7 items-center justify-center rounded-full text-stone-300 transition hover:bg-stone-800">
              <MdSkipNext size={14} />
            </button>
            <span className="ml-2 font-mono text-xs text-stone-400">
              {msToClock(currentMs)} <span className="text-stone-600">/ {msToClock(durationMs)}</span>
            </span>

            <div className="mx-3 h-5 w-px bg-stone-800" />

            <button onClick={doSplit} title={`${t("videoEditor.split")} (S)`}
              className="flex items-center gap-1.5 rounded-lg bg-stone-800 px-2.5 py-1.5 text-xs font-medium text-stone-300 transition hover:bg-stone-700">
              <MdContentCut size={13} /> {t("videoEditor.split")}
            </button>
            <button onClick={doDelete} title={`${t("videoEditor.deleteSegment")} (Del)`}
              disabled={!selection || (selection.kind === "video" && videoSegs.length <= 1)}
              className="flex items-center gap-1.5 rounded-lg bg-stone-800 px-2.5 py-1.5 text-xs font-medium text-stone-300 transition hover:bg-stone-700 hover:text-red-400 disabled:opacity-40">
              <MdDeleteOutline size={14} /> {t("videoEditor.deleteSegment")}
            </button>

            {selectedAudioSeg && (
              <div className="ml-2 flex items-center gap-2 rounded-lg bg-stone-900 px-2.5 py-1">
                <button onClick={() => setSelectedVolume(selectedAudioSeg.volume === 0 ? 1 : 0)}
                  className="text-stone-400 transition hover:text-stone-200">
                  {selectedAudioSeg.volume === 0 ? <MdVolumeOff size={14} /> : <MdVolumeUp size={14} />}
                </button>
                <input type="range" min={0} max={2} step={0.05} value={selectedAudioSeg.volume}
                  onChange={(e) => setSelectedVolume(Number(e.target.value))}
                  className="h-1 w-28 accent-accent-400" />
                <span className="w-9 text-right font-mono text-[10px] text-stone-400">
                  {Math.round(selectedAudioSeg.volume * 100)}%
                </span>
              </div>
            )}
          </div>

          <div className="relative shrink-0 overflow-y-auto border-t border-stone-800 bg-stone-900/40 px-3 pb-3" style={{ maxHeight: "38vh" }}>
            <div className="flex">
              <div className="w-28 shrink-0" />
              <div className="min-w-0 flex-1"><Ruler durationMs={durationMs} onSeek={seekTo} /></div>
            </div>

            <div className="relative">
              <div className="flex flex-col gap-1.5">
                <div className="flex items-center gap-2">
                  <div className="flex w-28 shrink-0 items-center gap-1.5 text-[11px] font-medium text-stone-400">
                    <MdMovie size={13} className="text-accent-400" />
                    <span className="truncate">{t("videoEditor.videoTrack")}</span>
                  </div>
                  <div className="relative min-w-0 flex-1">
                    <TrackLane
                      kind="video"
                      durationMs={durationMs}
                      segs={videoSegs}
                      selectedId={selection?.kind === "video" ? selection.id : null}
                      gapLabel={t("videoEditor.removed")}
                      onSelect={(id) => setSelection({ kind: "video", id })}
                      onResize={(id, edge, ms) => setVideoSegs((segs) => resizeSegs(segs, id, edge, ms, durationMs))}
                      onSeek={seekTo}
                    />
                  </div>
                </div>

                {/* Audio lanes — collapsed view shows just the recorder's
                    primary/mix track; split view shows every other one
                    instead (see `visibleAudioTracks`). */}
                {visibleAudioTracks.map((tr) => (
                  <AudioLaneRow
                    key={tr.index}
                    t={t}
                    clipPath={clip.path}
                    track={tr}
                    displayLabel={audioTracks.length > 1 && !splitAudio ? t("videoEditor.mixedAudio") : null}
                    durationMs={durationMs}
                    selectedId={selection?.kind === "audio" && selection.track === tr.index ? selection.id : null}
                    onSelect={(id) => setSelection({ kind: "audio", track: tr.index, id })}
                    onResize={(id, edge, ms) =>
                      setAudioTracks((ts) => ts.map((x) =>
                        x.index === tr.index ? { ...x, segs: resizeSegs(x.segs, id, edge, ms, durationMs) } : x
                      ))
                    }
                    onSeek={seekTo}
                    onToggleMute={toggleTrackMute}
                  />
                ))}

                {/* Split-mode toggle — bottom-left, under the track labels.
                    Only meaningful when there's more than one audio track. */}
                {audioTracks.length > 1 && (
                  <div className="flex items-center gap-2">
                    <div className="w-28 shrink-0">
                      <button
                        onClick={() => { setSplitAudio((v) => !v); setSelection(null); }}
                        className={`flex items-center gap-1 rounded-md px-1.5 py-1 text-[10px] font-medium transition ${
                          splitAudio ? "bg-accent-500/20 text-accent-300" : "bg-stone-800 text-stone-400 hover:bg-stone-700"
                        }`}
                      >
                        <MdCallSplit size={12} />
                        {splitAudio ? t("videoEditor.mixedAudio") : t("videoEditor.splitAudio")}
                      </button>
                    </div>
                  </div>
                )}
              </div>

              {/* Playhead spanning all lanes */}
              <div className="pointer-events-none absolute inset-y-0" style={{ left: `calc(7rem + 0.5rem + ${(durationMs > 0 ? currentMs / durationMs : 0)} * (100% - 7rem - 0.5rem))` }}>
                <div className="h-full w-px bg-white shadow-[0_0_4px_rgba(255,255,255,0.7)]" />
              </div>
            </div>
          </div>
        </>
      )}

      {showYoutubeModal && lastExportedPath && (
        <YouTubeUploadModal
          t={t}
          path={lastExportedPath}
          defaultTitle={fileNameFromPath(lastExportedPath).replace(/\.[^.]+$/, "")}
          connected={driveConnected}
          onOpenSettings={() => invoke("open_settings")}
          onClose={() => setShowYoutubeModal(false)}
        />
      )}
    </div>
  );
}

function AudioLaneRow({ t, clipPath, track, displayLabel, durationMs, selectedId, onSelect, onResize, onSeek, onToggleMute }) {
  const waveform = useWaveform(clipPath, track.index);
  const label = displayLabel ?? track.label;
  return (
    <div className="flex items-center gap-2">
      <div className="flex w-28 shrink-0 items-center gap-1.5 text-[11px] font-medium text-stone-400">
        <button
          onClick={() => onToggleMute(track.index)}
          title={track.muted ? t("videoEditor.unmute") : t("videoEditor.mute")}
          className={`shrink-0 transition ${track.muted ? "text-red-400 hover:text-red-300" : "text-emerald-400 hover:text-emerald-300"}`}
        >
          {track.muted ? <MdVolumeOff size={13} /> : <MdVolumeUp size={13} />}
        </button>
        <span className={`truncate ${track.muted ? "text-stone-600 line-through" : ""}`} title={label}>{label}</span>
      </div>
      <div className={`relative min-w-0 flex-1 ${track.muted ? "opacity-40" : ""}`}>
        <TrackLane
          kind="audio"
          durationMs={durationMs}
          segs={track.segs}
          selectedId={selectedId}
          waveform={waveform}
          gapLabel={t("videoEditor.silence")}
          onSelect={onSelect}
          onResize={onResize}
          onSeek={onSeek}
        />
      </div>
    </div>
  );
}
