import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { MdClose, MdTune } from "react-icons/md";
import { invoke, listen } from "../lib/tauri.js";
import {
  RESOLUTION_OPTIONS, RESOLUTION_LABELS, RESOLUTION_DIMENSIONS,
  ENCODER_GROUPS, ENCODER_LABELS,
  CONTAINER_OPTIONS, CONTAINER_LABELS,
  AUDIO_CODECS, AUDIO_CODEC_KBPS,
  RATE_CONTROL_OPTIONS, rateControlUsesBitrate, rateControlUsesQuality,
  bitrateFieldLabel,
  estimateQualityBitrateKbps, estimateLosslessBitrateKbps,
  fmtBufferBytes,
} from "./RecordSettingsCard.jsx";
import { PillGroup, GroupedDropdown, Field } from "./SizeCalculatorCard.jsx";

// Fragmented containers only matter for a long-running *live* recording
// (crash mid-write still leaves it playable) — meaningless for a one-shot
// export, so they're left out here even though the live settings have them.
const EXPORT_CONTAINER_OPTIONS = CONTAINER_OPTIONS.filter((v) => !v.endsWith("_fragmented"));

function fmtSecs(s) {
  const total = Math.max(0, Math.round(s));
  const m = Math.floor(total / 60);
  const ss = String(total % 60).padStart(2, "0");
  return `${m}:${ss}`;
}

// Custom pointer-driven slider — a plain `<input type="range">` renders with
// each platform/webview's own inconsistent chrome; this matches the rest of
// the player's own trim/volume bars instead.
function Slider({ value, min, max, step = 1, onChange, formatValue }) {
  const barRef = useRef(null);
  const [dragging, setDragging] = useState(false);

  const setFromClientX = (clientX) => {
    const bar = barRef.current;
    if (!bar) return;
    const rect = bar.getBoundingClientRect();
    const frac = Math.min(1, Math.max(0, (clientX - rect.left) / rect.width));
    const stepped = Math.round((min + frac * (max - min)) / step) * step;
    onChange(Math.min(max, Math.max(min, stepped)));
  };

  useEffect(() => {
    if (!dragging) return undefined;
    const onMove = (e) => setFromClientX(e.clientX);
    const onUp = () => setDragging(false);
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
    return () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [dragging]);

  const pct = ((value - min) / (max - min)) * 100;

  return (
    <div className="flex items-center gap-3">
      <div
        ref={barRef}
        onPointerDown={(e) => { setDragging(true); setFromClientX(e.clientX); }}
        className="group/slider relative flex h-4 flex-1 cursor-pointer items-center"
      >
        <div className="h-1.5 w-full overflow-hidden rounded-full bg-white/10">
          <div className="h-full rounded-full bg-accent-500" style={{ width: `${pct}%` }} />
        </div>
        <div
          className={`absolute top-1/2 h-3 w-3 -translate-x-1/2 -translate-y-1/2 rounded-full bg-white shadow transition-opacity ${
            dragging ? "opacity-100" : "opacity-0 group-hover/slider:opacity-100"
          }`}
          style={{ left: `${pct}%` }}
        />
      </div>
      <span className="w-16 shrink-0 text-right text-xs font-semibold tabular-nums text-stone-200">{formatValue(value)}</span>
    </div>
  );
}

/** Trim tool's advanced export — resolution/encoder/format/rate-control for
 * this one clip, with a live estimated size, instead of just the app's
 * current recording quality settings (see the plain "Save Clip" button). */
export default function AdvancedExportModal({ path, startMs, endMs, t, onClose, onExported }) {
  const [entered, setEntered] = useState(false);
  const [resolution, setResolution] = useState("native");
  const [encoder, setEncoder] = useState("auto");
  const [container, setContainer] = useState("mp4");
  const [audioCodec, setAudioCodec] = useState("aac");
  const [rateControl, setRateControl] = useState("cbr");
  const [bitrateKbps, setBitrateKbps] = useState(8000);
  const [quality, setQuality] = useState(23);
  const [source, setSource] = useState(null); // {width, height, fps}
  const [exporting, setExporting] = useState(false);
  const [exportProgress, setExportProgress] = useState(0);
  const [error, setError] = useState("");

  useEffect(() => {
    let raf = requestAnimationFrame(() => { raf = requestAnimationFrame(() => setEntered(true)); });
    return () => cancelAnimationFrame(raf);
  }, []);
  useEffect(() => {
    const onKey = (e) => { if (e.key === "Escape" && !exporting) onClose(); };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose, exporting]);
  useEffect(() => {
    if (!exporting) return undefined;
    setExportProgress(0);
    let unlisten;
    (async () => {
      unlisten = await listen("trim-export-progress", (e) => {
        const { done_ms, total_ms } = e.payload;
        setExportProgress(total_ms > 0 ? Math.min(100, Math.round((done_ms / total_ms) * 100)) : 0);
      });
    })();
    return () => unlisten?.();
  }, [exporting]);

  // Prefilled from the app's current recording quality settings once, then
  // freely editable — same "own local state" approach as the size calculator.
  useEffect(() => {
    invoke("get_settings").then((settings) => {
      const v = settings.video ?? {};
      if (v.encoder) setEncoder(v.encoder);
      if (v.container) setContainer(v.container.replace("_fragmented", ""));
      if (v.audio_codec) setAudioCodec(v.audio_codec);
      if (v.rate_control) setRateControl(v.rate_control);
      if (v.bitrate_kbps) setBitrateKbps(v.bitrate_kbps);
      if (v.quality) setQuality(v.quality);
    }).catch(() => {});
  }, []);

  useEffect(() => {
    let cancelled = false;
    invoke("probe_video", { path })
      .then((info) => { if (!cancelled) setSource({ width: info.width, height: info.height, fps: info.fps || 30 }); })
      .catch(() => {});
    return () => { cancelled = true; };
  }, [path]);

  const durationSecs = Math.max(0, (endMs - startMs) / 1000);
  const targetDims = (() => {
    if (resolution === "native") return source ?? { width: 1920, height: 1080 };
    const [w, h] = RESOLUTION_DIMENSIONS[resolution];
    if (!source || h <= source.height) return { width: w, height: h };
    // Backend never upscales past the source either (`scale=-2:min(H,ih)`).
    return source;
  })();
  const fps = source?.fps ?? 30;
  const audioKbps = AUDIO_CODEC_KBPS[audioCodec] ?? 192;
  const usesBitrate = rateControlUsesBitrate(rateControl);
  const usesQuality = rateControlUsesQuality(rateControl);
  const isLossless = rateControl === "lossless";

  const videoKbps = (() => {
    if (isLossless) return estimateLosslessBitrateKbps({ width: targetDims.width, height: targetDims.height, fps });
    const qualityEstimate = usesQuality
      ? estimateQualityBitrateKbps({ width: targetDims.width, height: targetDims.height, fps, quality, encoder })
      : null;
    // VBR+CQ: quality drives it, bitrate only caps — same as the real encode.
    if (usesQuality && usesBitrate) return Math.min(bitrateKbps, qualityEstimate);
    if (usesQuality) return qualityEstimate;
    return bitrateKbps;
  })();
  const estimatedBytes = ((videoKbps + audioKbps) * 1000 / 8) * durationSecs;

  const doExport = async () => {
    setExporting(true);
    setError("");
    try {
      const clip = await invoke("export_trim_clip", {
        path,
        startMs: Math.round(startMs),
        endMs: Math.round(endMs),
        // `advanced`'s own fields are a nested struct, not top-level command
        // params — Tauri only camelCase-converts the latter, so these must
        // match `AdvancedTrimOptions`'s Rust field names exactly.
        advanced: {
          resolution, encoder, container,
          audio_codec: audioCodec,
          rate_control: rateControl,
          bitrate_kbps: bitrateKbps,
          quality,
        },
      });
      onExported(clip);
      onClose();
    } catch (e) {
      setError(String(e));
    } finally {
      setExporting(false);
    }
  };

  return createPortal(
    <div
      className="fixed inset-0 z-[80] flex items-center justify-center bg-black/70 p-4 backdrop-blur-sm"
      style={{ opacity: entered ? 1 : 0, transition: "opacity 200ms ease" }}
      onClick={exporting ? undefined : onClose}
    >
      <div
        className="relative flex max-h-[85vh] w-full max-w-md flex-col overflow-hidden rounded-3xl border border-white/10 bg-gradient-to-b from-stone-900 to-stone-950 shadow-[0_20px_80px_-20px_rgba(0,0,0,0.7)]"
        style={{
          transform: entered ? "scale(1) translateY(0)" : "scale(0.95) translateY(12px)",
          opacity: entered ? 1 : 0,
          transition: "transform 260ms cubic-bezier(0.16,1,0.3,1), opacity 200ms ease",
        }}
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center gap-3 border-b border-white/10 bg-gradient-to-r from-accent-500/10 to-transparent px-5 py-4">
          <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-xl bg-accent-500/15 text-accent-400">
            <MdTune size={20} />
          </span>
          <div className="min-w-0 flex-1">
            <div className="text-base font-bold text-stone-50">{t("videoEditor.trim.advancedExport.title")}</div>
            <div className="truncate text-xs text-stone-500">{t("videoEditor.trim.advancedExport.desc")}</div>
          </div>
          <button onClick={onClose} disabled={exporting} className="rounded-full p-2 text-stone-500 transition hover:bg-white/10 hover:text-stone-200 disabled:opacity-40">
            <MdClose size={18} />
          </button>
        </div>

        <div className="flex flex-col gap-4 overflow-y-auto px-5 py-5">
          <Field label={t("settings.video.resolution")}>
            <PillGroup options={RESOLUTION_OPTIONS.map(([v]) => v)} value={resolution} onChange={setResolution} labelFor={(v) => RESOLUTION_LABELS[v]} />
          </Field>
          <div className="flex flex-wrap gap-x-4 gap-y-3">
            <Field label={t("settings.video.encoder")}>
              <GroupedDropdown groups={ENCODER_GROUPS} value={encoder} onChange={setEncoder} labelFor={(v) => ENCODER_LABELS[v]} />
            </Field>
            <Field label={t("settings.video.container")}>
              <PillGroup options={EXPORT_CONTAINER_OPTIONS} value={container} onChange={setContainer} labelFor={(v) => CONTAINER_LABELS[v]} />
            </Field>
            <Field label={t("settings.video.audioCodec")}>
              <PillGroup options={AUDIO_CODECS.map(([v]) => v)} value={audioCodec} onChange={setAudioCodec}
                labelFor={(v) => AUDIO_CODECS.find(([av]) => av === v)?.[1] ?? v} />
            </Field>
          </div>
          <Field label={t("settings.video.rateControl")}>
            <PillGroup options={RATE_CONTROL_OPTIONS} value={rateControl} onChange={setRateControl}
              labelFor={(rc) => t(`settings.video.rateControlOptions.${rc}`)} />
          </Field>
          {usesBitrate && (
            <Field label={bitrateFieldLabel(t, rateControl)}>
              <Slider value={bitrateKbps} min={500} max={60000} step={250} onChange={setBitrateKbps}
                formatValue={(v) => `${(v / 1000).toFixed(1)} Mbps`} />
            </Field>
          )}
          {usesQuality && (
            <Field label={t("settings.video.quality")}>
              <Slider value={quality} min={15} max={51} step={1} onChange={setQuality} formatValue={(v) => v} />
            </Field>
          )}

          <div className="rounded-2xl border border-white/10 bg-black/20 p-4">
            <div className="text-[10px] font-semibold uppercase tracking-wider text-stone-500">{t("videoEditor.trim.advancedExport.estimatedSize")}</div>
            <div className="mt-1 text-2xl font-bold text-stone-50">{fmtBufferBytes(estimatedBytes)}</div>
            <div className="mt-0.5 text-xs text-stone-500">{targetDims.width}×{targetDims.height} · {fmtSecs(durationSecs)}</div>
          </div>

          {error && <div className="text-xs text-red-400">{error}</div>}

          {exporting && (
            <div className="flex items-center gap-2">
              <div className="h-1.5 flex-1 overflow-hidden rounded-full bg-white/10">
                <div className="h-full rounded-full bg-accent-500 transition-[width]" style={{ width: `${exportProgress}%` }} />
              </div>
              <span className="w-9 shrink-0 text-right text-xs font-semibold tabular-nums text-stone-300">{exportProgress}%</span>
            </div>
          )}
        </div>

        <div className="flex items-center justify-end gap-2 border-t border-white/10 px-5 py-4">
          <button onClick={onClose} disabled={exporting} className="rounded-full px-4 py-2 text-xs font-medium text-stone-400 transition hover:text-stone-200 disabled:opacity-40">
            {t("videoEditor.youtubeModal.cancel")}
          </button>
          <button onClick={doExport} disabled={exporting} className="flex items-center gap-2 rounded-full bg-accent-500 px-4 py-2 text-xs font-semibold text-stone-950 transition hover:bg-accent-400 disabled:opacity-60">
            {exporting && <span className="h-3.5 w-3.5 animate-spin rounded-full border-2 border-stone-900/40 border-t-stone-950" />}
            {t(exporting ? "videoEditor.trim.advancedExport.exporting" : "videoEditor.trim.advancedExport.export")}
          </button>
        </div>
      </div>
    </div>,
    document.body
  );
}
