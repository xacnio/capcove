import { Fragment, useEffect, useState } from "react";
import { MdClose } from "react-icons/md";
import { invoke } from "../lib/tauri.js";
import { useAppCover, useAppIcon } from "../gallery/appIcons.js";
import { relativeTime } from "../lib/relativeTime.js";

// Heavier ffprobe details (codec/fps/bitrate) than the per-card duration
// probe — only fetched here, when the header actually mounts for a given
// video, and cached per name+mtime so reopening the same video in one
// session doesn't re-probe it.
const detailsCache = new Map();

function fmtBytes(bytes) {
  if (!bytes) return "";
  const units = ["B", "KB", "MB", "GB"];
  let v = bytes, i = 0;
  while (v >= 1024 && i < units.length - 1) { v /= 1024; i++; }
  return `${v.toFixed(i === 0 ? 0 : 1)} ${units[i]}`;
}

function fmtDuration(totalSecs) {
  if (totalSecs == null || !Number.isFinite(totalSecs)) return null;
  const s = Math.max(0, Math.floor(totalSecs));
  const m = Math.floor(s / 60);
  const h = Math.floor(m / 60);
  const mm = h > 0 ? String(m % 60).padStart(2, "0") : String(m);
  const ss = String(s % 60).padStart(2, "0");
  return h > 0 ? `${h}:${mm}:${ss}` : `${mm}:${ss}`;
}

// Same rule as VideoGrid's own `displayTitle` (kept duplicated rather than
// exported): real files show their filename, never the stale/auto-captured
// `video.title` — only YouTube-live's synthetic-name virtual entry needs it.
function displayTitle(video) {
  if (video.kind === "youtube_live") return video.title || video.name;
  return video.name.split(/[\\/]/).pop() || video.name;
}

// "Today"/"Yesterday" plus clock time ("Today, 12:33") for the last two
// calendar days; the plain relative string ("3 days ago") isn't specific
// enough beyond that, so anything older shows its actual date+time instead.
// The exact full date+time is always one hover away via the `title=` tooltip.
function fmtWhen(ms, lang) {
  if (!ms) return "";
  const rel = relativeTime(ms, lang);
  const isCalendarDay = rel === (lang === "tr" ? "Bugün" : "Today") || rel === (lang === "tr" ? "Dün" : "Yesterday");
  if (isCalendarDay) {
    const clock = new Intl.DateTimeFormat(lang === "tr" ? "tr-TR" : "en-US", { hour: "2-digit", minute: "2-digit" }).format(new Date(ms));
    return `${rel}, ${clock}`;
  }
  return new Intl.DateTimeFormat(lang === "tr" ? "tr-TR" : "en-US", { dateStyle: "medium", timeStyle: "short" }).format(new Date(ms));
}

function fmtFullDate(ms, lang) {
  if (!ms) return "";
  return new Intl.DateTimeFormat(lang === "tr" ? "tr-TR" : "en-US", { dateStyle: "full", timeStyle: "medium" }).format(new Date(ms));
}

// Sits above the video in the player modal: game cover art as a fading
// backdrop, the video's title, and its metadata (game, date, size,
// duration, tags) as a small info row. Display-only, nothing editable.
export default function PlayerInfoHeader({ video, tags, t, lang, onClose }) {
  const cover = useAppCover(video.app);
  const icon = useAppIcon(video.app);
  const art = cover || icon;

  // Drive-only cards already carry width/height (fetched from Drive at list
  // time) — no local file to ffprobe. Local files get the fuller probe
  // (codec/fps/bitrate) here on demand.
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
  const fps = details?.fps ? `${Math.round(details.fps * 100) / 100}fps` : null;
  const videoCodec = details?.video_codec ? details.video_codec.toUpperCase() : null;
  const bitrate = details?.video_bitrate_kbps ?? details?.overall_bitrate_kbps;
  const bitrateLabel = bitrate ? (bitrate >= 1000 ? `${(bitrate / 1000).toFixed(1)} Mbps` : `${bitrate} Kbps`) : null;
  const audioLabel = details?.audio_codec
    ? [details.audio_codec.toUpperCase(), details.audio_channels === 1 ? "Mono" : details.audio_channels === 2 ? "Stereo" : details.audio_channels ? `${details.audio_channels}ch` : null].filter(Boolean).join(" ")
    : null;
  const extraBadges = [resolution, videoCodec, fps, bitrateLabel, audioLabel].filter(Boolean);
  const resolvedTags = (video.tags ?? []).map((id) => tags?.find((tg) => tg.id === id)).filter(Boolean);

  return (
    <div className="relative shrink-0 overflow-hidden bg-stone-950">
      {art && (
        <>
          <img src={art} alt="" className="absolute inset-0 h-full w-full object-cover" />
          <div className="absolute inset-0 bg-gradient-to-t from-stone-950 via-stone-950/80 to-stone-950/40" />
        </>
      )}
      <div className="relative z-10 flex items-start justify-between gap-3 px-5 pb-3.5 pt-4">
        <div className="min-w-0">
          <div className="truncate text-base font-semibold text-stone-100">{displayTitle(video)}</div>
          <div className="mt-1.5 flex flex-wrap items-center gap-x-2 gap-y-1 text-[11px] text-stone-400">
            {video.app && (
              <span className="flex items-center gap-1.5 truncate">
                {icon && <img src={icon} alt="" className="h-4 w-4 shrink-0 rounded object-cover" />}
                <span className="truncate font-medium text-stone-300">{video.app}</span>
              </span>
            )}
            {video.app && <span className="text-stone-600">•</span>}
            {video.modified != null && (
              <span className="whitespace-nowrap" title={fmtFullDate(video.modified, lang)}>{fmtWhen(video.modified, lang)}</span>
            )}
            {video.size != null && <span className="text-stone-600">•</span>}
            {video.size != null && <span className="whitespace-nowrap">{fmtBytes(video.size)}</span>}
            {fmtDuration(video.duration_secs) && <span className="text-stone-600">•</span>}
            {fmtDuration(video.duration_secs) && <span className="whitespace-nowrap">{fmtDuration(video.duration_secs)}</span>}
            {extraBadges.map((b) => (
              <Fragment key={b}>
                <span className="text-stone-600">•</span>
                <span className="whitespace-nowrap">{b}</span>
              </Fragment>
            ))}
            {resolvedTags.map((tg) => (
              <Fragment key={tg.id}>
                <span className="text-stone-600">•</span>
                <span className="flex items-center gap-1 whitespace-nowrap">
                  <span className="h-1.5 w-1.5 shrink-0 rounded-full" style={{ backgroundColor: tg.color }} />
                  {tg.name}
                </span>
              </Fragment>
            ))}
          </div>
        </div>
        <button
          onClick={onClose}
          title={t ? t("common.close") : "Close"}
          className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-stone-900/70 text-stone-300 transition hover:bg-stone-800 hover:text-white"
        >
          <MdClose size={18} />
        </button>
      </div>
    </div>
  );
}
