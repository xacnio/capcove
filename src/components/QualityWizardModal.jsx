import { useEffect, useState } from "react";
import { createPortal } from "react-dom";
import { invoke } from "../lib/tauri.js";
import {
  MdAutoAwesome, MdClose, MdStar, MdBalance, MdCompress, MdWorkspacePremium,
  MdGpsFixed, MdGroups, MdExplore, MdApartment, MdChat, MdArrowBack, MdCheck,
  MdAspectRatio, MdSpeed, MdTune, MdGraphicEq, MdMemory, MdVerified,
  MdHighQuality, MdFullscreen, MdVolumeUp,
} from "react-icons/md";
import { ENCODER_LABELS, RESOLUTION_LABELS } from "./RecordSettingsCard.jsx";

// "auto" only ever resolves to H.264 (see `recording::resolve_auto`), so
// Quality/Small File pick explicitly instead, ranked most-efficient first.
const EFFICIENT_ENCODER_RANKING = [
  "nvenc_av1", "amf_av1", "qsv_av1",
  "nvenc_hevc", "amf_hevc", "qsv_hevc",
  "svt_av1", "aom_av1", "x265_software",
  "nvenc_h264", "amf_h264", "qsv_h264", "x264_software",
];

function bestEncoderFor(priority, availability) {
  // Balanced stays on "auto" — the most compatible pick, and it keeps
  // tracking the best available hardware automatically.
  if (priority === "balanced") return "auto";
  return EFFICIENT_ENCODER_RANKING.find((enc) => availability[enc]) ?? "auto";
}

function isEfficientCodec(encoder) {
  return encoder.includes("hevc") || encoder.includes("av1");
}

// Base settings per priority; content-type adjustment and the encoder pick
// layer on top. Resolution/audio codec are separate questions.
const PRIORITY_BASE = {
  quality: { rate_control: "vbr_cq", quality: 18, bitrate_kbps: 20000 },
  balanced: { rate_control: "vbr", bitrate_kbps: 12000 },
  small: { rate_control: "vbr", bitrate_kbps: 6000 },
};

// fps and a bitrate/quality nudge — fast motion needs more bits (or a lower
// CRF/QP) to hold the same visual quality; static content needs less.
const CONTENT_ADJUST = {
  fast: { fps: 60, mult: 1.3, qualityDelta: -2 },
  normal: { fps: 30, mult: 1.0, qualityDelta: 0 },
  general: { fps: 30, mult: 0.8, qualityDelta: 2 },
};

// Several genres can share the same underlying fps/bitrate bucket.
const CONTENT_BUCKET = {
  competitive: "fast",
  moba: "fast",
  openworld: "normal",
  strategy: "general",
  variety: "general",
};

// "native" leaves the resolution field untouched (whatever the user already
// has); the other two are explicit downscale targets.
const RESOLUTION_CHOICES = { native: null, p1080: "p1080", p720: "p720" };
const AUDIO_CHOICES = { best: "flac", standard: "aac" };

// color keys map to `ACCENT_STYLES` below — spelled out as complete class
// names there (not built with template strings) so Tailwind's scanner
// actually picks them up.
const PRIORITY_OPTIONS = [
  { id: "quality", icon: MdStar, color: "amber" },
  { id: "balanced", icon: MdBalance, color: "accent", recommended: true },
  { id: "small", icon: MdCompress, color: "emerald" },
];
const CONTENT_OPTIONS = [
  { id: "competitive", icon: MdGpsFixed, color: "rose" },
  { id: "moba", icon: MdGroups, color: "amber" },
  { id: "openworld", icon: MdExplore, color: "accent", recommended: true },
  { id: "strategy", icon: MdApartment, color: "emerald" },
  { id: "variety", icon: MdChat, color: "stone" },
];
const RESOLUTION_OPTIONS = [
  { id: "native", icon: MdFullscreen, color: "accent", recommended: true },
  { id: "p1080", icon: MdHighQuality, color: "emerald" },
  { id: "p720", icon: MdAspectRatio, color: "stone" },
];
const AUDIO_OPTIONS = [
  { id: "best", icon: MdGraphicEq, color: "violet" },
  { id: "standard", icon: MdVolumeUp, color: "accent", recommended: true },
];

// NVENC/AMF/QSV "lossless" is only their lowest-QP mode, not bit-exact —
// pin software x264, the one encoder here that actually is lossless.
const LOSSLESS_PRESET = { rate_control: "lossless", encoder: "x264_software", container: "mkv", audio_codec: "flac" };

// Presets carry their own resolution/audio pick, skipping straight to review.
const PRESET_PATCHES = {
  quality: { ...PRIORITY_BASE.quality, audio_codec: "flac" },
  balanced: { ...PRIORITY_BASE.balanced, audio_codec: "aac" },
  small: { ...PRIORITY_BASE.small, audio_codec: "opus", resolution: "p1080" },
};

function computeResult(priority, content, resolution, audio, availability) {
  const base = PRIORITY_BASE[priority];
  const adjust = CONTENT_ADJUST[CONTENT_BUCKET[content]];
  const patch = { ...base, fps: adjust.fps, encoder: bestEncoderFor(priority, availability), audio_codec: AUDIO_CHOICES[audio] };
  const resValue = RESOLUTION_CHOICES[resolution];
  if (resValue) patch.resolution = resValue;
  if (patch.bitrate_kbps != null) {
    patch.bitrate_kbps = Math.max(1000, Math.round((patch.bitrate_kbps * adjust.mult) / 100) * 100);
  }
  if (patch.quality != null) {
    patch.quality = Math.min(45, Math.max(10, patch.quality + adjust.qualityDelta));
  }
  return patch;
}

function computePresetResult(id, availability) {
  return { ...PRESET_PATCHES[id], fps: CONTENT_ADJUST.normal.fps, encoder: bestEncoderFor(id, availability) };
}

const ACCENT_STYLES = {
  amber: { text: "text-amber-400", border: "border-amber-500/50", bg: "bg-amber-500/10", ring: "ring-amber-500/30" },
  accent: { text: "text-accent-400", border: "border-accent-500/50", bg: "bg-accent-500/10", ring: "ring-accent-500/30" },
  emerald: { text: "text-emerald-400", border: "border-emerald-500/50", bg: "bg-emerald-500/10", ring: "ring-emerald-500/30" },
  violet: { text: "text-violet-400", border: "border-violet-500/50", bg: "bg-violet-500/10", ring: "ring-violet-500/30" },
  rose: { text: "text-rose-400", border: "border-rose-500/50", bg: "bg-rose-500/10", ring: "ring-rose-500/30" },
  stone: { text: "text-stone-400", border: "border-stone-500/50", bg: "bg-stone-500/10", ring: "ring-stone-500/30" },
};

function OptionCard({ icon: Icn, label, desc, color = "accent", badge, selected, onClick }) {
  const c = ACCENT_STYLES[color];
  return (
    <button type="button" onClick={onClick}
      className={`group relative flex flex-1 flex-col items-center gap-2 rounded-2xl border px-4 py-5 text-center transition-all duration-150 hover:-translate-y-0.5 active:translate-y-0 active:scale-[0.98] ${
        selected ? `${c.border} ${c.bg} ring-1 ${c.ring}` : "border-white/10 bg-white/[0.03] hover:border-white/20 hover:bg-white/[0.06]"
      }`}>
      {badge && (
        <span className={`absolute -top-2.5 left-1/2 -translate-x-1/2 whitespace-nowrap rounded-full px-2 py-0.5 text-[9px] font-bold uppercase tracking-wider ${c.bg} ${c.text} ring-1 ${c.ring}`}>
          {badge}
        </span>
      )}
      <Icn size={24} className={selected ? c.text : "text-stone-500 transition group-hover:text-stone-300"} />
      <span className={`text-sm font-semibold ${selected ? "text-stone-100" : "text-stone-300"}`}>{label}</span>
      {desc && <span className="text-[11px] leading-snug text-stone-500">{desc}</span>}
    </button>
  );
}

function PresetCard({ icon: Icn, label, desc, color, badge, onClick }) {
  const c = ACCENT_STYLES[color];
  return (
    <button type="button" onClick={onClick}
      className="group relative flex flex-col gap-2 rounded-2xl border border-white/10 bg-white/[0.03] p-4 text-left transition-all duration-150 hover:-translate-y-0.5 hover:border-white/20 hover:bg-white/[0.06] active:translate-y-0 active:scale-[0.98]">
      {badge && (
        <span className={`absolute right-3 top-3 rounded-full px-2 py-0.5 text-[9px] font-bold uppercase tracking-wider ${c.bg} ${c.text}`}>
          {badge}
        </span>
      )}
      <span className={`flex h-9 w-9 items-center justify-center rounded-xl ${c.bg} ${c.text}`}>
        <Icn size={18} />
      </span>
      <span className="text-sm font-bold text-stone-100">{label}</span>
      <span className="text-[11px] leading-snug text-stone-500">{desc}</span>
    </button>
  );
}

function StatChip({ icon: Icn, label, value }) {
  return (
    <div className="flex items-center gap-2.5 rounded-xl border border-white/5 bg-white/[0.02] px-3 py-2.5">
      <Icn size={15} className="shrink-0 text-stone-500" />
      <div className="min-w-0">
        <div className="text-[9px] font-semibold uppercase tracking-wider text-stone-600">{label}</div>
        <div className="truncate text-xs font-semibold text-stone-200">{value}</div>
      </div>
    </div>
  );
}

/** Guided alternative to the Quality tab's raw encoder/bitrate/CRF controls. */
export default function QualityWizardModal({ settings, t, apply, onClose }) {
  const video = settings.video ?? {};
  const [step, setStep] = useState("priority"); // priority | content | resolution | audio | presets | review
  const [priority, setPriority] = useState(null);
  const [content, setContent] = useState(null);
  const [resolution, setResolution] = useState(null);
  const [audio, setAudio] = useState(null);
  const [fromPresets, setFromPresets] = useState(false);
  const [pendingPatch, setPendingPatch] = useState(null);
  const [entered, setEntered] = useState(false);
  const [encoders, setEncoders] = useState(null);

  useEffect(() => {
    invoke("list_available_encoders").then(setEncoders).catch(() => setEncoders([]));
  }, []);
  useEffect(() => {
    let raf = requestAnimationFrame(() => { raf = requestAnimationFrame(() => setEntered(true)); });
    return () => cancelAnimationFrame(raf);
  }, []);
  useEffect(() => {
    const onKey = (e) => { if (e.key === "Escape") onClose(); };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const availability = Object.fromEntries((encoders ?? []).map((e) => [e.kind, e.available]));

  const goToReview = (patch) => { setPendingPatch(patch); setStep("review"); };
  const pickPreset = (id) => {
    setFromPresets(true);
    if (id === "lossless") { goToReview(LOSSLESS_PRESET); return; }
    goToReview(computePresetResult(id, availability));
  };
  const pickPriority = (id) => { setPriority(id); setFromPresets(false); setStep("content"); };
  const pickContent = (id) => { setContent(id); setStep("resolution"); };
  const pickResolution = (id) => { setResolution(id); setStep("audio"); };
  const pickAudio = (id) => { setAudio(id); goToReview(computeResult(priority, content, resolution, id, availability)); };
  const goBack = () => {
    if (step === "review") setStep(fromPresets ? "presets" : "audio");
    else if (step === "audio") setStep("resolution");
    else if (step === "resolution") setStep("content");
    else if (step === "content") setStep("priority");
    else if (step === "presets") setStep("priority");
  };

  const applyAndClose = () => {
    apply({ video: { ...video, ...pendingPatch } });
    onClose();
  };

  const summary = pendingPatch && (() => {
    const rc = pendingPatch.rate_control;
    const isLossless = rc === "lossless";
    const rcLabel = t(`settings.video.rateControlOptions.${rc}`);
    const bitrateLabel = pendingPatch.bitrate_kbps != null ? `~${(pendingPatch.bitrate_kbps / 1000).toFixed(0)} Mbps` : null;
    const qualityLabel = pendingPatch.quality != null ? `CRF/QP ${pendingPatch.quality}` : null;
    const encoderLabel = ENCODER_LABELS[pendingPatch.encoder] ?? pendingPatch.encoder;
    const resolutionLabel = RESOLUTION_LABELS[pendingPatch.resolution ?? video.resolution ?? "native"];
    const encoderNote = pendingPatch.encoder === "x264_software"
      ? t("settings.qualityWizard.encoderNoteLossless")
      : isEfficientCodec(pendingPatch.encoder)
      ? t("settings.qualityWizard.encoderNoteEfficient")
      : null;
    return { isLossless, rcLabel, bitrateLabel, qualityLabel, encoderLabel, resolutionLabel, encoderNote };
  })();

  const progress = { priority: 20, content: 40, resolution: 60, audio: 80, review: 100 }[step] ?? 0;

  return createPortal(
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/70 p-4 backdrop-blur-sm md:p-8"
      style={{ opacity: entered ? 1 : 0, transition: "opacity 200ms ease" }}
      onClick={onClose}
    >
      <div
        className="relative flex max-h-[85vh] w-full max-w-2xl flex-col overflow-hidden rounded-3xl border border-white/10 bg-stone-950 shadow-[0_20px_80px_-20px_rgba(0,0,0,0.75)]"
        style={{
          transform: entered ? "scale(1) translateY(0)" : "scale(0.95) translateY(12px)",
          opacity: entered ? 1 : 0,
          transition: "transform 260ms cubic-bezier(0.16,1,0.3,1), opacity 200ms ease",
        }}
        onClick={(e) => e.stopPropagation()}
      >
        {/* Progress rail — hidden on the presets shortcut, which isn't part of the 4-question count. */}
        <div className="h-[3px] w-full bg-white/5">
          {step !== "presets" && (
            <div className="h-full bg-accent-500 transition-[width] duration-300 ease-out" style={{ width: `${progress}%` }} />
          )}
        </div>

        <div className="flex items-center gap-3 px-6 pb-4 pt-5">
          {step !== "priority" && (
            <button onClick={goBack} className="rounded-full p-1.5 text-stone-500 transition hover:bg-white/10 hover:text-stone-200">
              <MdArrowBack size={16} />
            </button>
          )}
          <div className="min-w-0 flex-1">
            <div className="flex items-center gap-1.5 text-[10px] font-bold uppercase tracking-wider text-accent-500">
              <MdAutoAwesome size={12} />
              {t("settings.qualityWizard.kicker")}
            </div>
            <div className="mt-0.5 text-lg font-bold text-stone-50">{t(`settings.qualityWizard.steps.${step}`)}</div>
          </div>
          <button onClick={onClose} className="rounded-full p-2 text-stone-500 transition hover:bg-white/10 hover:text-stone-200">
            <MdClose size={18} />
          </button>
        </div>

        <div className="overflow-y-auto px-6 pb-6">
          {step === "priority" && (
            <>
              <div className="mb-4 text-[15px] font-medium text-stone-200">{t("settings.qualityWizard.priorityQuestion")}</div>
              <div className="flex gap-3">
                {PRIORITY_OPTIONS.map((opt) => (
                  <OptionCard key={opt.id} icon={opt.icon} color={opt.color}
                    badge={opt.recommended ? t("settings.qualityWizard.popular") : null}
                    label={t(`settings.qualityWizard.priorityOptions.${opt.id}`)}
                    desc={t(`settings.qualityWizard.priorityOptions.${opt.id}Desc`)}
                    selected={priority === opt.id} onClick={() => pickPriority(opt.id)} />
                ))}
              </div>
              <button type="button" onClick={() => { setFromPresets(true); setStep("presets"); }}
                className="mt-3 flex w-full items-center justify-center gap-2 rounded-xl border border-dashed border-white/15 py-3 text-xs font-semibold text-stone-400 transition hover:border-accent-500/40 hover:text-accent-300">
                <MdWorkspacePremium size={14} />
                {t("settings.qualityWizard.askInstead")}
              </button>
            </>
          )}

          {step === "presets" && (
            <div className="grid grid-cols-2 gap-3">
              <PresetCard icon={MdStar} color="amber" label={t("settings.qualityWizard.presets.quality")}
                desc={t("settings.qualityWizard.presets.qualityDesc")} onClick={() => pickPreset("quality")} />
              <PresetCard icon={MdBalance} color="accent" badge={t("settings.qualityWizard.popular")}
                label={t("settings.qualityWizard.presets.balanced")}
                desc={t("settings.qualityWizard.presets.balancedDesc")} onClick={() => pickPreset("balanced")} />
              <PresetCard icon={MdCompress} color="emerald" label={t("settings.qualityWizard.presets.small")}
                desc={t("settings.qualityWizard.presets.smallDesc")} onClick={() => pickPreset("small")} />
              <PresetCard icon={MdWorkspacePremium} color="violet" label={t("settings.qualityWizard.presets.lossless")}
                desc={t("settings.qualityWizard.presets.losslessDesc")} onClick={() => pickPreset("lossless")} />
            </div>
          )}

          {step === "content" && (
            <>
              <div className="mb-4 text-[15px] font-medium text-stone-200">{t("settings.qualityWizard.contentQuestion")}</div>
              <div className="grid grid-cols-3 gap-3">
                {CONTENT_OPTIONS.map((opt) => (
                  <OptionCard key={opt.id} icon={opt.icon} color={opt.color}
                    badge={opt.recommended ? t("settings.qualityWizard.popular") : null}
                    label={t(`settings.qualityWizard.contentOptions.${opt.id}`)}
                    desc={t(`settings.qualityWizard.contentOptions.${opt.id}Desc`)}
                    selected={content === opt.id} onClick={() => pickContent(opt.id)} />
                ))}
              </div>
            </>
          )}

          {step === "resolution" && (
            <>
              <div className="mb-4 text-[15px] font-medium text-stone-200">{t("settings.qualityWizard.resolutionQuestion")}</div>
              <div className="flex gap-3">
                {RESOLUTION_OPTIONS.map((opt) => (
                  <OptionCard key={opt.id} icon={opt.icon} color={opt.color}
                    badge={opt.recommended ? t("settings.qualityWizard.popular") : null}
                    label={t(`settings.qualityWizard.resolutionOptions.${opt.id}`)}
                    desc={t(`settings.qualityWizard.resolutionOptions.${opt.id}Desc`)}
                    selected={resolution === opt.id} onClick={() => pickResolution(opt.id)} />
                ))}
              </div>
            </>
          )}

          {step === "audio" && (
            <>
              <div className="mb-4 text-[15px] font-medium text-stone-200">{t("settings.qualityWizard.audioQuestion")}</div>
              <div className="flex gap-3">
                {AUDIO_OPTIONS.map((opt) => (
                  <OptionCard key={opt.id} icon={opt.icon} color={opt.color}
                    badge={opt.recommended ? t("settings.qualityWizard.popular") : null}
                    label={t(`settings.qualityWizard.audioOptions.${opt.id}`)}
                    desc={t(`settings.qualityWizard.audioOptions.${opt.id}Desc`)}
                    selected={audio === opt.id} onClick={() => pickAudio(opt.id)} />
                ))}
              </div>
            </>
          )}

          {step === "review" && summary && (
            <>
              <div className="grid grid-cols-2 gap-2">
                <StatChip icon={MdAspectRatio} label={t("settings.video.resolution")} value={summary.resolutionLabel} />
                <StatChip icon={MdSpeed} label={t("settings.video.fps")} value={`${pendingPatch.fps ?? video.fps ?? 30} FPS`} />
                <StatChip icon={MdGraphicEq} label={t("settings.video.rateControl")}
                  value={summary.qualityLabel ? `${summary.rcLabel} · ${summary.qualityLabel}` : summary.rcLabel} />
                <StatChip icon={MdTune} label={t("settings.video.bitrate")} value={summary.bitrateLabel ?? "—"} />
                <StatChip icon={MdMemory} label={t("settings.video.encoder")} value={summary.encoderLabel} />
                <StatChip icon={MdVerified} label={t("settings.video.audioCodec")} value={pendingPatch.audio_codec.toUpperCase()} />
              </div>
              {summary.encoderNote && (
                <div className="mt-3 flex items-start gap-2 rounded-xl bg-white/[0.03] px-3 py-2.5 text-[11px] leading-relaxed text-stone-500">
                  <MdAutoAwesome size={13} className="mt-0.5 shrink-0 text-accent-500" />
                  {summary.encoderNote}
                </div>
              )}
              {summary.isLossless && (
                <div className="mt-3 text-[11px] leading-relaxed text-stone-600">{t("settings.qualityWizard.losslessWarning")}</div>
              )}
              <div className="mt-4 flex gap-2">
                <button type="button" onClick={onClose}
                  className="rounded-xl px-4 py-3 text-xs font-semibold text-stone-500 transition hover:text-stone-300">
                  {t("settings.qualityWizard.cancel")}
                </button>
                <button type="button" onClick={applyAndClose}
                  className="flex flex-1 items-center justify-center gap-2 rounded-xl bg-accent-500 py-3 text-sm font-bold text-stone-950 transition hover:bg-accent-400 active:scale-[0.98]">
                  <MdCheck size={18} />
                  {t("settings.qualityWizard.apply")}
                </button>
              </div>
            </>
          )}
        </div>
      </div>
    </div>,
    document.body
  );
}
