import { useEffect, useRef, useState } from "react";
import { invoke } from "../lib/tauri.js";
import {
  MdDesktopWindows, MdMic, MdApps, MdClose, MdSportsEsports,
  MdBolt, MdFiberManualRecord, MdBlock, MdUnfoldMore, MdInfoOutline, MdTune,
  MdExpandMore, MdExpandLess, MdPlayArrow,
} from "react-icons/md";
import { SiYoutube } from "react-icons/si";
import { Toggle, Row, Card, Button, inputCls, OverrideTile } from "./settingsUI.jsx";
import { useAppIcon } from "../gallery/appIcons.js";
import { HUD_ICONS, HUD_ICON_CHOICES } from "../lib/hudIcons.js";

// What happens automatically when a fullscreen game is detected — an icon
// tab strip with the selected mode explained underneath.
const GAME_DETECT_MODES = [
  { id: "clips", icon: MdBolt },
  { id: "full_session", icon: MdFiberManualRecord },
  { id: "off", icon: MdBlock },
];

export function RecordingModeCard({ settings, apply, t }) {
  const video = settings.video ?? {};
  const rb = video.replay_buffer ?? {};
  const mode = rb.game_detect_mode ?? "off";

  const setMode = (m) => apply({ video: { ...video, replay_buffer: { ...rb, game_detect_mode: m } } });

  return (
    <Card title={t("settings.recordingMode.title")}>
      <div className="py-3">
        <div className="grid grid-cols-3 gap-1 rounded-xl bg-stone-950 p-1">
          {GAME_DETECT_MODES.map(({ id, icon: Icn }) => {
            const sel = mode === id;
            return (
              <button key={id} type="button" onClick={() => setMode(id)}
                className={`flex flex-col items-center gap-1.5 rounded-lg px-2 py-3 transition ${
                  sel ? "bg-stone-800 ring-1 ring-stone-700" : "hover:bg-stone-900"
                }`}>
                <Icn size={18} className={sel ? "text-accent-400" : "text-stone-600"} />
                <span className={`text-xs font-semibold ${sel ? "text-stone-100" : "text-stone-500"}`}>
                  {t(`settings.recordingMode.${id}.label`)}
                </span>
              </button>
            );
          })}
        </div>
        <div className="mt-3 flex items-start gap-2 rounded-lg bg-stone-900/60 px-3 py-2.5">
          <MdInfoOutline size={14} className="mt-0.5 shrink-0 text-stone-600" />
          <p className="text-xs leading-relaxed text-stone-400">{t(`settings.recordingMode.${mode}.desc`)}</p>
        </div>

        {mode === "full_session" && (
          <div className="mt-2 flex items-center justify-between gap-4 rounded-lg border border-stone-800 bg-stone-950/60 px-3 py-2.5">
            <div className="min-w-0">
              <div className="flex items-center gap-1.5 text-sm text-stone-200">
                <SiYoutube size={14} className="shrink-0 text-red-500" />
                {t("settings.recordingMode.youtubeLive")}
              </div>
              <div className="mt-0.5 text-xs text-stone-500">{t("settings.recordingMode.youtubeLiveHint")}</div>
            </div>
            <Toggle labeled checked={rb.full_session_youtube_live ?? false}
              onChange={(v) => apply({ video: { ...video, replay_buffer: { ...rb, full_session_youtube_live: v } } })} />
          </div>
        )}
      </div>
    </Card>
  );
}

const YOUTUBE_PRIVACY_OPTIONS = ["private", "unlisted", "public"];
const DEFAULT_TITLE_TEMPLATE = "{game} — {date} {time}";
// YouTube's ingest enforces a hard per-resolution/codec bitrate ceiling, so
// this list is deliberately narrower than the local-recording options.
const YOUTUBE_BITRATE_OPTIONS = [3000, 4000, 5000, 6000, 8000, 10000, 12000];
const YOUTUBE_FPS_OPTIONS = [30, 60];
const YOUTUBE_KEYFRAME_OPTIONS = [1, 2, 3, 4];
const YOUTUBE_BUFFER_OPTIONS = [0.5, 1, 1.5, 2];
const YOUTUBE_AUDIO_CODEC_OPTIONS = ["aac", "mp3"];
const YOUTUBE_SAMPLE_RATE_OPTIONS = [44100, 48000];
// YouTube's published encoder-settings reference (support.google.com/youtube/answer/2853702).
const YOUTUBE_BITRATE_TABLE = [
  { res: "4K / 2160p @60fps", min: 10, max: 40, h264: 35 },
  { res: "4K / 2160p @30fps", min: 8, max: 35, h264: 30 },
  { res: "1440p @60fps", min: 6, max: 30, h264: 24 },
  { res: "1440p @30fps", min: 5, max: 25, h264: 15 },
  { res: "1080p @60fps", min: 4, max: 10, h264: 12 },
  { res: "1080p @30fps", min: 3, max: 8, h264: 10 },
  { res: "720p @60fps", min: 3, max: 8, h264: 6 },
  { res: "240p-720p @30fps", min: 3, max: 8, h264: 4 },
];

function YoutubeBitrateTable({ t }) {
  return (
    <div className="overflow-x-auto rounded-lg border border-stone-800">
      <table className="w-full whitespace-nowrap text-[11px]">
        <thead>
          <tr className="border-b border-stone-800 bg-stone-950/60 text-left text-stone-500">
            <th className="px-2.5 py-1.5 font-medium">{t("settings.youtubeLive.table.resolution")}</th>
            <th className="px-2.5 py-1.5 text-center font-medium">{t("settings.youtubeLive.table.av1hevcMin")}</th>
            <th className="px-2.5 py-1.5 text-center font-medium">{t("settings.youtubeLive.table.av1hevcMax")}</th>
            <th className="px-2.5 py-1.5 text-center font-medium">{t("settings.youtubeLive.table.h264")}</th>
          </tr>
        </thead>
        <tbody>
          {YOUTUBE_BITRATE_TABLE.map((r) => (
            <tr key={r.res} className="border-b border-stone-800/60 text-stone-300 last:border-0">
              <td className="px-2.5 py-1.5">{r.res}</td>
              <td className="px-2.5 py-1.5 text-center text-stone-500">{r.min} Mbps</td>
              <td className="px-2.5 py-1.5 text-center text-stone-500">{r.max} Mbps</td>
              <td className="px-2.5 py-1.5 text-center text-stone-500">{r.h264} Mbps</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function renderTitleTemplatePreview(template) {
  const now = new Date();
  const pad = (n) => String(n).padStart(2, "0");
  const date = `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())}`;
  const time = `${pad(now.getHours())}:${pad(now.getMinutes())}`;
  return (template || DEFAULT_TITLE_TEMPLATE)
    .replaceAll("{game}", "Valorant")
    .replaceAll("{datetime}", `${date} ${time}`)
    .replaceAll("{date}", date)
    .replaceAll("{time}", time);
}

// Shared by every YouTube live stream this app can start — account-wide
// preferences, not something worth exposing per-stream.
export function YoutubeLiveSettingsCard({ settings, apply, t }) {
  const video = settings.video ?? {};
  const yt = video.youtube_live ?? {};
  const setField = (patch) => apply({ video: { ...video, youtube_live: { ...yt, ...patch } } });
  const [advancedOpen, setAdvancedOpen] = useState(false);

  return (
    <Card title={t("settings.youtubeLive.title")}>
      <div className="mt-3 flex items-start gap-2 rounded-lg border border-sky-500/25 bg-sky-500/5 px-3 py-2.5 text-xs leading-relaxed text-sky-200/90">
        <MdInfoOutline size={15} className="mt-0.5 shrink-0 text-sky-400" />
        <span>{t("settings.youtubeLive.noLocalRecording")}</span>
      </div>
      <div className="py-3">
        <div className="mb-1 text-sm text-stone-200">{t("settings.youtubeLive.titleTemplateLabel")}</div>
        <input value={yt.title_template ?? ""} onChange={(e) => setField({ title_template: e.target.value })}
          placeholder={DEFAULT_TITLE_TEMPLATE} className={`${inputCls} w-full`} />
        <div className="mt-1.5 text-xs text-stone-500">{t("settings.youtubeLive.titleTemplateHint")}</div>
        <div className="mt-1 text-[11px] text-stone-600">
          {t("settings.youtubeLive.preview")}: {renderTitleTemplatePreview(yt.title_template)}
        </div>
      </div>
      <Row label={t("settings.youtubeLive.privacyLabel")} hint={t("settings.youtubeLive.privacyHint")}>
        <select value={yt.privacy ?? "private"} onChange={(e) => setField({ privacy: e.target.value })}
          className={`${inputCls} cursor-pointer py-1 text-xs`}>
          {YOUTUBE_PRIVACY_OPTIONS.map((p) => (
            <option key={p} value={p}>{t(`settings.youtubeLive.${p}`)}</option>
          ))}
        </select>
      </Row>
      <Row label={t("settings.youtubeLive.maxResolutionLabel")} hint={t("settings.youtubeLive.maxResolutionHint")}>
        <select value={yt.max_resolution ?? "p1080"} onChange={(e) => setField({ max_resolution: e.target.value })}
          className={`${inputCls} cursor-pointer py-1 text-xs`}>
          {RESOLUTION_OPTIONS.map(([v, label]) => <option key={v} value={v}>{label}</option>)}
        </select>
      </Row>
      <Row label={t("settings.youtubeLive.maxBitrateLabel")} hint={t("settings.youtubeLive.maxBitrateHint")}>
        <select value={yt.max_bitrate_kbps ?? 8000} onChange={(e) => setField({ max_bitrate_kbps: Number(e.target.value) })}
          className={`${inputCls} cursor-pointer py-1 text-xs`}>
          {YOUTUBE_BITRATE_OPTIONS.map((b) => <option key={b} value={b}>{(b / 1000).toFixed(0)} Mbps</option>)}
        </select>
      </Row>
      <Row label={t("settings.youtubeLive.maxFpsLabel")} hint={t("settings.youtubeLive.maxFpsHint")}>
        <select value={yt.max_fps ?? 60} onChange={(e) => setField({ max_fps: Number(e.target.value) })}
          className={`${inputCls} cursor-pointer py-1 text-xs`}>
          {YOUTUBE_FPS_OPTIONS.map((f) => <option key={f} value={f}>{f} FPS</option>)}
        </select>
      </Row>

      <div className="py-3">
        <button type="button" onClick={() => setAdvancedOpen((v) => !v)}
          className="flex w-full items-center justify-between text-sm text-stone-200">
          <span className="flex items-center gap-1.5">
            <MdTune size={14} className="text-stone-500" />
            {t("settings.youtubeLive.advancedTitle")}
          </span>
          {advancedOpen ? <MdExpandLess size={18} className="text-stone-500" /> : <MdExpandMore size={18} className="text-stone-500" />}
        </button>

        {advancedOpen && (
          <div className="mt-2 flex flex-col divide-y divide-stone-800/70">
            <Row label={t("settings.youtubeLive.keyframeLabel")} hint={t("settings.youtubeLive.keyframeHint")}>
              <select value={yt.keyframe_interval_secs ?? 2} onChange={(e) => setField({ keyframe_interval_secs: Number(e.target.value) })}
                className={`${inputCls} cursor-pointer py-1 text-xs`}>
                {YOUTUBE_KEYFRAME_OPTIONS.map((s) => <option key={s} value={s}>{s}s</option>)}
              </select>
            </Row>
            <Row label={t("settings.youtubeLive.bufferLabel")} hint={t("settings.youtubeLive.bufferHint")}>
              <select value={yt.cbr_buffer_secs ?? 1} onChange={(e) => setField({ cbr_buffer_secs: Number(e.target.value) })}
                className={`${inputCls} cursor-pointer py-1 text-xs`}>
                {YOUTUBE_BUFFER_OPTIONS.map((s) => <option key={s} value={s}>{s}s</option>)}
              </select>
            </Row>
            <Row label={t("settings.youtubeLive.audioCodecLabel")} hint={t("settings.youtubeLive.audioCodecHint")}>
              <select value={yt.audio_codec ?? "aac"} onChange={(e) => setField({ audio_codec: e.target.value })}
                className={`${inputCls} cursor-pointer py-1 text-xs`}>
                {YOUTUBE_AUDIO_CODEC_OPTIONS.map((c) => <option key={c} value={c}>{c.toUpperCase()}</option>)}
              </select>
            </Row>
            <Row label={t("settings.youtubeLive.audioSampleRateLabel")} hint={t("settings.youtubeLive.audioSampleRateHint")}>
              <select value={yt.audio_sample_rate ?? 48000} onChange={(e) => setField({ audio_sample_rate: Number(e.target.value) })}
                className={`${inputCls} cursor-pointer py-1 text-xs`}>
                {YOUTUBE_SAMPLE_RATE_OPTIONS.map((r) => <option key={r} value={r}>{(r / 1000).toFixed(1)} kHz</option>)}
              </select>
            </Row>
            <div className="py-3">
              <div className="mb-1.5 flex items-center gap-1.5 text-xs font-semibold uppercase tracking-wider text-stone-500">
                <SiYoutube size={12} className="text-red-500" />
                {t("settings.youtubeLive.table.title")}
              </div>
              <YoutubeBitrateTable t={t} />
              <div className="mt-1.5 text-[11px] text-stone-600">{t("settings.youtubeLive.table.cbrNote")}</div>
            </div>
          </div>
        )}
      </div>
    </Card>
  );
}

export const FPS_OPTIONS = [24, 30, 60, 120, 144];
export const RESOLUTION_OPTIONS = [
  ["native", "Native"],
  ["p2160", "4K (2160p)"],
  ["p1440", "1440p"],
  ["p1080", "1080p"],
  ["p720", "720p"],
  ["p480", "480p"],
];
export const RESOLUTION_LABELS = Object.fromEntries(RESOLUTION_OPTIONS);
export const BITRATE_OPTIONS = [4000, 6000, 8000, 12000, 15000, 20000, 30000, 50000];
// QP/CRF scale: lower is better/larger, 0 is near-lossless, 51 is the floor.
export const QUALITY_OPTIONS = [15, 18, 20, 23, 26, 30, 35, 40, 45, 51];
export const RATE_CONTROL_OPTIONS = ["cbr", "vbr", "cqp", "vbr_cq", "lossless"];
// VBR+CQ uses both: bitrate as a cap, quality as the actual driver.
export function rateControlUsesBitrate(rc) { return rc === "cbr" || rc === "vbr" || rc === "vbr_cq"; }
export function rateControlUsesQuality(rc) { return rc === "cqp" || rc === "vbr_cq"; }
// CBR's bitrate is an exact target; VBR's is only the average (~1.5x
// ceiling); VBR+CQ's is purely a safety ceiling, not an average.
export function bitrateFieldLabel(t, rateControl) {
  const base = t("settings.video.bitrate");
  if (rateControl === "vbr") return `${base} (${t("settings.video.bitrateAvgSuffix")})`;
  if (rateControl === "vbr_cq") return `${base} (${t("settings.video.bitrateCapSuffix")})`;
  return base;
}

// "Native" has no fixed size — capture resolution, unknowable here — so it's
// approximated as 1080p, the most common actual capture size.
export const RESOLUTION_DIMENSIONS = {
  native: [1920, 1080],
  p2160: [3840, 2160],
  p1440: [2560, 1440],
  p1080: [1920, 1080],
  p720: [1280, 720],
  p480: [854, 480],
};

export function encoderFamily(encoder) {
  if (!encoder || encoder === "auto") return "h264"; // resolve_auto only ever picks an H.264 candidate
  if (encoder.includes("av1")) return "av1";
  if (encoder.includes("hevc")) return "hevc";
  return "h264";
}

// Empirical, tuned for typical game-recording content (moderate-to-high motion).
const QUALITY_BITRATE_REFERENCE = { width: 1920, height: 1080, fps: 30, quality: 23, kbps: 8000 };
// HEVC/AV1 need meaningfully fewer bits than H.264 for the same visual
// quality at the same CRF/QP-equivalent setting.
const ENCODER_EFFICIENCY = { h264: 1.0, hevc: 0.62, av1: 0.5 };

/** Rough average bitrate for a CQP/VBR+CQ encode at the given settings. */
export function estimateQualityBitrateKbps({ width, height, fps, quality, encoder }) {
  const pixelRatio = (width * height) / (QUALITY_BITRATE_REFERENCE.width * QUALITY_BITRATE_REFERENCE.height);
  // Dampened, not linear — doubling fps doesn't double the bits actually
  // needed, since consecutive frames stay highly similar either way.
  const fpsRatio = Math.pow(fps / QUALITY_BITRATE_REFERENCE.fps, 0.7);
  // x264's well-known rule of thumb: every ±6 CRF/QP roughly halves/doubles
  // the bitrate needed for equivalent quality.
  const qualityMultiplier = Math.pow(2, (QUALITY_BITRATE_REFERENCE.quality - quality) / 6);
  const efficiency = ENCODER_EFFICIENCY[encoderFamily(encoder)] ?? 1.0;
  return QUALITY_BITRATE_REFERENCE.kbps * pixelRatio * fpsRatio * qualityMultiplier * efficiency;
}

/** Rough average bitrate for a true-lossless encode — extremely content-dependent. */
export function estimateLosslessBitrateKbps({ width, height, fps }) {
  // ~0.5 bits/pixel/frame is a rough middle-of-the-road figure for lossless
  // game-recording content; real footage can land well above or below this.
  return (width * height * fps * 0.5) / 1000;
}

// Concrete resolutions only, smallest to largest — "native" has no fixed
// size, so it doesn't belong as a matrix row.
export const RESOLUTION_ROWS = ["p480", "p720", "p1080", "p1440", "p2160"];

// H.264 baseline at 30/60fps; HEVC/AV1 scale down via `ENCODER_EFFICIENCY`.
const RECOMMENDED_BITRATE_KBPS = {
  p480: { 30: 2500, 60: 4000 },
  p720: { 30: 5000, 60: 7500 },
  p1080: { 30: 8000, 60: 12000 },
  p1440: { 30: 16000, 60: 24000 },
  p2160: { 30: 40000, 60: 60000 },
};

/** Recommended average bitrate (kbps) for a resolution/fps/encoder combo. */
export function recommendedBitrateKbps(resolution, fps, encoder) {
  const table = RECOMMENDED_BITRATE_KBPS[resolution] ?? RECOMMENDED_BITRATE_KBPS.p1080;
  const bucket = fps > 30 ? 60 : 30;
  // Dampened extra scaling for fps values that aren't exactly the bucket's
  // (24 vs. 30, or 120/144 vs. 60) — same reasoning as `estimateQualityBitrateKbps`.
  const fpsRatio = Math.pow(fps / bucket, 0.7);
  const efficiency = ENCODER_EFFICIENCY[encoderFamily(encoder)] ?? 1.0;
  return table[bucket] * fpsRatio * efficiency;
}

export const CONTAINER_OPTIONS = ["mp4", "mkv", "mov", "mp4_fragmented", "mov_fragmented"];
// "Fragmented" only changes how ffmpeg muxes the file (chunked instead of
// one index at the end), so a crash mid-recording still leaves it playable.
// `row.v.toUpperCase()` would read "MP4_FRAGMENTED", hence explicit labels.
export const CONTAINER_LABELS = {
  mp4: "MP4",
  mkv: "MKV",
  mov: "MOV",
  mp4_fragmented: "MP4 (Fragmented)",
  mov_fragmented: "MOV (Fragmented)",
};
const HUD_CORNERS = ["top_left", "top_right", "bottom_left", "bottom_right"];

// Quality presets: picking one writes concrete fps/bitrate values; hand-
// editing any control makes the selection read as "custom" again (detection
// is purely by value match, nothing is stored).
const QUALITY_PRESETS = [
  { id: "low",      fps: 24, bitrate_kbps: 6000 },
  { id: "standard", fps: 30, bitrate_kbps: 12000 },
  { id: "high",     fps: 60, bitrate_kbps: 20000 },
];

// 2x2 grid of corner buttons around a small rectangle outline — click the
// corner to anchor the on-screen recording indicator there.
function HudCornerPicker({ value, onChange }) {
  return (
    <div className="relative h-14 w-20 rounded-md border border-stone-700 bg-stone-800/40">
      {HUD_CORNERS.map((corner) => {
        const [v, h] = corner.split("_");
        return (
          <button
            key={corner}
            type="button"
            onClick={() => onChange(corner)}
            className={`absolute h-2.5 w-2.5 rounded-full transition ${
              value === corner ? "bg-accent-400 ring-2 ring-accent-400/40" : "bg-stone-600 hover:bg-stone-400"
            }`}
            style={{
              [v === "top" ? "top" : "bottom"]: 4,
              [h === "left" ? "left" : "right"]: 4,
            }}
          />
        );
      })}
    </div>
  );
}

// Grouped by vendor instead of a flat alphabetical list.
export const ENCODER_GROUPS = [
  { label: "Auto", options: ["auto"] },
  { label: "NVIDIA NVENC", options: ["nvenc_h264", "nvenc_hevc", "nvenc_av1"] },
  { label: "AMD AMF", options: ["amf_h264", "amf_hevc", "amf_av1"] },
  { label: "Intel QSV", options: ["qsv_h264", "qsv_hevc", "qsv_av1"] },
  { label: "Software", options: ["x264_software", "x265_software", "svt_av1", "aom_av1"] },
];

export const ENCODER_LABELS = {
  auto: "Auto",
  nvenc_h264: "NVENC H.264", nvenc_hevc: "NVENC HEVC", nvenc_av1: "NVENC AV1",
  amf_h264: "AMF H.264", amf_hevc: "AMF HEVC", amf_av1: "AMF AV1",
  qsv_h264: "QSV H.264", qsv_hevc: "QSV HEVC", qsv_av1: "QSV AV1",
  x264_software: "Software x264", x265_software: "Software x265",
  svt_av1: "SVT-AV1", aom_av1: "AOM AV1",
};

export const AUDIO_CODECS = [
  ["aac", "AAC"],
  ["opus", "Opus"],
  ["mp3", "MP3"],
  ["flac", "FLAC"],
];
export const AUDIO_CODEC_LABELS = Object.fromEntries(AUDIO_CODECS);

// "Which should I pick?" — a comparison matrix, not prose. Each option is a
// row of level bars per metric (more = better); rows apply the selection on
// click, the full explanation lives in the row tooltip.

// Encoder metrics, scored 1-10. Hardware encoders tie on `perf` (fixed-function block);
// software encoders share the CPU with the game, so slower codecs score lower.
const ENCODER_GUIDE = [
  { v: "nvenc_h264", tag: "gpu", vendor: "nvenc", codec: "h264", m: { perf: 10, size: 4, compat: 10 } },
  { v: "nvenc_hevc", tag: "gpu", vendor: "nvenc", codec: "hevc", m: { perf: 10, size: 7, compat: 6 } },
  { v: "nvenc_av1", tag: "gpu", vendor: "nvenc", codec: "av1", m: { perf: 10, size: 9, compat: 5 } },
  { v: "amf_h264", tag: "gpu", vendor: "amf", codec: "h264", m: { perf: 10, size: 4, compat: 10 } },
  { v: "amf_hevc", tag: "gpu", vendor: "amf", codec: "hevc", m: { perf: 10, size: 7, compat: 6 } },
  { v: "amf_av1", tag: "gpu", vendor: "amf", codec: "av1", m: { perf: 10, size: 9, compat: 5 } },
  { v: "qsv_h264", tag: "gpu", vendor: "qsv", codec: "h264", m: { perf: 10, size: 4, compat: 10 } },
  { v: "qsv_hevc", tag: "gpu", vendor: "qsv", codec: "hevc", m: { perf: 10, size: 7, compat: 6 } },
  { v: "qsv_av1", tag: "gpu", vendor: "qsv", codec: "av1", m: { perf: 10, size: 9, compat: 5 } },
  { v: "x264_software", tag: "cpu", vendor: "x264", codec: "h264", m: { perf: 6, size: 4, compat: 10 } },
  { v: "x265_software", tag: "cpu", vendor: "x265", codec: "hevc", m: { perf: 3, size: 7, compat: 6 } },
  { v: "svt_av1", tag: "cpu", vendor: "svt", codec: "av1", m: { perf: 3, size: 9, compat: 5 } },
  { v: "aom_av1", tag: "cpu", vendor: "aom", codec: "av1", m: { perf: 1, size: 9, compat: 5 } },
];

// mp4's low durability score matches this app's actual ffmpeg args
// (`encoder.rs` muxes with plain `-movflags +faststart`, not fragmented) —
// an interrupted recording loses its index and becomes unplayable, unlike MKV.
const CONTAINER_GUIDE = [
  { v: "mp4", m: { compat: 10, durability: 3, editing: 9 } },
  { v: "mkv", m: { compat: 6, durability: 9, editing: 6 } },
  { v: "mov", m: { compat: 6, durability: 3, editing: 9 } },
  { v: "mp4_fragmented", recommended: true, m: { compat: 8, durability: 9, editing: 7 } },
  { v: "mov_fragmented", m: { compat: 7, durability: 9, editing: 7 } },
];

const AUDIO_GUIDE = [
  { v: "aac", recommended: true, m: { quality: 7, size: 8, compat: 10 } },
  { v: "opus", m: { quality: 9, size: 8, compat: 6 } },
  { v: "mp3", m: { quality: 4, size: 8, compat: 10 } },
  { v: "flac", m: { quality: 10, size: 2, compat: 6 } },
];

const GUIDE_COLUMNS = {
  encoder: ["perf", "size", "compat"],
  container: ["compat", "durability", "editing"],
  audio: ["quality", "size", "compat"],
};

// Shows the actual score rather than a 3-way tier, so close options don't
// collide into the same label; color still gives a quick good/mid/poor read.
function Meter({ score, t }) {
  const style = score >= 8 ? "bg-emerald-500/15 text-emerald-300" : score >= 5 ? "bg-amber-500/15 text-amber-300" : "bg-red-500/15 text-red-300";
  return (
    <span className={`rounded px-1.5 py-0.5 text-[10px] font-semibold tabular-nums ${style}`}>
      {t("settings.video.codecGuide.score")(score)}
    </span>
  );
}

// Name column needs extra width for entries like "Auto → NVENC H.264" plus
// the "Recommended" badge; score columns stay short ("10/10") either way.
const GUIDE_GRID = { display: "grid", gridTemplateColumns: "minmax(150px,2.2fr) minmax(44px,1fr) minmax(44px,1fr) minmax(44px,1fr)", alignItems: "center", gap: "0.5rem" };

function GuideRow({ name, tag, tip, metrics, cols, recommended, selected, unavailable, onSelect, t }) {
  return (
    <button type="button" onClick={onSelect} disabled={unavailable} title={tip} style={GUIDE_GRID}
      className={`w-full rounded-lg px-3 py-1.5 text-left transition ${
        selected ? "bg-accent-500/10 ring-1 ring-accent-500/40" : unavailable ? "opacity-35" : "hover:bg-stone-900"
      }`}>
      <span className="flex min-w-0 items-center gap-1.5">
        <span className={`truncate text-xs font-semibold ${selected ? "text-accent-300" : "text-stone-200"}`}>{name}</span>
        {tag && (
          <span className={`shrink-0 rounded px-1 py-px text-[8px] font-bold uppercase ${
            tag === "gpu" ? "bg-emerald-500/10 text-emerald-300" : "bg-sky-500/10 text-sky-300"
          }`}>
            {tag}
          </span>
        )}
        {recommended && (
          <span className="shrink-0 rounded bg-accent-500/15 px-1 py-px text-[8px] font-bold uppercase text-accent-300">
            {t("settings.video.codecGuide.recommended")}
          </span>
        )}
      </span>
      {unavailable ? (
        <span className="text-[10px] text-stone-600" style={{ gridColumn: `span ${cols.length}` }}>
          {t("settings.video.unavailable")}
        </span>
      ) : (
        cols.map((c) => (
          <span key={c} className="flex justify-center"><Meter score={metrics[c]} t={t} /></span>
        ))
      )}
    </button>
  );
}

function CodecGuide({ video, applyVideo, availability, encodersLoaded, t }) {
  const [tab, setTab] = useState("encoder");
  // What Auto concretely resolves to on this machine, shown inline so
  // "Auto" isn't a black box.
  const [autoResolved, setAutoResolved] = useState(null);
  useEffect(() => { invoke("resolve_auto_encoder").then(setAutoResolved).catch(() => {}); }, []);
  // Collapsed to only the encoders that work on this machine by default;
  // the full matrix is one click away.
  const [showAllEncoders, setShowAllEncoders] = useState(false);

  const g = (k) => t(`settings.video.codecGuide.${k}`);
  const cols = GUIDE_COLUMNS[tab];

  const current = tab === "encoder" ? (video.encoder ?? "auto") : tab === "container" ? (video.container ?? "mp4") : (video.audio_codec ?? "aac");
  const encoderRows = ENCODER_GUIDE.filter((r) =>
    showAllEncoders || !encodersLoaded || availability[r.v] !== false || r.v === current
  );
  const hiddenEncoderCount = ENCODER_GUIDE.length - encoderRows.length;
  const rows = tab === "encoder" ? encoderRows : tab === "container" ? CONTAINER_GUIDE : AUDIO_GUIDE;
  const select = (v) =>
    applyVideo(tab === "encoder" ? { encoder: v } : tab === "container" ? { container: v } : { audio_codec: v });

  const selectedDesc = (() => {
    if (tab === "encoder") {
      if (current === "auto") {
        const resolved = autoResolved && ENCODER_LABELS[autoResolved];
        return `${g("encoders.auto")}${resolved ? ` ${t("settings.video.codecGuide.autoNow")(resolved)}` : ""}`;
      }
      const row = ENCODER_GUIDE.find((r) => r.v === current);
      return row ? `${g(`encoders.${row.vendor}`)} ${g(`codecs.${row.codec}`)}` : "";
    }
    return g(`${tab === "container" ? "containers" : "audio"}.${current}`);
  })();

  return (
    <div className="mt-4 overflow-hidden rounded-xl border border-stone-800 bg-stone-950/60">
      <div className="flex flex-wrap items-center justify-between gap-2 border-b border-stone-800/60 px-3 py-2">
        <span className="text-xs font-semibold text-stone-300">{g("title")}</span>
        <div className="flex items-center rounded-lg bg-stone-900 p-0.5">
          {["encoder", "container", "audio"].map((id) => (
            <button key={id} type="button" onClick={() => setTab(id)}
              className={`rounded-md px-2.5 py-1 text-[11px] font-semibold transition ${
                tab === id ? "bg-stone-700 text-stone-100" : "text-stone-500 hover:text-stone-300"
              }`}>
              {g(`tabs.${id}`)}
            </button>
          ))}
        </div>
      </div>

      <div className="p-1.5">
        <div style={GUIDE_GRID} className="px-3 pb-1 pt-0.5">
          <span />
          {cols.map((c) => (
            <span key={c} className="text-center text-[9px] font-semibold uppercase tracking-wider text-stone-500">
              {g(`cols.${c}`)}
            </span>
          ))}
        </div>

        {/* The selected row's explanation expands right underneath it. */}
        {(() => {
          const selectedNote = (
            <div className="mb-1 ml-3 mr-1 mt-1.5 rounded-r-md border-l-2 border-accent-500/60 bg-stone-900/60 px-2.5 py-1.5 text-[11px] leading-relaxed text-stone-400">
              {selectedDesc}
            </div>
          );
          return (
            <div className="flex flex-col gap-0.5">
              {tab === "encoder" && (
                <>
                  <GuideRow cols={cols} metrics={{ perf: 10, size: 4, compat: 10 }} recommended
                    name={autoResolved ? `Auto → ${ENCODER_LABELS[autoResolved]}` : "Auto"}
                    tip={g("encoders.auto")} selected={current === "auto"} onSelect={() => select("auto")} t={t} />
                  {current === "auto" && selectedNote}
                </>
              )}
              {tab === "encoder" && !encodersLoaded ? (
                // Availability isn't known yet — show placeholders instead
                // of an unfiltered list that would flash then shrink.
                <div className="flex flex-col gap-0.5 px-3 py-1">
                  {Array.from({ length: 4 }).map((_, i) => (
                    <div key={i} className="h-7 animate-pulse rounded-lg bg-stone-900" />
                  ))}
                </div>
              ) : (
                rows.map((row) => (
                  <div key={row.v} className="flex flex-col gap-0.5">
                    <GuideRow t={t} cols={cols}
                      name={tab === "encoder" ? ENCODER_LABELS[row.v] : tab === "container" ? CONTAINER_LABELS[row.v] : AUDIO_CODEC_LABELS[row.v]}
                      tag={row.tag}
                      recommended={row.recommended}
                      metrics={row.m}
                      tip={tab === "encoder" ? `${g(`encoders.${row.vendor}`)} ${g(`codecs.${row.codec}`)}` : g(`${tab === "container" ? "containers" : "audio"}.${row.v}`)}
                      selected={current === row.v}
                      unavailable={tab === "encoder" && encodersLoaded && availability[row.v] === false}
                      onSelect={() => select(row.v)} />
                    {current === row.v && selectedNote}
                  </div>
                ))
              )}
              {tab === "encoder" && encodersLoaded && (hiddenEncoderCount > 0 || showAllEncoders) && (
                <button type="button" onClick={() => setShowAllEncoders((v) => !v)}
                  className="mx-3 mt-1 self-start text-[11px] font-medium text-stone-500 underline-offset-2 hover:text-stone-300 hover:underline">
                  {showAllEncoders ? g("hideUnavailable") : g("showHidden")(hiddenEncoderCount)}
                </button>
              )}
            </div>
          );
        })()}
      </div>
    </div>
  );
}

// One quality stat as a tile: label on top, value underneath, with an
// invisible native <select> stretched over it so clicking anywhere opens the picker.
function StatTile({ label, display, unit, value, onChange, children, big = true }) {
  return (
    <div className="group relative flex min-w-0 flex-col justify-center gap-0.5 rounded-xl border border-stone-800 bg-stone-950 px-3 py-2.5 transition hover:border-stone-600">
      <span className="text-[10px] font-medium uppercase tracking-wider text-stone-500">{label}</span>
      <span className={`truncate font-bold text-stone-100 ${big ? "text-lg" : "text-sm leading-6"}`}>
        {display}
        {unit && <span className="ml-1 text-[11px] font-medium text-stone-500">{unit}</span>}
      </span>
      <MdUnfoldMore size={14} className="absolute right-2 top-2.5 text-stone-700 transition group-hover:text-stone-400" />
      {/* opacity-0 hides the box but the popup list still takes its colors
          from the select — unstyled it renders white-on-white. */}
      <select value={value} onChange={onChange} title={label}
        className="absolute inset-0 h-full w-full cursor-pointer bg-stone-800 text-stone-100 opacity-0">
        {children}
      </select>
    </div>
  );
}

// A quiet divider-label between grouped tiles, so the flat wall of knobs
// reads as Video / Encoding / Output instead of one undifferentiated grid.
function SectionLabel({ children }) {
  return (
    <div className="mb-2 mt-4 flex items-center gap-2.5 first:mt-1">
      <span className="text-[10px] font-semibold uppercase tracking-widest text-stone-500">{children}</span>
      <span className="h-px flex-1 bg-stone-800" />
    </div>
  );
}

// The expandable header shared by the "compare" guide and the capture-
// behavior block — both collapsed by default to keep the quality tab compact.
function Disclosure({ open, onToggle, icon: Icn, label }) {
  return (
    <button type="button" onClick={onToggle}
      className="flex w-full items-center justify-between rounded-xl border border-stone-800 bg-stone-950/60 px-3 py-2.5 text-left transition hover:border-stone-600">
      <span className="flex items-center gap-2 text-xs font-semibold text-stone-300">
        {Icn && <Icn size={15} className="text-accent-400" />}
        {label}
      </span>
      {open ? <MdExpandLess size={18} className="text-stone-500" /> : <MdExpandMore size={18} className="text-stone-500" />}
    </button>
  );
}

export function RecordSettingsCard({ settings, apply, t }) {
  const video = settings.video ?? {};
  const [encoders, setEncoders] = useState(null); // null = loading
  const [refreshing, setRefreshing] = useState(false);
  const [showGuide, setShowGuide] = useState(false);
  const [showCapture, setShowCapture] = useState(false);

  const loadEncoders = async () => {
    setRefreshing(true);
    try { setEncoders(await invoke("list_available_encoders")); }
    catch { setEncoders([]); }
    finally { setRefreshing(false); }
  };

  useEffect(() => { loadEncoders(); }, []);

  const availability = Object.fromEntries((encoders ?? []).map((e) => [e.kind, e.available]));

  const applyVideo = (patch) => apply({ video: { ...video, ...patch } });
  const showBitrate = rateControlUsesBitrate(video.rate_control ?? "cbr");
  const showQuality = rateControlUsesQuality(video.rate_control ?? "cbr");

  const activePreset = QUALITY_PRESETS.find(
    (p) => (video.fps ?? 30) === p.fps && (video.bitrate_kbps ?? 12000) === p.bitrate_kbps
  )?.id ?? "custom";

  return (
    <Card
      title={t("settings.quality.title")}
      right={
        <button onClick={loadEncoders} disabled={refreshing}
          className="text-xs px-2 py-1 rounded bg-stone-800 hover:bg-stone-700 text-stone-300 transition disabled:opacity-50">
          {refreshing ? "…" : t("settings.video.refreshEncoders")}
        </button>
      }
    >
      <div className="py-3">
        {/* Preset pills; "custom" is a state indicator that any tile edit
            away from the preset values flips to automatically. */}
        <div className="flex flex-wrap items-center justify-between gap-2">
          <div className="flex items-center rounded-lg bg-stone-950 p-0.5">
            {QUALITY_PRESETS.map((p) => (
              <button key={p.id} type="button"
                onClick={() => applyVideo({ fps: p.fps, bitrate_kbps: p.bitrate_kbps })}
                className={`rounded-md px-3 py-1.5 text-xs font-semibold transition ${
                  activePreset === p.id ? "bg-stone-700 text-stone-100" : "text-stone-500 hover:text-stone-300"
                }`}>
                {t(`settings.quality.presets.${p.id}.label`)}
              </button>
            ))}
            <span className={`flex items-center gap-1 rounded-md px-3 py-1.5 text-xs font-semibold ${
              activePreset === "custom" ? "bg-accent-500/20 text-accent-300" : "text-stone-600"
            }`}>
              <MdTune size={13} />
              {t("settings.quality.presets.custom.label")}
            </span>
          </div>
          <span className="text-xs text-stone-500">{t(`settings.quality.presets.${activePreset}.desc`)}</span>
        </div>

        {/* Knobs grouped into Video / Encoding / Output so the grid reads in
            logical chunks instead of one long undifferentiated row. Capped at
            3 columns since `lg:` is a viewport breakpoint, not a container
            one, and this card's actual width can be narrower. */}
        <SectionLabel>{t("settings.quality.sections.video")}</SectionLabel>
        <div className="grid grid-cols-2 gap-2">
          <StatTile label={t("settings.video.resolution")} display={RESOLUTION_LABELS[video.resolution ?? "native"]}
            value={video.resolution ?? "native"} onChange={(e) => applyVideo({ resolution: e.target.value })}>
            {RESOLUTION_OPTIONS.map(([v, label]) => <option key={v} value={v}>{label}</option>)}
          </StatTile>
          <StatTile label={t("settings.video.fps")} display={video.fps ?? 30} unit="FPS"
            value={video.fps ?? 30} onChange={(e) => applyVideo({ fps: Number(e.target.value) })}>
            {FPS_OPTIONS.map((f) => <option key={f} value={f}>{f}</option>)}
          </StatTile>
        </div>

        <SectionLabel>{t("settings.quality.sections.encoding")}</SectionLabel>
        <div className="grid grid-cols-2 gap-2 sm:grid-cols-3">
          <StatTile label={t("settings.video.encoder")} display={ENCODER_LABELS[video.encoder ?? "auto"]} big={false}
            value={video.encoder ?? "auto"} onChange={(e) => applyVideo({ encoder: e.target.value })}>
            {ENCODER_GROUPS.map((group) => (
              <optgroup key={group.label} label={group.label}>
                {group.options.map((opt) => {
                  const disabled = opt !== "auto" && encoders !== null && availability[opt] === false;
                  return (
                    <option key={opt} value={opt} disabled={disabled}>
                      {ENCODER_LABELS[opt]}{disabled ? ` (${t("settings.video.unavailable")})` : ""}
                    </option>
                  );
                })}
              </optgroup>
            ))}
          </StatTile>
          <StatTile label={t("settings.video.rateControl")} display={t(`settings.video.rateControlOptions.${video.rate_control ?? "cbr"}`)} big={false}
            value={video.rate_control ?? "cbr"} onChange={(e) => applyVideo({ rate_control: e.target.value })}>
            {RATE_CONTROL_OPTIONS.map((rc) => <option key={rc} value={rc}>{t(`settings.video.rateControlOptions.${rc}`)}</option>)}
          </StatTile>
          {showBitrate && (
            <StatTile label={bitrateFieldLabel(t, video.rate_control ?? "cbr")} display={((video.bitrate_kbps ?? 12000) / 1000).toFixed(0)} unit="Mbps"
              value={video.bitrate_kbps ?? 12000} onChange={(e) => applyVideo({ bitrate_kbps: Number(e.target.value) })}>
              {[...new Set([...BITRATE_OPTIONS, video.bitrate_kbps ?? 12000])].sort((a, b) => a - b).map((b) => (
                <option key={b} value={b}>{(b / 1000).toFixed(0)}M</option>
              ))}
            </StatTile>
          )}
          {showQuality && (
            <StatTile label={t("settings.video.quality")} display={video.quality ?? 23} big={false}
              value={video.quality ?? 23} onChange={(e) => applyVideo({ quality: Number(e.target.value) })}>
              {[...new Set([...QUALITY_OPTIONS, video.quality ?? 23])].sort((a, b) => a - b).map((q) => (
                <option key={q} value={q}>{q}</option>
              ))}
            </StatTile>
          )}
        </div>

        <SectionLabel>{t("settings.quality.sections.output")}</SectionLabel>
        <div className="grid grid-cols-2 gap-2">
          <StatTile label={t("settings.video.container")} display={CONTAINER_LABELS[video.container ?? "mp4"]} big={false}
            value={video.container ?? "mp4"} onChange={(e) => applyVideo({ container: e.target.value })}>
            {CONTAINER_OPTIONS.map((c) => <option key={c} value={c}>{CONTAINER_LABELS[c]}</option>)}
          </StatTile>
          <StatTile label={t("settings.video.audioCodec")} display={AUDIO_CODEC_LABELS[video.audio_codec ?? "aac"]} big={false}
            value={video.audio_codec ?? "aac"} onChange={(e) => applyVideo({ audio_codec: e.target.value })}>
            {AUDIO_CODECS.map(([v, label]) => <option key={v} value={v}>{label}</option>)}
          </StatTile>
        </div>

        {/* The full "which should I pick?" comparison matrix is dense, so it's
            collapsed by default — one click reveals every encoder/container/
            audio codec scored side by side (rows apply the selection too). */}
        <div className="mt-4">
          <Disclosure open={showGuide} onToggle={() => setShowGuide((v) => !v)}
            icon={MdInfoOutline} label={t("settings.quality.guideToggle")} />
          {showGuide && (
            <CodecGuide video={video} applyVideo={applyVideo}
              availability={availability} encodersLoaded={encoders !== null} t={t} />
          )}
        </div>
      </div>

      {/* Capture behavior lives behind its own disclosure — it's set-once
          plumbing, not something touched per recording, so it stays out of
          the way of the quality knobs above. */}
      <div className="py-3">
        <Disclosure open={showCapture} onToggle={() => setShowCapture((v) => !v)}
          icon={MdDesktopWindows} label={t("settings.quality.captureTitle")} />
        {showCapture && (
          <div className="mt-1 divide-y divide-stone-800/70">
            <Row label={t("settings.video.captureCursor")}>
              <Toggle labeled checked={video.capture_cursor ?? true} onChange={(v) => applyVideo({ capture_cursor: v })} />
            </Row>

            <Row label={t("settings.video.excludeOverlayWindows")} hint={t("settings.video.excludeOverlayWindowsHint")}>
              <Toggle labeled checked={video.exclude_overlay_windows ?? false} onChange={(v) => applyVideo({ exclude_overlay_windows: v })} />
            </Row>

            <Row label={t("settings.video.cropTitlebar")} hint={t("settings.video.cropTitlebarHint")}>
              <Toggle labeled checked={video.crop_titlebar ?? true} onChange={(v) => applyVideo({ crop_titlebar: v })} />
            </Row>

            <Row label={t("settings.video.minimizedBehavior")} hint={t("settings.video.minimizedBehaviorHint")}>
              <select value={video.minimized_behavior ?? "branded"}
                onChange={(e) => applyVideo({ minimized_behavior: e.target.value })}
                className={`${inputCls} cursor-pointer`}>
                <option value="branded">{t("settings.video.minimizedOptions.branded")}</option>
                <option value="black">{t("settings.video.minimizedOptions.black")}</option>
                <option value="freeze">{t("settings.video.minimizedOptions.freeze")}</option>
                <option value="pause">{t("settings.video.minimizedOptions.pause")}</option>
              </select>
            </Row>

            <Row label={t("settings.video.hideOverlaysFromCapture")} hint={t("settings.video.hideOverlaysFromCaptureHint")}>
              <Toggle labeled checked={settings.hide_overlays_from_capture ?? true}
                onChange={(v) => apply({ hide_overlays_from_capture: v })} />
            </Row>
          </div>
        )}
      </div>
    </Card>
  );
}

// A single icon glyph, pulled from the shared hudIcons.js library so every
// preview here is pixel-identical to the on-screen HUD.
function BadgeIcon({ iconKey, className }) {
  return <svg viewBox="0 0 24 24" className={className} dangerouslySetInnerHTML={{ __html: HUD_ICONS[iconKey] ?? "" }} />;
}

// Click-to-open icon picker: a round trigger showing the current pick, and
// a popover grid of every other choice.
function IconPicker({ icon, choices, onPick }) {
  const [open, setOpen] = useState(false);
  const rootRef = useRef(null);

  useEffect(() => {
    if (!open) return;
    const onDocClick = (e) => {
      if (rootRef.current && !rootRef.current.contains(e.target)) setOpen(false);
    };
    const onKey = (e) => { if (e.key === "Escape") setOpen(false); };
    document.addEventListener("mousedown", onDocClick);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDocClick);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  return (
    <div ref={rootRef} className="relative">
      <button type="button" onClick={() => setOpen((v) => !v)}
        className={`flex h-9 w-9 items-center justify-center rounded-full border transition ${
          open ? "border-accent-400 bg-stone-800" : "border-stone-700 bg-stone-900 hover:border-stone-600"
        }`}>
        <BadgeIcon iconKey={icon} className="h-4 w-4 fill-stone-100" />
      </button>

      {open && (
        <div className="absolute left-0 top-full z-20 mt-2 grid w-56 grid-cols-6 gap-1 rounded-xl border border-stone-700 bg-stone-900 p-2 shadow-xl">
          {choices.map((key) => (
            <button key={key} type="button"
              onClick={() => { onPick(key); setOpen(false); }}
              className={`flex h-7 w-7 items-center justify-center rounded-lg transition ${
                icon === key ? "bg-accent-500/20 ring-1 ring-accent-500/50" : "hover:bg-stone-800"
              }`}>
              <BadgeIcon iconKey={key} className={`h-3.5 w-3.5 ${icon === key ? "fill-accent-300" : "fill-stone-400"}`} />
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

// One badge's row: the icon picker above, plus its own on/off toggle.
function BadgeRow({ label, hint, enabled, onToggleEnabled, icon, choices, onPickIcon }) {
  return (
    <Row label={label} hint={hint}>
      <div className="flex items-center gap-3">
        <IconPicker icon={icon} choices={choices} onPick={onPickIcon} />
        <Toggle labeled checked={enabled} onChange={onToggleEnabled} />
      </div>
    </Row>
  );
}

// Its own settings page (Settings → Recording Indicator), split out from
// Recording since a corner picker doesn't read naturally next to encoder/bitrate knobs.
export function IndicatorSettingsCard({ settings, apply, t }) {
  const video = settings.video ?? {};
  const badges = video.hud_badges ?? {};
  const applyVideo = (patch) => apply({ video: { ...video, ...patch } });
  const applyBadges = (patch) => applyVideo({ hud_badges: { ...badges, ...patch } });

  return (
    <Card title={t("settings.indicator.title")}>
      <Row label={t("settings.video.hudCorner")} hint={t("settings.video.hudCornerHint")}>
        <HudCornerPicker value={video.hud_corner ?? "top_right"} onChange={(c) => applyVideo({ hud_corner: c })} />
      </Row>

      <BadgeRow
        label={t("settings.indicator.recording")} hint={t("settings.indicator.recordingHint")}
        enabled={badges.recording_enabled ?? true} onToggleEnabled={(v) => applyBadges({ recording_enabled: v })}
        icon={badges.recording_icon ?? "dot"} choices={HUD_ICON_CHOICES.recording}
        onPickIcon={(k) => applyBadges({ recording_icon: k })}
      />
      <BadgeRow
        label={t("settings.indicator.buffer")} hint={t("settings.indicator.bufferHint")}
        enabled={badges.buffer_enabled ?? true} onToggleEnabled={(v) => applyBadges({ buffer_enabled: v })}
        icon={badges.buffer_icon ?? "history"} choices={HUD_ICON_CHOICES.buffer}
        onPickIcon={(k) => applyBadges({ buffer_icon: k })}
      />
      <BadgeRow
        label={t("settings.indicator.mic")} hint={t("settings.indicator.micHint")}
        enabled={badges.mic_enabled ?? true} onToggleEnabled={(v) => applyBadges({ mic_enabled: v })}
        icon={badges.mic_icon ?? "mic"} choices={HUD_ICON_CHOICES.mic}
        onPickIcon={(k) => applyBadges({ mic_icon: k })}
      />
    </Card>
  );
}

// Its own settings page (Settings → Notifications) rather than tucked into
// Recording — this is a "how Capcove talks to me" setting, not a recording one.
export function NotificationSettingsCard({ settings, apply, t }) {
  const video = settings.video ?? {};
  const applyVideo = (patch) => apply({ video: { ...video, ...patch } });

  return (
    <Card title={t("settings.notifications.title")}>
      <Row label={t("settings.video.toastCorner")} hint={t("settings.video.toastCornerHint")}>
        <HudCornerPicker value={video.toast_corner ?? "top_right"} onChange={(c) => applyVideo({ toast_corner: c })} />
      </Row>

      {["recording", "session", "stream", "buffer", "clip"].map((cat) => {
        const checked = video.toast_categories?.[cat] ?? true;
        return (
          <Row key={cat} label={t(`settings.video.toastCategory.${cat}`)} hint={t(`settings.video.toastCategory.${cat}Hint`)}>
            <Toggle labeled checked={checked}
              onChange={(v) => applyVideo({ toast_categories: { ...video.toast_categories, [cat]: v } })} />
          </Row>
        );
      })}
    </Card>
  );
}

const SOUND_PRESETS = ["soft_ping", "marimba", "glass", "soft_bell", "two_tone_up", "two_tone_down", "coin", "success", "alert"];
const DEFAULT_SOUND_SETTING = { enabled: true, source: { kind: "preset", preset: "soft_ping" } };

// A custom source is either a user-browsed file or one of Windows' own
// %SystemRoot%\Media\*.wav sounds — both are the same backend `Custom {
// path }` source, distinguished here only by whether the path happens to
// sit inside `windowsSoundsDir`.
function isWindowsSoundPath(path, windowsSoundsDir) {
  return !!path && !!windowsSoundsDir && path.toLowerCase().startsWith(windowsSoundsDir.toLowerCase());
}

// One event's row: source-type selector (preset / Windows sound / custom
// file), the matching picker, and a preview button that plays regardless of
// the toggle — auditioning a sound is the whole point of clicking it.
function SoundEffectRow({ label, hint, setting, onChange, t, windowsSounds, windowsSoundsDir }) {
  const source = setting.source ?? DEFAULT_SOUND_SETTING.source;
  const isWindows = source.kind === "custom" && isWindowsSoundPath(source.path, windowsSoundsDir);
  const isCustomFile = source.kind === "custom" && !isWindows;
  const kind = source.kind === "preset" ? "preset" : isWindows ? "windows" : "custom";
  const [picking, setPicking] = useState(false);

  const preview = () => invoke("preview_sound_effect", { source }).catch(() => {});

  const pickFile = async () => {
    setPicking(true);
    try {
      const path = await invoke("pick_sound_file").catch(() => null);
      if (path) onChange({ ...setting, source: { kind: "custom", path } });
    } finally {
      setPicking(false);
    }
  };

  return (
    <Row label={label} hint={hint}>
      <div className="flex items-center gap-2">
        <select value={kind} className={`${inputCls} w-32 shrink-0 truncate`}
          onChange={(e) => {
            const next = e.target.value;
            if (next === "preset") onChange({ ...setting, source: { kind: "preset", preset: SOUND_PRESETS[0] } });
            else if (next === "windows") onChange({ ...setting, source: { kind: "custom", path: windowsSounds[0]?.path ?? "" } });
            else onChange({ ...setting, source: { kind: "custom", path: "" } });
          }}>
          <option value="preset">{t("settings.sounds.preset")}</option>
          {windowsSounds.length > 0 && <option value="windows">{t("settings.sounds.windowsSound")}</option>}
          <option value="custom">{t("settings.sounds.custom")}</option>
        </select>

        {/* Fixed width regardless of which of the three renders — otherwise
            switching a row's type (or just having different rows on
            different types) shifts the preview button/toggle out of a
            consistent column. */}
        {kind === "preset" && (
          <select value={source.preset} className={`${inputCls} w-44 shrink-0 truncate`}
            onChange={(e) => onChange({ ...setting, source: { kind: "preset", preset: e.target.value } })}>
            {SOUND_PRESETS.map((p) => <option key={p} value={p}>{t(`settings.sounds.presets.${p}`)}</option>)}
          </select>
        )}
        {kind === "windows" && (
          <select value={source.path} className={`${inputCls} w-44 shrink-0 truncate`}
            onChange={(e) => onChange({ ...setting, source: { kind: "custom", path: e.target.value } })}>
            {windowsSounds.map((s) => <option key={s.path} value={s.path}>{s.name}</option>)}
          </select>
        )}
        {isCustomFile && (
          <Button onClick={pickFile} disabled={picking} className="w-44 shrink-0 truncate">
            {picking ? t("common.loading") : (source.path ? source.path.split(/[\\/]/).pop() : t("common.browse"))}
          </Button>
        )}

        <button onClick={preview} title={t("settings.sounds.preview")}
          className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-stone-800 text-stone-300 transition hover:bg-stone-700">
          <MdPlayArrow size={16} />
        </button>
        <Toggle checked={setting.enabled} onChange={(v) => onChange({ ...setting, enabled: v })} />
      </div>
    </Row>
  );
}

// Its own page under Alerts — a sound per recording/replay-buffer state
// change, each independently toggleable and pointed at a synthesized preset
// tone, one of Windows' own system sounds, or a user-picked audio file.
export function SoundEffectsCard({ settings, apply, t }) {
  const se = settings.sound_effects ?? {};
  const applySound = (key, next) => apply({ sound_effects: { ...se, [key]: next } });
  const [windowsSounds, setWindowsSounds] = useState([]);

  useEffect(() => { invoke("list_windows_sounds").then(setWindowsSounds).catch(() => {}); }, []);
  // Every Windows sound lives in the same directory — used to tell "a
  // Windows sound" apart from "a file the user browsed to" (see
  // `isWindowsSoundPath`), without needing a third backend source kind.
  const windowsSoundsDir = windowsSounds[0]?.path.replace(/[^\\/]+$/, "") ?? "";

  const EVENTS = [
    ["recording_started", "recordingStarted"],
    ["recording_stopped", "recordingStopped"],
    ["buffer_started", "bufferStarted"],
    ["buffer_stopped", "bufferStopped"],
    ["clip_saved", "clipSaved"],
  ];

  return (
    <Card title={t("settings.sounds.title")}>
      {EVENTS.map(([key, labelKey]) => (
        <SoundEffectRow key={key}
          label={t(`settings.sounds.${labelKey}`)} hint={t(`settings.sounds.${labelKey}Hint`)}
          setting={se[key] ?? DEFAULT_SOUND_SETTING}
          onChange={(next) => applySound(key, next)}
          windowsSounds={windowsSounds} windowsSoundsDir={windowsSoundsDir}
          t={t} />
      ))}
    </Card>
  );
}

export const AUDIO_CODEC_KBPS = { aac: 192, opus: 160, mp3: 192, flac: 1000 }; // flac has no fixed bitrate — rough ballpark for lossless stereo

export function fmtBufferBytes(bytes) {
  if (!bytes || bytes <= 0) return "0 MB";
  const units = ["B", "KB", "MB", "GB"];
  let v = bytes, i = 0;
  while (v >= 1024 && i < units.length - 1) { v /= 1024; i++; }
  return `${v.toFixed(i >= 2 ? 1 : 0)} ${units[i]}`;
}

// CBR: `bytes` is firm. VBR/VBR+CQ: an average with a 1.5x cap, so `bytes`
// is a ceiling. CQP/Lossless have no bitrate — `bytes` comes from
// `estimateQualityBitrateKbps`/`estimateLosslessBitrateKbps` instead
// (`roughEstimate: true`).
function estimateBufferBytes(video, rb, ov) {
  const useCustom = rb.use_custom_video;
  const bitrateKbps = (useCustom && ov.bitrate_kbps != null) ? ov.bitrate_kbps : (video.bitrate_kbps ?? 8000);
  const rateControl = (useCustom && ov.rate_control != null) ? ov.rate_control : (video.rate_control ?? "cbr");
  const quality = (useCustom && ov.quality != null) ? ov.quality : (video.quality ?? 23);
  const encoder = (useCustom && ov.encoder != null) ? ov.encoder : (video.encoder ?? "auto");
  const resolution = (useCustom && ov.resolution != null) ? ov.resolution : (video.resolution ?? "native");
  const fps = (useCustom && ov.fps != null) ? ov.fps : (video.fps ?? 30);
  const [width, height] = RESOLUTION_DIMENSIONS[resolution] ?? RESOLUTION_DIMENSIONS.native;
  const audioCodec = (useCustom && ov.audio_codec != null) ? ov.audio_codec : (video.audio_codec ?? "aac");
  const perTrackKbps = AUDIO_CODEC_KBPS[audioCodec] ?? 192;

  const audio = video.audio ?? {};
  const qualifying = (audio.sources ?? []).filter((s) => {
    if (s.enabled === false) return false;
    if (s.kind === "system_output" && audio.system_muted) return false;
    if (s.kind === "microphone" && audio.mic_muted) return false;
    return true;
  });
  const audioTracks = qualifying.length === 0 ? 0 : ((audio.separate_tracks ?? true) ? qualifying.length : 1);

  let videoKbps;
  let roughEstimate = false;
  if (rateControl === "cbr") {
    videoKbps = bitrateKbps;
  } else if (rateControl === "cqp") {
    videoKbps = estimateQualityBitrateKbps({ width, height, fps, quality, encoder });
    roughEstimate = true;
  } else if (rateControl === "lossless") {
    videoKbps = estimateLosslessBitrateKbps({ width, height, fps });
    roughEstimate = true;
  } else {
    // vbr / vbr_cq — bitrate is a cap either way, video track only.
    videoKbps = bitrateKbps * 1.5;
  }
  const totalKbps = videoKbps + audioTracks * perTrackKbps;

  const minutes = rb.buffer_minutes ?? 5;
  return {
    bytes: (totalKbps * 1000 / 8) * minutes * 60, // kbps -> bytes/sec -> * seconds
    videoKbps,
    roughEstimate,
    bitrateKbps,
    rateControl,
    audioTracks,
    perTrackKbps,
  };
}

export function ReplayBufferCard({ settings, apply, t }) {
  const video = settings.video ?? {};
  const rb = video.replay_buffer ?? { enabled: false, buffer_minutes: 5, target: { kind: "primary_monitor" }, game_detect_mode: "clips" };
  const ov = rb.video_override ?? {};
  const sizeEstimate = estimateBufferBytes(video, rb, ov);
  const effectiveRateControl = ov.rate_control ?? video.rate_control ?? "cbr";
  const showBitrateOverride = rateControlUsesBitrate(effectiveRateControl);
  const showQualityOverride = rateControlUsesQuality(effectiveRateControl);
  const [encoders, setEncoders] = useState(null); // null = loading

  useEffect(() => {
    invoke("list_available_encoders").then(setEncoders).catch(() => setEncoders([]));
  }, []);

  const availability = Object.fromEntries((encoders ?? []).map((e) => [e.kind, e.available]));

  const applyRb = (patch) => {
    const next = { ...rb, ...patch };
    apply({ video: { ...video, replay_buffer: next } });
    // Backend reads replay_buffer settings only at (re)start — flip the
    // running buffer immediately so the toggle takes effect right away.
    if (patch.enabled === true) invoke("start_replay_buffer").catch(() => {});
    if (patch.enabled === false) invoke("stop_replay_buffer").catch(() => {});
    // Storage mode and video-quality overrides are baked into the running
    // encoder — restart to apply.
    if ((patch.storage || patch.video_override || patch.use_custom_video !== undefined) && rb.enabled) {
      invoke("stop_replay_buffer")
        .then(() => invoke("start_replay_buffer"))
        .catch(() => {});
    }
  };

  const setVideoOverride = (field, value) => applyRb({ video_override: { ...ov, [field]: value } });
  const def = <option value="">{t("settings.games.overrideDefault")}</option>;
  const defaultLabel = t("settings.games.overrideDefault");

  return (
    <Card title={t("settings.replayBuffer.title")}>
      <Row label={t("settings.replayBuffer.enable")} hint={t("settings.replayBuffer.enableHint")}>
        <Toggle labeled checked={rb.enabled} onChange={(v) => applyRb({ enabled: v })} />
      </Row>
      <Row label={t("settings.replayBuffer.altTabPrivacy")} hint={t("settings.replayBuffer.altTabPrivacyHint")}>
        <Toggle labeled checked={rb.alt_tab_privacy ?? false} onChange={(v) => applyRb({ alt_tab_privacy: v })} />
      </Row>
      <Row label={t("settings.replayBuffer.confirmSaveOnClose")} hint={t("settings.replayBuffer.confirmSaveOnCloseHint")}>
        <Toggle labeled checked={rb.confirm_save_on_close ?? true} onChange={(v) => applyRb({ confirm_save_on_close: v })} />
      </Row>
      <Row label={t("settings.replayBuffer.storage")} hint={t("settings.replayBuffer.storageHint")}>
        <div className="flex items-center rounded-lg bg-stone-900 p-0.5">
          {[
            ["disk", t("settings.replayBuffer.storageDisk")],
            ["memory", t("settings.replayBuffer.storageMemory")],
          ].map(([value, label]) => (
            <button key={value} onClick={() => applyRb({ storage: value })}
              className={`rounded-md px-2.5 py-1 text-xs font-medium transition ${
                (rb.storage ?? "disk") === value ? "bg-stone-700 text-stone-100" : "text-stone-500 hover:text-stone-300"
              }`}>
              {label}
            </button>
          ))}
        </div>
      </Row>
      <Row
        label={t("settings.replayBuffer.minutes")}
        hint={
          <>
            {t("settings.replayBuffer.minutesHint")}
            <br />
            {t(
              sizeEstimate.roughEstimate
                ? ((rb.storage ?? "disk") === "memory" ? "settings.replayBuffer.sizeEstimateMemoryRough" : "settings.replayBuffer.sizeEstimateDiskRough")
                : sizeEstimate.rateControl !== "cbr"
                ? ((rb.storage ?? "disk") === "memory" ? "settings.replayBuffer.sizeEstimateMemoryUpTo" : "settings.replayBuffer.sizeEstimateDiskUpTo")
                : ((rb.storage ?? "disk") === "memory" ? "settings.replayBuffer.sizeEstimateMemory" : "settings.replayBuffer.sizeEstimateDisk")
            ).replace("{size}", fmtBufferBytes(sizeEstimate.bytes))}
            {" "}
            <span className="text-stone-600">
              ({sizeEstimate.rateControl.toUpperCase()} · {(sizeEstimate.videoKbps / 1000).toFixed(1)} Mbps video
              {sizeEstimate.audioTracks > 0 && ` + ${sizeEstimate.audioTracks}× ${sizeEstimate.perTrackKbps}kbps audio`})
            </span>
            <br />
            {t("settings.replayBuffer.sizeEstimateNote")}
          </>
        }
      >
        <div className="flex items-center gap-3">
          <input type="range" min="1" max="60" step="1" value={rb.buffer_minutes ?? 5}
            onChange={(e) => applyRb({ buffer_minutes: Number(e.target.value) })}
            className="w-44 accent-accent-500 cursor-pointer" />
          <span className="text-sm font-medium text-stone-300 w-16 text-right">{rb.buffer_minutes ?? 5} min</span>
        </div>
      </Row>
      <Row label={t("settings.replayBuffer.customVideo")} hint={t("settings.replayBuffer.customVideoHint")}>
        <Toggle labeled checked={rb.use_custom_video ?? false} onChange={(v) => applyRb({ use_custom_video: v })} />
      </Row>
      {rb.use_custom_video && (
        <div className="py-3">
          <div className="grid grid-cols-2 gap-2 sm:grid-cols-3">
            <OverrideTile label={t("settings.video.fps")} display={`${ov.fps} FPS`} overridden={ov.fps != null}
              defaultLabel={defaultLabel} value={ov.fps} onChange={(v) => setVideoOverride("fps", v === null ? null : Number(v))}>
              {def}
              {FPS_OPTIONS.map((f) => <option key={f} value={f}>{f} FPS</option>)}
            </OverrideTile>
            <OverrideTile label={t("settings.video.resolution")} display={RESOLUTION_LABELS[ov.resolution] ?? ""}
              overridden={ov.resolution != null} defaultLabel={defaultLabel} value={ov.resolution}
              onChange={(v) => setVideoOverride("resolution", v)}>
              {def}
              {RESOLUTION_OPTIONS.map(([v, label]) => <option key={v} value={v}>{label}</option>)}
            </OverrideTile>
            <OverrideTile label={t("settings.video.rateControl")} display={t(`settings.video.rateControlOptions.${ov.rate_control}`)}
              overridden={ov.rate_control != null} defaultLabel={defaultLabel} value={ov.rate_control}
              onChange={(v) => setVideoOverride("rate_control", v)}>
              {def}
              {RATE_CONTROL_OPTIONS.map((rc) => <option key={rc} value={rc}>{t(`settings.video.rateControlOptions.${rc}`)}</option>)}
            </OverrideTile>
            {showBitrateOverride && (
              <OverrideTile label={bitrateFieldLabel(t, effectiveRateControl)} display={ov.bitrate_kbps != null ? `${(ov.bitrate_kbps / 1000).toFixed(0)} Mbps` : ""}
                overridden={ov.bitrate_kbps != null} defaultLabel={defaultLabel} value={ov.bitrate_kbps}
                onChange={(v) => setVideoOverride("bitrate_kbps", v === null ? null : Number(v))}>
                {def}
                {BITRATE_OPTIONS.map((b) => <option key={b} value={b}>{(b / 1000).toFixed(0)} Mbps</option>)}
              </OverrideTile>
            )}
            {showQualityOverride && (
              <OverrideTile label={t("settings.video.quality")} display={ov.quality != null ? String(ov.quality) : ""}
                overridden={ov.quality != null} defaultLabel={defaultLabel} value={ov.quality}
                onChange={(v) => setVideoOverride("quality", v === null ? null : Number(v))}>
                {def}
                {QUALITY_OPTIONS.map((q) => <option key={q} value={q}>{q}</option>)}
              </OverrideTile>
            )}
            <OverrideTile label={t("settings.video.encoder")} display={ENCODER_LABELS[ov.encoder] ?? ""} overridden={ov.encoder != null}
              defaultLabel={defaultLabel} value={ov.encoder} onChange={(v) => setVideoOverride("encoder", v)}>
              {def}
              {ENCODER_GROUPS.map((group) => (
                <optgroup key={group.label} label={group.label}>
                  {group.options.map((opt) => {
                    const disabled = opt !== "auto" && encoders !== null && availability[opt] === false;
                    return (
                      <option key={opt} value={opt} disabled={disabled}>
                        {ENCODER_LABELS[opt]}{disabled ? ` (${t("settings.video.unavailable")})` : ""}
                      </option>
                    );
                  })}
                </optgroup>
              ))}
            </OverrideTile>
            <OverrideTile label={t("settings.video.container")} display={ov.container ? CONTAINER_LABELS[ov.container] : ""} overridden={ov.container != null}
              defaultLabel={defaultLabel} value={ov.container} onChange={(v) => setVideoOverride("container", v)}>
              {def}
              {CONTAINER_OPTIONS.map((c) => <option key={c} value={c}>{CONTAINER_LABELS[c]}</option>)}
            </OverrideTile>
            <OverrideTile label={t("settings.video.audioCodec")} display={(AUDIO_CODECS.find(([v]) => v === ov.audio_codec) ?? [])[1] ?? ""}
              overridden={ov.audio_codec != null} defaultLabel={defaultLabel} value={ov.audio_codec}
              onChange={(v) => setVideoOverride("audio_codec", v)}>
              {def}
              {AUDIO_CODECS.map(([v, label]) => <option key={v} value={v}>{label}</option>)}
            </OverrideTile>
          </div>
        </div>
      )}
    </Card>
  );
}

// Per-row toggle for first-track ("Mix") membership, since most players use only
// the first audio track. stopPropagation/preventDefault avoid triggering the row's own click.
function MixChip({ on, onToggle, t }) {
  return (
    <button type="button"
      onClick={(e) => { e.stopPropagation(); e.preventDefault(); onToggle(); }}
      title={t("settings.audio.mainMixHint")}
      className={`shrink-0 rounded-full border px-2 py-0.5 text-[10px] font-semibold transition ${
        on ? "border-accent-500/50 bg-accent-500/15 text-accent-300" : "border-stone-700 bg-stone-900 text-stone-600 hover:border-stone-600"
      }`}>
      {t("settings.audio.mainMix")}
    </button>
  );
}

// A manually-renamed track name survives switching the underlying device —
// only a still-default label gets replaced by the new device's name.
function labelForDeviceSwitch(existingLabel, prevDeviceLabel, newDeviceLabel) {
  const customized = existingLabel && existingLabel !== (prevDeviceLabel ?? "");
  return customized ? existingLabel : (newDeviceLabel ?? "");
}

// One "primary device" row: enable checkbox + device dropdown, managing the
// FIRST source of `kind` in the sources array. Renameable track name on top,
// device dropdown below, so renaming never hides the selected device.
function PrimaryDeviceRow({ icon: Icn, label, kind, devices, sources, applyAudio, showMix, t }) {
  const source = sources.find((s) => s.kind === kind);
  const on = Boolean(source);
  const defaultDev = devices.find((d) => d.is_default);

  const setDevice = (deviceId) => {
    const dev = devices.find((d) => d.id === deviceId);
    const idx = sources.findIndex((s) => s.kind === kind);
    const existing = idx >= 0 ? sources[idx] : null;
    const prevDevice = existing ? devices.find((d) => d.id === existing.device_id) : null;
    const label = labelForDeviceSwitch(existing?.label, prevDevice?.label, dev?.label);
    const entry = { ...existing, kind, device_id: deviceId, label, enabled: true };
    if (idx >= 0) {
      applyAudio(sources.map((s, i) => (i === idx ? entry : s)));
    } else {
      applyAudio([...sources, entry]);
    }
  };

  const toggle = () => {
    if (on) {
      const idx = sources.findIndex((s) => s.kind === kind);
      applyAudio(sources.filter((_, i) => i !== idx));
    } else {
      setDevice(""); // enable following the OS default device
    }
  };

  return (
    <div className={`flex items-start gap-3 rounded-lg px-3 py-2.5 transition ${on ? "bg-stone-950" : "bg-stone-950/50"}`}>
      <input type="checkbox" checked={on} onChange={toggle} className="mt-1 h-4 w-4 shrink-0 cursor-pointer accent-accent-500" />
      <Icn size={16} className={`mt-1 shrink-0 ${on ? "text-stone-300" : "text-stone-600"}`} />
      <div className="min-w-0 flex-1">
        {/* Track name recorded into the file (ffmpeg stream title) —
            defaults to the device's name but is freely renameable; see
            `labelForDeviceSwitch` for why it survives a device switch. */}
        <input
          type="text"
          value={source?.label ?? ""}
          placeholder={label}
          disabled={!on}
          onChange={(e) => applyAudio(sources.map((s) => (s === source ? { ...s, label: e.target.value } : s)))}
          title={t("settings.audio.renameTrack")}
          className={`w-full truncate bg-transparent text-sm outline-none placeholder:text-stone-500 disabled:cursor-default ${
            on ? "font-medium text-stone-100" : "text-stone-400"
          }`}
        />
        <select
          value={source?.device_id ?? ""}
          onChange={(e) => setDevice(e.target.value)}
          disabled={!on}
          className={`${inputCls} mt-1 w-full cursor-pointer truncate !py-1 !text-xs !text-stone-500 disabled:cursor-default disabled:opacity-40`}
        >
          <option value="">{t("settings.audio.defaultDevice")}{defaultDev ? ` (${defaultDev.label})` : ""}</option>
          {devices.map((d) => <option key={d.id} value={d.id}>{d.label}</option>)}
        </select>
      </div>
      {on && showMix && (
        <MixChip t={t} on={source?.main_mix ?? true}
          onToggle={() => applyAudio(sources.map((s) => (s === source ? { ...s, main_mix: !(s.main_mix ?? true) } : s)))} />
      )}
    </div>
  );
}

// One application row. `exeIcon` (extracted from the process's own binary,
// backend-side) wins over the game-catalog lookup by name — non-game apps
// like Discord only have the former.
function AppSourceRow({ exe, name, exeIcon, running, checked, onToggle, mixOn, onToggleMix, onRename, t }) {
  const catalogIcon = useAppIcon(name);
  const icon = exeIcon ?? catalogIcon;
  return (
    <label className={`flex cursor-pointer items-start gap-3 rounded-lg px-3 py-2.5 transition ${
      checked ? "bg-stone-950" : "bg-stone-950/50 hover:bg-stone-950"
    } ${running ? "" : "opacity-60"}`}>
      <input type="checkbox" checked={checked} onChange={onToggle} className="mt-1 h-4 w-4 shrink-0 cursor-pointer accent-accent-500" />
      {icon
        ? <img src={icon} alt="" className="mt-0.5 h-5 w-5 shrink-0 rounded" />
        : <MdApps size={17} className={`mt-0.5 shrink-0 ${checked ? "text-stone-300" : "text-stone-600"}`} />}
      <div className="min-w-0 flex-1">
        {/* Track name (ffmpeg stream title) is only editable once added,
            since there's no source entry to save the rename onto yet. */}
        {checked ? (
          <input
            type="text"
            value={name}
            onChange={(e) => onRename(e.target.value)}
            onClick={(e) => e.stopPropagation()}
            title={t("settings.audio.renameTrack")}
            className="w-full truncate bg-transparent text-sm font-medium text-stone-100 outline-none"
          />
        ) : (
          <span className="block truncate text-sm text-stone-400">{name}</span>
        )}
        <span className="block truncate text-[11px] text-stone-600">{exe}</span>
      </div>
      {checked && onToggleMix && <MixChip t={t} on={mixOn} onToggle={onToggleMix} />}
      {!running && <span className="shrink-0 rounded bg-stone-800 px-1.5 py-0.5 text-[10px] text-stone-500">{t("settings.audio.notRunning")}</span>}
    </label>
  );
}

export function AudioSettingsCard({ settings, apply, t }) {
  const video = settings.video ?? {};
  const sources = video.audio?.sources ?? [];
  const [devices, setDevices] = useState(null); // null = loading
  const [apps, setApps] = useState([]); // running audio sessions
  const [refreshing, setRefreshing] = useState(false);
  const [addingDevice, setAddingDevice] = useState(false);

  const load = async () => {
    setRefreshing(true);
    try { setDevices(await invoke("list_audio_devices")); }
    catch { setDevices({ outputs: [], inputs: [] }); }
    try { setApps(await invoke("list_audio_apps")); }
    catch { setApps([]); }
    setRefreshing(false);
  };

  useEffect(() => { load(); }, []);

  // Live app list while the page is open — an app that starts playing audio
  // appends to the list within seconds without a manual refresh. Devices
  // stay manual since they rarely change.
  useEffect(() => {
    const id = setInterval(() => {
      invoke("list_audio_apps").then(setApps).catch(() => {});
    }, 4000);
    return () => clearInterval(id);
  }, []);

  const applyAudio = (nextSources) => apply({ video: { ...video, audio: { ...video.audio, sources: nextSources } } });
  const applyAudioCfg = (patch) => apply({ video: { ...video, audio: { ...video.audio, ...patch } } });
  const separateOn = video.audio?.separate_tracks ?? true;

  // Primary rows own the FIRST system_output / microphone entry; anything
  // beyond those two is a "custom device" extra track.
  const firstIdx = (kind) => sources.findIndex((s) => s.kind === kind);
  const extraDevices = sources
    .map((s, i) => ({ s, i }))
    .filter(({ s, i }) => (s.kind === "system_output" || s.kind === "microphone") && i !== firstIdx(s.kind));

  // App rows: union of running audio sessions and already-selected apps
  // (a selected app that isn't running stays listed so it can be unchecked).
  const appSources = sources.filter((s) => s.kind === "application");
  const appRows = [
    // A renamed track's custom label wins over the live catalog name,
    // otherwise relaunching the app would revert the display each time.
    ...apps.map((a) => {
      const existing = appSources.find((s) => s.exe.toLowerCase() === a.exe.toLowerCase());
      return { exe: a.exe, name: existing?.label || a.name, exeIcon: a.icon ?? null, running: true };
    }),
    ...appSources
      .filter((s) => !apps.some((a) => a.exe.toLowerCase() === s.exe.toLowerCase()))
      .map((s) => ({ exe: s.exe, name: s.label || s.exe, exeIcon: null, running: false })),
  ];

  const toggleApp = (exe, name) => {
    const idx = sources.findIndex((s) => s.kind === "application" && s.exe.toLowerCase() === exe.toLowerCase());
    if (idx >= 0) {
      applyAudio(sources.filter((_, i) => i !== idx));
    } else {
      applyAudio([...sources, { kind: "application", exe, label: name, enabled: true }]);
    }
  };

  const allDevices = [
    ...(devices?.outputs ?? []).map((d) => ({ ...d, kind: "system_output" })),
    ...(devices?.inputs ?? []).map((d) => ({ ...d, kind: "microphone" })),
  ];

  return (
    <Card
      title={t("settings.audio.title")}
      right={
        <button onClick={load} disabled={refreshing}
          className="text-xs px-2 py-1 rounded bg-stone-800 hover:bg-stone-700 text-stone-300 transition disabled:opacity-50">
          {refreshing ? "…" : t("settings.video.refreshEncoders")}
        </button>
      }
    >
      {/* Primary device rows. Custom (extra) devices sit right underneath
          in the same row design: their checkbox flips the persisted
          `enabled` flag instead of removing the entry. */}
      <div className="flex flex-col gap-1.5 py-3">
        {/* Separate-tracks master switch: ON = every source gets its own
            track, first track = the curated Mix. OFF = everything
            collapses into one mixed track. */}
        <div className="flex items-center justify-between gap-4 rounded-lg border border-stone-800 bg-stone-950/60 px-3 py-2.5">
          <div className="min-w-0">
            <div className="flex items-center gap-1.5 text-sm text-stone-200">
              <MdSportsEsports size={15} className="shrink-0 text-stone-400" />
              {t("settings.audio.separateTracks")}
            </div>
            <div className="mt-0.5 text-xs text-stone-500">
              {separateOn ? t("settings.audio.separateTracksOnHint") : t("settings.audio.separateTracksOffHint")}
            </div>
          </div>
          <div className="flex shrink-0 items-center gap-2">
            {separateOn && (
              <MixChip t={t} on={video.audio?.game_track_main_mix ?? true}
                onToggle={() => applyAudioCfg({ game_track_main_mix: !(video.audio?.game_track_main_mix ?? true) })} />
            )}
            <Toggle labeled checked={separateOn}
              onChange={(v) => applyAudioCfg({ separate_tracks: v })} />
          </div>
        </div>

        {/* Game-audio-only: during game sessions, System Audio is skipped
            and the game's process loopback stands in, avoiding doubled audio
            on virtual-device setups. */}
        <div className="flex items-center justify-between gap-4 rounded-lg border border-stone-800 bg-stone-950/60 px-3 py-2.5">
          <div className="min-w-0">
            <div className="text-sm text-stone-200">{t("settings.audio.gameOnly")}</div>
            <div className="mt-0.5 text-xs text-stone-500">{t("settings.audio.gameOnlyHint")}</div>
          </div>
          <Toggle labeled checked={video.audio?.game_audio_only ?? false}
            onChange={(v) => applyAudioCfg({ game_audio_only: v })} />
        </div>

        <PrimaryDeviceRow icon={MdDesktopWindows} label={t("settings.audio.systemAudio")} kind="system_output"
          devices={devices?.outputs ?? []} sources={sources} applyAudio={applyAudio} showMix={separateOn} t={t} />
        <PrimaryDeviceRow icon={MdMic} label={t("settings.audio.microphone")} kind="microphone"
          devices={devices?.inputs ?? []} sources={sources} applyAudio={applyAudio} showMix={separateOn} t={t} />

        {extraDevices.map(({ s, i }) => {
          const on = s.enabled ?? true;
          // Same two-line shape as `PrimaryDeviceRow`: renameable track
          // name on top, reassignable device below.
          const deviceList = s.kind === "microphone" ? (devices?.inputs ?? []) : (devices?.outputs ?? []);
          const setExtraDevice = (deviceId) => {
            const dev = deviceList.find((d) => d.id === deviceId);
            const prevDevice = deviceList.find((d) => d.id === s.device_id);
            applyAudio(sources.map((x, j) => (j === i
              ? { ...x, device_id: deviceId, label: labelForDeviceSwitch(x.label, prevDevice?.label, dev?.label) }
              : x
            )));
          };
          return (
            <div key={`${s.kind}:${s.device_id}:${i}`}
              className={`flex items-start gap-3 rounded-lg px-3 py-2.5 transition ${on ? "bg-stone-950" : "bg-stone-950/50"}`}>
              <input type="checkbox" checked={on}
                onChange={() => applyAudio(sources.map((x, j) => (j === i ? { ...x, enabled: !on } : x)))}
                className="mt-1 h-4 w-4 shrink-0 cursor-pointer accent-accent-500" />
              {s.kind === "microphone"
                ? <MdMic size={16} className={`mt-1 shrink-0 ${on ? "text-stone-300" : "text-stone-600"}`} />
                : <MdDesktopWindows size={16} className={`mt-1 shrink-0 ${on ? "text-stone-300" : "text-stone-600"}`} />}
              <div className="min-w-0 flex-1">
                <input
                  type="text"
                  value={s.label ?? ""}
                  placeholder={s.device_id}
                  onChange={(e) => applyAudio(sources.map((x, j) => (j === i ? { ...x, label: e.target.value } : x)))}
                  title={t("settings.audio.renameTrack")}
                  className={`w-full truncate bg-transparent text-sm outline-none placeholder:text-stone-500 ${
                    on ? "font-medium text-stone-100" : "text-stone-400"
                  }`}
                />
                <select
                  value={s.device_id}
                  onChange={(e) => setExtraDevice(e.target.value)}
                  className={`${inputCls} mt-1 w-full cursor-pointer truncate !py-1 !text-xs !text-stone-500`}
                >
                  {!deviceList.some((d) => d.id === s.device_id) && (
                    <option value={s.device_id}>{s.label || s.device_id}</option>
                  )}
                  {deviceList.map((d) => <option key={d.id} value={d.id}>{d.label}</option>)}
                </select>
              </div>
              {on && separateOn && (
                <MixChip t={t} on={s.main_mix ?? true}
                  onToggle={() => applyAudio(sources.map((x, j) => (j === i ? { ...x, main_mix: !(x.main_mix ?? true) } : x)))} />
              )}
              <button onClick={() => applyAudio(sources.filter((_, j) => j !== i))}
                title={t("common.delete")}
                className="mt-1 shrink-0 rounded p-1 text-stone-600 transition hover:bg-stone-800 hover:text-red-400">
                <MdClose size={14} />
              </button>
            </div>
          );
        })}

        {addingDevice && (
          <select
            className={`${inputCls} w-full cursor-pointer`}
            value=""
            onChange={(e) => {
              const d = allDevices.find((x) => x.id === e.target.value);
              if (d) applyAudio([...sources, { kind: d.kind, device_id: d.id, label: d.label, enabled: true }]);
              setAddingDevice(false);
            }}
          >
            <option value="" disabled>{t("settings.audio.pickDevice")}</option>
            {allDevices.map((d) => (
              <option key={`${d.kind}:${d.id}`} value={d.id}>
                {d.kind === "microphone" ? "🎤 " : "🔊 "}{d.label}
              </option>
            ))}
          </select>
        )}
        <button onClick={() => setAddingDevice((v) => !v)}
          className="flex items-center gap-1 self-start px-1 text-xs font-medium text-accent-400 transition hover:text-accent-300">
          + {t("settings.audio.addDevice")}
        </button>
        {separateOn && <div className="px-1 text-[11px] text-stone-600">{t("settings.audio.mainMixHint")}</div>}
      </div>

      <div className="py-3">
        <div className="mb-1.5 text-xs uppercase tracking-wide text-stone-500">{t("settings.audio.apps")}</div>
        <div className="flex flex-col gap-1.5">
          {appRows.map((row) => (
            <AppSourceRow key={row.exe.toLowerCase()} {...row} t={t}
              checked={appSources.some((s) => s.exe.toLowerCase() === row.exe.toLowerCase())}
              onToggle={() => toggleApp(row.exe, row.name)}
              mixOn={appSources.find((s) => s.exe.toLowerCase() === row.exe.toLowerCase())?.main_mix ?? true}
              onToggleMix={separateOn ? () => applyAudio(sources.map((s) => (
                s.kind === "application" && s.exe.toLowerCase() === row.exe.toLowerCase()
                  ? { ...s, main_mix: !(s.main_mix ?? true) }
                  : s
              ))) : null}
              onRename={(label) => applyAudio(sources.map((s) => (
                s.kind === "application" && s.exe.toLowerCase() === row.exe.toLowerCase()
                  ? { ...s, label }
                  : s
              )))} />
          ))}
        </div>
        {appRows.length === 0 && <div className="text-xs text-stone-600">{t("settings.audio.noApps")}</div>}
        <div className="mt-2 text-[11px] text-stone-600">{t("settings.audio.appsHint")}</div>
      </div>

      {sources.length > 1 && (video.container === "mp4" || video.container === "mp4_fragmented") && (
        <div className="py-2 text-xs text-yellow-400">{t("settings.audio.multiTrackMp4Hint")}</div>
      )}
      {sources.length === 0 && (
        <div className="py-2 text-xs text-stone-500">{t("settings.audio.noneSelectedHint")}</div>
      )}
    </Card>
  );
}
