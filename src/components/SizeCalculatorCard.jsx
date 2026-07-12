import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { MdCalculate, MdClose, MdUnfoldMore } from "react-icons/md";
import {
  FPS_OPTIONS, RESOLUTION_ROWS, RESOLUTION_DIMENSIONS, RESOLUTION_LABELS,
  RATE_CONTROL_OPTIONS, AUDIO_CODECS, AUDIO_CODEC_KBPS,
  ENCODER_GROUPS, ENCODER_LABELS,
  recommendedBitrateKbps, estimateQualityBitrateKbps, estimateLosslessBitrateKbps,
  fmtBufferBytes,
} from "./RecordSettingsCard.jsx";

const DURATION_PRESETS = [1, 5, 15, 30, 60, 120, 240];

function fmtDuration(minutes) {
  if (minutes < 60) return `${minutes}m`;
  const h = Math.floor(minutes / 60);
  const m = minutes % 60;
  return m ? `${h}h ${m}m` : `${h}h`;
}

// Columns for a bitrate-driven rate control (CBR/VBR/VBR+CQ) — multipliers
// of `recommendedBitrateKbps`. For a quality-driven mode these instead become
// fixed CRF/QP values, lower = better.
const TIERS = [
  { id: "low", bitrateMult: 0.5, quality: 32 },
  { id: "recommended", bitrateMult: 1.0, quality: 26 },
  { id: "high", bitrateMult: 1.5, quality: 20 },
  { id: "ultra", bitrateMult: 2.0, quality: 15 },
];

const REFERENCE_ENCODER_FAMILIES = ["h264", "hevc", "av1"];
const REFERENCE_FPS_BUCKETS = [30, 60];

// A pill-tab row — a couple of small option sets share this instead of plain
// `<select>`s, for a look distinct from Settings' form-field style.
export function PillGroup({ options, value, onChange, labelFor }) {
  return (
    <div className="flex items-center gap-0.5 rounded-full bg-white/5 p-0.5">
      {options.map((opt) => (
        <button key={opt} type="button" onClick={() => onChange(opt)}
          className={`rounded-full px-3 py-1.5 text-xs font-medium transition ${
            value === opt ? "bg-accent-500 text-stone-950" : "text-stone-400 hover:text-stone-200"
          }`}>
          {labelFor(opt)}
        </button>
      ))}
    </div>
  );
}

// Native `<select>` popups render with the OS's own (light) combo-box chrome
// in WebView2 — they ignore the page's `color-scheme: dark`, since Windows
// draws that particular popup itself rather than Chromium. A fully custom,
// div-based dropdown sidesteps that entirely and actually matches the modal.
export function GroupedDropdown({ groups, value, labelFor, onChange }) {
  const [open, setOpen] = useState(false);
  const ref = useRef(null);

  useEffect(() => {
    if (!open) return;
    const onClickOutside = (e) => { if (ref.current && !ref.current.contains(e.target)) setOpen(false); };
    document.addEventListener("mousedown", onClickOutside);
    return () => document.removeEventListener("mousedown", onClickOutside);
  }, [open]);

  return (
    <div className="relative" ref={ref}>
      <button type="button" onClick={() => setOpen((o) => !o)}
        className={`${selCls} flex w-40 items-center justify-between gap-2`}>
        <span className="truncate">{labelFor(value)}</span>
        <MdUnfoldMore size={13} className="shrink-0 text-stone-500" />
      </button>
      {open && (
        <div className="absolute left-0 top-[calc(100%+4px)] z-10 max-h-64 w-52 overflow-y-auto rounded-lg border border-white/10 bg-stone-900 p-1 shadow-xl">
          {groups.map((group) => (
            <div key={group.label}>
              <div className="px-2 pb-0.5 pt-1.5 text-[9px] font-semibold uppercase tracking-wider text-stone-600">{group.label}</div>
              {group.options.map((opt) => (
                <button key={opt} type="button" onClick={() => { onChange(opt); setOpen(false); }}
                  className={`block w-full truncate rounded-md px-2 py-1.5 text-left text-xs transition ${
                    value === opt ? "bg-accent-500/20 text-accent-300" : "text-stone-300 hover:bg-white/5"
                  }`}>
                  {labelFor(opt)}
                </button>
              ))}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

export function Field({ label, children }) {
  return (
    <div className="flex flex-col gap-1.5">
      <span className="text-[10px] font-semibold uppercase tracking-wider text-stone-500">{label}</span>
      {children}
    </div>
  );
}

export const selCls = "rounded-lg border border-white/10 bg-white/5 px-2.5 py-1.5 text-xs text-stone-100 outline-none transition focus:border-accent-500 cursor-pointer";

/** Standalone "what-if" tool — own local state, only prefilled from settings once. */
export default function SizeCalculatorModal({ settings, t, onClose }) {
  const video = settings.video ?? {};
  const [fps, setFps] = useState(video.fps ?? 30);
  const [encoder, setEncoder] = useState(video.encoder ?? "auto");
  const [rateControl, setRateControl] = useState(video.rate_control ?? "cbr");
  const [audioCodec, setAudioCodec] = useState(video.audio_codec ?? "aac");
  const [audioTracks, setAudioTracks] = useState(1);
  const [durationMinutes, setDurationMinutes] = useState(30);
  const [entered, setEntered] = useState(false);

  useEffect(() => {
    let raf = requestAnimationFrame(() => { raf = requestAnimationFrame(() => setEntered(true)); });
    return () => cancelAnimationFrame(raf);
  }, []);
  useEffect(() => {
    const onKey = (e) => { if (e.key === "Escape") onClose(); };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const audioKbpsPerTrack = AUDIO_CODEC_KBPS[audioCodec] ?? 192;
  const audioKbps = audioTracks * audioKbpsPerTrack;
  const isQualityDriven = rateControl === "cqp";
  const isLossless = rateControl === "lossless";

  const cellFor = (resolutionKey, tier) => {
    const [width, height] = RESOLUTION_DIMENSIONS[resolutionKey];
    let videoKbps;
    let sublabel;
    if (isLossless) {
      videoKbps = estimateLosslessBitrateKbps({ width, height, fps });
      sublabel = t("settings.sizeCalculator.losslessTag");
    } else if (isQualityDriven) {
      videoKbps = estimateQualityBitrateKbps({ width, height, fps, quality: tier.quality, encoder });
      sublabel = `CRF/QP ${tier.quality}`;
    } else {
      const recommended = recommendedBitrateKbps(resolutionKey, fps, encoder);
      const target = recommended * tier.bitrateMult;
      videoKbps = rateControl === "cbr" ? target : target * 1.5;
      sublabel = `${(target / 1000).toFixed(1)} Mbps`;
    }
    const totalKbps = videoKbps + audioKbps;
    const bytes = (totalKbps * 1000 / 8) * durationMinutes * 60;
    return { bytes, sublabel };
  };

  return createPortal(
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/70 p-4 backdrop-blur-sm md:p-8"
      style={{ opacity: entered ? 1 : 0, transition: "opacity 200ms ease" }}
      onClick={onClose}
    >
      <div
        className="relative flex max-h-[88vh] w-full max-w-5xl flex-col overflow-hidden rounded-3xl border border-white/10 bg-gradient-to-b from-stone-900 to-stone-950 shadow-[0_20px_80px_-20px_rgba(0,0,0,0.7)]"
        style={{
          transform: entered ? "scale(1) translateY(0)" : "scale(0.95) translateY(12px)",
          opacity: entered ? 1 : 0,
          transition: "transform 260ms cubic-bezier(0.16,1,0.3,1), opacity 200ms ease",
        }}
        onClick={(e) => e.stopPropagation()}
      >
        {/* Header */}
        <div className="flex items-center gap-3 border-b border-white/10 bg-gradient-to-r from-accent-500/10 to-transparent px-6 py-4">
          <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-xl bg-accent-500/15 text-accent-400">
            <MdCalculate size={20} />
          </span>
          <div className="min-w-0 flex-1">
            <div className="text-base font-bold text-stone-50">{t("settings.sizeCalculator.title")}</div>
            <div className="truncate text-xs text-stone-500">{t("settings.sizeCalculator.desc")}</div>
          </div>
          <button onClick={onClose} className="rounded-full p-2 text-stone-500 transition hover:bg-white/10 hover:text-stone-200">
            <MdClose size={18} />
          </button>
        </div>

        <div className="overflow-y-auto px-6 py-5">
          {/* Toolbar */}
          <div className="flex flex-wrap items-end gap-4 rounded-2xl border border-white/5 bg-black/20 p-4">
            <Field label={t("settings.video.fps")}>
              <PillGroup options={FPS_OPTIONS} value={fps} onChange={setFps} labelFor={(f) => f} />
            </Field>
            <Field label={t("settings.video.rateControl")}>
              <PillGroup options={RATE_CONTROL_OPTIONS} value={rateControl} onChange={setRateControl}
                labelFor={(rc) => t(`settings.video.rateControlOptions.${rc}`)} />
            </Field>
            <Field label={t("settings.video.encoder")}>
              <GroupedDropdown groups={ENCODER_GROUPS} value={encoder} onChange={setEncoder} labelFor={(v) => ENCODER_LABELS[v]} />
            </Field>
            <Field label={t("settings.video.audioCodec")}>
              <PillGroup options={AUDIO_CODECS.map(([v]) => v)} value={audioCodec} onChange={setAudioCodec}
                labelFor={(v) => AUDIO_CODECS.find(([av]) => av === v)?.[1] ?? v} />
            </Field>
            <Field label={t("settings.sizeCalculator.audioTracks")}>
              <div className="flex items-center gap-2 pt-1.5">
                <input type="range" min="0" max="6" step="1" value={audioTracks}
                  onChange={(e) => setAudioTracks(Number(e.target.value))}
                  className="w-24 accent-accent-500 cursor-pointer" />
                <span className="w-3 text-xs font-semibold text-stone-300">{audioTracks}</span>
              </div>
            </Field>
            <Field label={t("settings.sizeCalculator.duration")}>
              <div className="flex flex-wrap items-center gap-1">
                <div className="flex items-center gap-0.5 rounded-full bg-white/5 p-0.5">
                  {DURATION_PRESETS.map((minutes) => (
                    <button key={minutes} type="button" onClick={() => setDurationMinutes(minutes)}
                      className={`rounded-full px-2.5 py-1.5 text-xs font-medium transition ${
                        durationMinutes === minutes ? "bg-accent-500 text-stone-950" : "text-stone-400 hover:text-stone-200"
                      }`}>
                      {fmtDuration(minutes)}
                    </button>
                  ))}
                </div>
                <input type="number" min="1" value={durationMinutes}
                  onChange={(e) => setDurationMinutes(Math.max(1, Number(e.target.value) || 1))}
                  className={`${selCls} w-16 text-right`} />
              </div>
            </Field>
          </div>

          {/* Table 1: static reference */}
          <div className="mb-1.5 mt-5 text-xs font-semibold uppercase tracking-wide text-stone-400">
            {t("settings.sizeCalculator.referenceTitle")}
          </div>
          <div className="overflow-x-auto rounded-2xl border border-white/10 bg-black/20">
            <table className="w-full border-collapse text-xs">
              <thead>
                <tr>
                  <th rowSpan={2} className="border-b border-white/10 px-3 py-2 text-left text-[9px] font-semibold uppercase tracking-wider text-stone-500 align-bottom">
                    {t("settings.video.resolution")}
                  </th>
                  {REFERENCE_FPS_BUCKETS.map((bucket) => (
                    <th key={bucket} colSpan={REFERENCE_ENCODER_FAMILIES.length}
                      className="border-b border-l border-white/10 px-3 py-1.5 text-center text-[9px] font-semibold uppercase tracking-wider text-stone-500">
                      {bucket} FPS
                    </th>
                  ))}
                </tr>
                <tr>
                  {REFERENCE_FPS_BUCKETS.map((bucket) =>
                    REFERENCE_ENCODER_FAMILIES.map((fam, i) => (
                      <th key={`${bucket}-${fam}`} className={`border-b border-white/10 px-3 py-1.5 text-center text-[9px] font-semibold uppercase tracking-wider text-stone-600 ${i === 0 ? "border-l border-white/10" : ""}`}>
                        {fam === "h264" ? "H.264" : fam.toUpperCase()}
                      </th>
                    ))
                  )}
                </tr>
              </thead>
              <tbody>
                {RESOLUTION_ROWS.map((resKey) => (
                  <tr key={resKey} className="border-t border-white/5 transition hover:bg-white/[0.03]">
                    <td className="px-3 py-2 font-medium text-stone-300">{RESOLUTION_LABELS[resKey]}</td>
                    {REFERENCE_FPS_BUCKETS.map((bucket) =>
                      REFERENCE_ENCODER_FAMILIES.map((fam, i) => {
                        const repEncoder = { h264: "x264_software", hevc: "x265_software", av1: "svt_av1" }[fam];
                        const kbps = recommendedBitrateKbps(resKey, bucket, repEncoder);
                        return (
                          <td key={`${bucket}-${fam}`} className={`px-3 py-2 text-center text-stone-400 ${i === 0 ? "border-l border-white/5" : ""}`}>
                            {(kbps / 1000).toFixed(1)}M
                          </td>
                        );
                      })
                    )}
                  </tr>
                ))}
              </tbody>
            </table>
          </div>

          {/* Table 2: live estimate */}
          <div className="mb-1.5 mt-5 text-xs font-semibold uppercase tracking-wide text-stone-400">
            {t("settings.sizeCalculator.estimateTitle")}
          </div>
          <div className="overflow-x-auto rounded-2xl border border-white/10 bg-black/20">
            <table className="w-full border-collapse text-sm">
              <thead>
                <tr>
                  <th className="border-b border-white/10 px-3 py-2 text-left text-[9px] font-semibold uppercase tracking-wider text-stone-500">
                    {t("settings.video.resolution")}
                  </th>
                  {isLossless
                    ? <th className="border-b border-white/10 px-3 py-2 text-center text-[9px] font-semibold uppercase tracking-wider text-stone-500">
                        {t("settings.sizeCalculator.losslessTag")}
                      </th>
                    : TIERS.map((tier) => (
                        <th key={tier.id} className={`border-b border-white/10 px-3 py-2 text-center text-[9px] font-semibold uppercase tracking-wider ${
                          tier.id === "recommended" ? "text-accent-400" : "text-stone-500"
                        }`}>
                          {t(`settings.sizeCalculator.tiers.${tier.id}`)}
                        </th>
                      ))
                  }
                </tr>
              </thead>
              <tbody>
                {RESOLUTION_ROWS.map((resKey) => (
                  <tr key={resKey} className="border-t border-white/5 transition hover:bg-white/[0.03]">
                    <td className="px-3 py-2.5 font-medium text-stone-300">{RESOLUTION_LABELS[resKey]}</td>
                    {isLossless
                      ? (() => {
                          const cell = cellFor(resKey, TIERS[0]);
                          return <td className="px-3 py-2.5 text-center font-bold text-stone-100">{fmtBufferBytes(cell.bytes)}</td>;
                        })()
                      : TIERS.map((tier) => {
                          const cell = cellFor(resKey, tier);
                          return (
                            <td key={tier.id} className={`px-3 py-2.5 text-center ${tier.id === "recommended" ? "bg-accent-500/10" : ""}`}>
                              <div className="font-bold text-stone-100">{fmtBufferBytes(cell.bytes)}</div>
                              <div className="text-[10px] text-stone-600">{cell.sublabel}</div>
                            </td>
                          );
                        })
                    }
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          <div className="mt-3 text-[11px] leading-relaxed text-stone-600">
            {(isQualityDriven || isLossless) ? t("settings.sizeCalculator.roughNote") : t("settings.sizeCalculator.recommendedNote")}
          </div>
        </div>
      </div>
    </div>,
    document.body
  );
}
