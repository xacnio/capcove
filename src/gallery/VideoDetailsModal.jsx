import { useEffect, useState } from "react";
import { invoke } from "../lib/tauri.js";
import { useAppIcon } from "./appIcons.js";
import * as Icon from "./icons.jsx";

// Same session cache as `PlayerInfoHeader` — deliberately not shared between
// the two files (different modules, no shared cache module exists yet), but
// keyed the same way so re-opening the same video in either place is cheap.
const detailsCache = new Map();

function fmtBytes(bytes) {
  if (!bytes) return null;
  const units = ["B", "KB", "MB", "GB"];
  let v = bytes, i = 0;
  while (v >= 1024 && i < units.length - 1) { v /= 1024; i++; }
  return `${v.toFixed(i === 0 ? 0 : 1)} ${units[i]}`;
}

function tagsFor(video, tags) {
  if (!video.tags?.length) return [];
  return video.tags.map((id) => tags.find((tg) => tg.id === id)).filter(Boolean);
}

const KIND_LABEL_KEYS = {
  clip: "gallery.details.kindClip",
  youtube_live: "gallery.details.kindYoutubeLive",
  youtube_only: "gallery.details.kindYoutubeOnly",
};

// One label/value pair; skipped entirely by the caller when `value` is nullish
// so the panel never shows an empty/dash row for data that isn't there.
function Row({ label, children }) {
  return (
    <div className="flex items-start justify-between gap-4 py-2">
      <span className="shrink-0 text-xs font-medium text-stone-500">{label}</span>
      <span className="min-w-0 text-right text-[13px] text-stone-200">{children}</span>
    </div>
  );
}

// Read-only info panel for a single card's right-click "Details" — surfaces
// the raw stored `title` (almost always the app's static window-title text,
// see VideoGrid's `displayTitle`) alongside everything else `list_videos`
// resolved, since none of it fits on the card itself.
export default function VideoDetailsModal({ video, tags, t, lang, onClose }) {
  const icon = useAppIcon(video.app);
  const cardTags = tagsFor(video, tags);
  const dateFmt = video.modified != null
    ? new Intl.DateTimeFormat(lang === "tr" ? "tr-TR" : "en-US", {
        dateStyle: "long", timeStyle: "medium",
      }).format(new Date(video.modified))
    : null;
  const fileName = video.name.split(/[\\/]/).pop() || video.name;
  const kindKey = KIND_LABEL_KEYS[video.kind];

  // Drive-only cards carry width/height from Drive's own metadata already —
  // no local file to ffprobe. Local files get the fuller probe (codec/fps/
  // bitrate/audio) here on open.
  const cacheKey = `${video.name}:${video.modified ?? 0}`;
  const [details, setDetails] = useState(() => (video.drive_only ? null : detailsCache.get(cacheKey) ?? null));
  useEffect(() => {
    if (video.drive_only || !video.name) return undefined;
    let cancelled = false;
    if (detailsCache.has(cacheKey)) {
      setDetails(detailsCache.get(cacheKey));
      return undefined;
    }
    invoke("get_video_details", { name: video.name })
      .then((d) => { detailsCache.set(cacheKey, d); if (!cancelled) setDetails(d); })
      .catch(() => {});
    return () => { cancelled = true; };
  }, [cacheKey, video.drive_only, video.name]);

  const width = video.drive_only ? video.width : details?.width;
  const height = video.drive_only ? video.height : details?.height;
  const resolution = width && height ? `${width}×${height}` : null;
  const fps = details?.fps ? `${Math.round(details.fps * 100) / 100}` : null;
  const bitrateKbps = details?.video_bitrate_kbps ?? details?.overall_bitrate_kbps;
  const bitrateLabel = bitrateKbps ? (bitrateKbps >= 1000 ? `${(bitrateKbps / 1000).toFixed(1)} Mbps` : `${bitrateKbps} Kbps`) : null;
  const audioLabel = details?.audio_codec
    ? [details.audio_codec.toUpperCase(), details.audio_channels === 1 ? "Mono" : details.audio_channels === 2 ? "Stereo" : details.audio_channels ? `${details.audio_channels}ch` : null, details.audio_sample_rate ? `${(details.audio_sample_rate / 1000).toFixed(1)} kHz` : null].filter(Boolean).join(" · ")
    : null;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-6" onClick={onClose}>
      <div className="w-full max-w-md rounded-xl border border-stone-800 bg-stone-900 p-4" onClick={(e) => e.stopPropagation()}>
        <div className="mb-3 flex items-center gap-2.5">
          {icon
            ? <img src={icon} alt="" className="h-8 w-8 shrink-0 rounded-lg object-cover" />
            : <span className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-stone-800 text-stone-600"><Icon.Monitor size={16} /></span>}
          <span className="min-w-0 flex-1 truncate text-sm font-semibold text-stone-100" title={fileName}>{fileName}</span>
          <button onClick={onClose} className="rounded p-1 text-stone-500 transition hover:bg-stone-800 hover:text-stone-200">
            <Icon.X size={16} />
          </button>
        </div>

        <div className="divide-y divide-stone-800/70">
          {video.title && (
            <Row label={t("gallery.details.title")}>{video.title}</Row>
          )}
          {video.app && (
            <Row label={t("gallery.details.app")}>{video.app}</Row>
          )}
          {video.folder && (
            <Row label={t("gallery.details.folder")}>{video.folder}</Row>
          )}
          <Row label={t("gallery.details.kind")}>
            {kindKey ? t(kindKey) : t("gallery.details.kindRecording")}
          </Row>
          {dateFmt && <Row label={t("gallery.details.date")}>{dateFmt}</Row>}
          {fmtBytes(video.size) && <Row label={t("gallery.details.size")}>{fmtBytes(video.size)}</Row>}
          {resolution && <Row label={t("gallery.details.resolution")}>{resolution}</Row>}
          {fps && <Row label={t("gallery.details.fps")}>{fps}</Row>}
          {details?.video_codec && <Row label={t("gallery.details.videoCodec")}>{details.video_codec.toUpperCase()}</Row>}
          {bitrateLabel && <Row label={t("gallery.details.bitrate")}>{bitrateLabel}</Row>}
          {audioLabel && <Row label={t("gallery.details.audio")}>{audioLabel}</Row>}
          {details?.container && <Row label={t("gallery.details.container")}>{details.container}</Row>}
          <Row label={t("gallery.details.location")}>
            {video.drive_only ? t("gallery.details.locationDriveOnly")
              : video.drive_synced ? t("gallery.details.locationBoth")
              : t("gallery.details.locationLocalOnly")}
          </Row>
          {cardTags.length > 0 && (
            <Row label={t("gallery.details.tags")}>
              <span className="flex flex-wrap justify-end gap-1">
                {cardTags.map((tg) => (
                  <span key={tg.id} className="flex items-center gap-1 rounded-full bg-stone-800 px-2 py-0.5 text-[11px]">
                    <span className="h-1.5 w-1.5 shrink-0 rounded-full" style={{ backgroundColor: tg.color }} />
                    {tg.name}
                  </span>
                ))}
              </span>
            </Row>
          )}
          <Row label={t("gallery.details.path")}>
            <span className="break-all text-[11px] text-stone-400">{video.local_path}</span>
          </Row>
        </div>
      </div>
    </div>
  );
}
