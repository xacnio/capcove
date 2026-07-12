import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useVirtualizer } from "@tanstack/react-virtual";
import { invoke, convertFileSrc, listen } from "../lib/tauri.js";
import * as Icon from "./icons.jsx";
import { SiYoutube } from "react-icons/si";
import VideoPlayer from "../components/VideoPlayer.jsx";
import LiveRecordingPlayer from "../components/LiveRecordingPlayer.jsx";
import PlayerInfoHeader from "../components/PlayerInfoHeader.jsx";
import CardMenu from "./CardMenu.jsx";
import BulkMenu from "./BulkMenu.jsx";
import YouTubeUploadModal from "../components/YouTubeUploadModal.jsx";
import VideoDetailsModal from "./VideoDetailsModal.jsx";
import { relativeTime } from "../lib/relativeTime.js";
import { useAppIcon } from "./appIcons.js";

// Held back from measured width so column count assumes a scrollbar is
// always present, keeping layout consistent whether or not one's showing.
const SCROLLBAR_RESERVE = 16;

// Full timeline editor is temporarily off — the player's own trim tool
// covers quick clips for now. Flip this back on when it's ready.
const EDITOR_ENABLED = false;

function tagsFor(video, tags) {
  if (!video.tags?.length) return [];
  return video.tags.map((id) => tags.find((tg) => tg.id === id)).filter(Boolean);
}

// Small colored-dot cluster showing which tags a card is assigned to — this
// is the only place assigned tags show up outside the menu that sets them.
function TagDots({ cardTags, max = 4 }) {
  if (cardTags.length === 0) return null;
  const shown = cardTags.slice(0, max);
  const extra = cardTags.length - shown.length;
  return (
    <span className="flex shrink-0 items-center gap-1" title={cardTags.map((tg) => tg.name).join(", ")}>
      {shown.map((tg) => (
        <span key={tg.id} className="h-2 w-2 shrink-0 rounded-full" style={{ backgroundColor: tg.color }} />
      ))}
      {extra > 0 && <span className="text-[9px] font-medium text-stone-500">+{extra}</span>}
    </span>
  );
}


function fmtDuration(secs) {
  if (!secs || !Number.isFinite(secs)) return "";
  const total = Math.round(secs);
  const m = Math.floor(total / 60);
  const s = total % 60;
  return `${m}:${s.toString().padStart(2, "0")}`;
}

function fmtBytes(bytes) {
  if (!bytes) return "";
  const units = ["B", "KB", "MB", "GB"];
  let v = bytes, i = 0;
  while (v >= 1024 && i < units.length - 1) { v /= 1024; i++; }
  return `${v.toFixed(i === 0 ? 0 : 1)} ${units[i]}`;
}

// The relative date ("Today"/"3 days ago") plus the exact clock time — e.g.
// "Today, 12:33" — so the meta line carries when in the day it happened, not
// just which day.
function fmtWhen(ms, lang) {
  if (!ms) return "";
  const rel = relativeTime(ms, lang);
  const clock = new Intl.DateTimeFormat(lang === "tr" ? "tr-TR" : "en-US", { hour: "2-digit", minute: "2-digit" }).format(new Date(ms));
  return rel ? `${rel}, ${clock}` : clock;
}

// The full, unambiguous date+time — shown as the `title=` tooltip on the
// relative "Today, 12:33" text so the exact moment is one hover away.
function fmtFullDate(ms, lang) {
  if (!ms) return "";
  return new Intl.DateTimeFormat(lang === "tr" ? "tr-TR" : "en-US", { dateStyle: "full", timeStyle: "medium" }).format(new Date(ms));
}

function fileNameFromPath(p) {
  return (p.split(/[\\/]/).pop() || p).replace(/\.[^.]+$/, "");
}

// `onCopied` is VideoGrid's own in-gallery toast, not the overlay toast.
function copyDriveLink(video, onCopied) {
  const url = `https://drive.google.com/file/d/${video.drive_id}/view`;
  Promise.resolve(invoke("copy_text", { text: url }).catch(() => navigator.clipboard?.writeText(url)))
    .then(onCopied);
}

// Opens a URL via the OS opener (browser-window fallback).
function openUrl(url) {
  invoke("open_url", { url }).catch(() => window.open(url, "_blank"));
}

// Drive respects `authuser=<email>`: the browser's *default* Google account is
// often a different one, which would open a wrong-account/no-access page;
// passing the connected account's email makes Google pick the right one (when
// it's signed in). Harmless when `email` is missing.
const openDriveFile = (driveId, email) =>
  openUrl(`https://drive.google.com/file/d/${driveId}/view${email ? `?authuser=${encodeURIComponent(email)}` : ""}`);

const openYoutubeVideo = (id) => openUrl(`https://youtube.com/watch?v=${id}`);

// Routes through YouTube's `channel_switcher` (authuser=email) so the right
// managing account/channel is active before landing on the Studio page.
const openStudioTab = (id, tab, email) => {
  const studio = `https://studio.youtube.com/video/${id}/${tab}`;
  openUrl(
    email
      ? `https://www.youtube.com/channel_switcher?next=${encodeURIComponent(studio)}&authuser=${encodeURIComponent(email)}`
      : studio
  );
};
const openYoutubeStudio = (id, email) => openStudioTab(id, "edit", email);
const openYoutubeClip = (id, email) => openStudioTab(id, "clips", email);

// `video.name` is a path relative to the recordings root, not a display
// title; real files show their filename, never `video.title` (no rename
// feature). YouTube-live's synthetic entry is the exception — its `name`
// has no real filename, so it needs `video.title` instead.
function displayTitle(video) {
  if (video.kind === "youtube_live") return video.title || video.name;
  return video.name.split(/[\\/]/).pop() || video.name;
}

// Hover-preview playback speed, scaled to clip length (see `useHoverPreview`)
// so a short clip plays at 1x while a long recording speeds through.
function previewRateForDuration(seconds) {
  if (!seconds || !Number.isFinite(seconds)) return 1;
  if (seconds <= 20) return 1;
  if (seconds <= 60) return 2;
  if (seconds <= 300) return 3;
  return 4;
}

// Converts the kebab button's rect into the {x,y} top-left CardMenu expects
// (same shape as a right-click's cursor position), right-aligned to itself.
const CARD_MENU_WIDTH = 224;
function kebabMenuPos(rect) {
  return { x: Math.max(8, rect.right - CARD_MENU_WIDTH), y: rect.bottom + 4 };
}

// Session-wide caches so cards render instantly on remount instead of
// re-fetching thumbnails/durations over IPC. Keyed by name+mtime.
const thumbCache = new Map();
const durationCache = new Map();
const pendingFetch = new Map();

function fetchCached(cache, key, fetcher) {
  if (cache.has(key)) return Promise.resolve(cache.get(key));
  let p = pendingFetch.get(key);
  if (!p) {
    p = fetcher()
      .then((v) => { cache.set(key, v); return v; })
      .catch(() => null)
      .finally(() => pendingFetch.delete(key));
    pendingFetch.set(key, p);
  }
  return p;
}

// Drive-only cards have no local file to probe, so their thumbnail comes
// from Drive's own `thumbnailLink` — which can take a while to appear (Drive
// generates it asynchronously after upload) or never appear at all for some
// codecs. Unlike `fetchCached`, a failed attempt is never cached: only the
// per-card retry loop in `useVideoMedia` paces re-attempts.
const driveThumbCache = new Map();
const driveThumbPending = new Map();
const DRIVE_THUMB_RETRY_MS = 5 * 60 * 1000;

function fetchDriveThumbnail(driveId) {
  if (driveThumbCache.has(driveId)) return Promise.resolve(driveThumbCache.get(driveId));
  let p = driveThumbPending.get(driveId);
  if (!p) {
    p = invoke("read_drive_video_thumbnail", { driveId })
      .then((b64) => {
        const v = `data:image/jpeg;base64,${b64}`;
        driveThumbCache.set(driveId, v);
        return v;
      })
      .catch(() => null)
      .finally(() => driveThumbPending.delete(driveId));
    driveThumbPending.set(driveId, p);
  }
  return p;
}

// Shared by VideoCard and VideoListRow so the two size/density variants of
// the same row can never drift on what a "drive-only"/"YouTube-hosted"
// card actually looks like or how its thumbnail gets fetched.
function useVideoMedia(video) {
  // Two "no local file" kinds: a still-running/finished YouTube live session,
  // or an uploaded recording whose local copy was deleted (kept as a
  // link-only card). Both source their thumbnail from YouTube.
  const isLiveKind = video.kind === "youtube_live";
  const isYoutubeOnly = video.kind === "youtube_only";
  const hostedOnYoutube = isLiveKind || isYoutubeOnly;
  // A local recording still being written (see `App.jsx`'s `inProgressEntry`)
  // has no thumbnail/duration probe yet; duration is wall-clock time ticked locally.
  const isRecordingInProgress = video.kind === "recording";
  // Frozen snapshot shown between a recording stopping and the finished file
  // reload landing (see `VideoGrid`'s `finishingEntry`).
  const isRecordingFinishing = video.kind === "recording_finishing";
  const isRecordingCard = isRecordingInProgress || isRecordingFinishing;
  // Backed up to Drive but not on this machine — clicking downloads first, then plays.
  const driveOnly = !!video.drive_only;
  const ytId = video.youtube_video_id;
  const cacheKey = `${video.name}:${video.modified ?? 0}`;
  const [thumb, setThumb] = useState(() => {
    if (hostedOnYoutube || isRecordingCard) return null;
    if (driveOnly) return (video.drive_id && driveThumbCache.get(video.drive_id)) ?? null;
    return thumbCache.get(`t:${cacheKey}`) ?? null;
  });
  const [meta, setMeta] = useState(() => durationCache.get(`d:${cacheKey}`) ?? null);
  const duration = meta?.duration_secs ?? null;
  const resolution = driveOnly
    ? (video.width && video.height ? { width: video.width, height: video.height } : null)
    : (meta?.width && meta?.height ? { width: meta.width, height: meta.height } : null);
  const [liveInfo, setLiveInfo] = useState(null); // {title,live,duration_secs,viewers,thumbnail}

  // YouTube-hosted entries: status + thumbnail come from the API since
  // YouTube generates the real thumbnail asynchronously. Polls until it
  // appears (or live status resolves), capped at `MAX_THUMB_ATTEMPTS`.
  useEffect(() => {
    if (!hostedOnYoutube || !ytId) return;
    let cancelled = false;
    let timer;
    let attempts = 0;
    const MAX_THUMB_ATTEMPTS = 20; // ~5 minutes at 15s apart
    const load = async () => {
      try {
        const info = await invoke("get_youtube_live_info", { videoId: ytId });
        if (cancelled) return;
        attempts++;
        setLiveInfo({ ...info, fetchedAt: Date.now() });
        if (info.thumbnail) setThumb(info.thumbnail);
        const stillLive = isLiveKind && (info.live || info.duration_secs == null);
        if ((stillLive || !info.thumbnail) && attempts < MAX_THUMB_ATTEMPTS) timer = setTimeout(load, 15000);
      } catch {
        if (!cancelled) setThumb(`https://i.ytimg.com/vi/${ytId}/mqdefault.jpg`);
      }
    };
    load();
    return () => { cancelled = true; clearTimeout(timer); };
  }, [hostedOnYoutube, isLiveKind, ytId]);

  // Tick the live counter locally between API refreshes, and the in-progress
  // recording's own wall-clock counter (both just `Date.now()`).
  const [, setLiveTick] = useState(0);
  useEffect(() => {
    if (!liveInfo?.live && !isRecordingInProgress) return;
    const id = setInterval(() => setLiveTick((t) => t + 1), 1000);
    return () => clearInterval(id);
  }, [liveInfo?.live, isRecordingInProgress]);
  const liveSecs = liveInfo?.live && liveInfo.duration_secs != null
    ? liveInfo.duration_secs + Math.max(0, Math.floor((Date.now() - liveInfo.fetchedAt) / 1000))
    : liveInfo?.duration_secs;
  const recSecs = isRecordingInProgress && video.recordingStartedAt != null
    ? Math.max(0, Math.floor(Date.now() / 1000) - video.recordingStartedAt)
    : isRecordingFinishing
      ? video.frozenSecs ?? null
      : null;

  // Live size/thumbnail from the writer thread's periodic reports (not a probe of the in-progress file).
  const [liveSize, setLiveSize] = useState(null);
  useEffect(() => {
    if (!isRecordingCard) { setLiveSize(null); return; }
    if (isRecordingFinishing) return;
    let unlisten;
    listen("recording-stats", (e) => {
      if (e.payload?.total_bytes != null) setLiveSize(e.payload.total_bytes);
    }).then((u) => { unlisten = u; });
    return () => unlisten?.();
  }, [isRecordingCard, isRecordingFinishing]);
  useEffect(() => {
    if (!isRecordingInProgress) return;
    let unlisten;
    listen("recording-thumbnail", (e) => {
      if (e.payload?.data_url) setThumb(e.payload.data_url);
    }).then((u) => { unlisten = u; });
    return () => unlisten?.();
  }, [isRecordingInProgress]);

  useEffect(() => {
    if (hostedOnYoutube || isRecordingCard) return;
    let cancelled = false;
    if (driveOnly) {
      // No local file to probe — Drive generates its own preview thumbnail;
      // duration falls back to `video.duration_secs` from metadata, if any.
      // Drive's thumbnail can take a while to appear (or never, for some
      // codecs), so keep retrying at an interval instead of giving up for
      // the rest of the session on the first miss.
      if (!video.drive_id) return () => { cancelled = true; };
      let retryTimer;
      const attempt = () => {
        fetchDriveThumbnail(video.drive_id).then((v) => {
          if (cancelled) return;
          if (v) setThumb(v);
          else retryTimer = setTimeout(attempt, DRIVE_THUMB_RETRY_MS);
        });
      };
      attempt();
      return () => { cancelled = true; clearTimeout(retryTimer); };
    }
    fetchCached(thumbCache, `t:${cacheKey}`, () =>
      invoke("read_video_thumbnail", { name: video.name }).then((b64) => `data:image/jpeg;base64,${b64}`)
    ).then((v) => { if (!cancelled && v) setThumb(v); });
    fetchCached(durationCache, `d:${cacheKey}`, () =>
      invoke("get_video_metadata", { name: video.name })
    ).then((m) => { if (!cancelled && m) setMeta(m); });
    return () => { cancelled = true; };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [cacheKey, hostedOnYoutube, isRecordingInProgress, driveOnly, video.drive_id, video.name]);

  return { isLiveKind, isYoutubeOnly, isRecordingInProgress, isRecordingFinishing, isRecordingCard, hostedOnYoutube, driveOnly, thumb, setThumb, duration, resolution, liveInfo, liveSecs, recSecs, liveSize };
}

// Drive-only: opening it now opens the player directly — it shows its own
// download-then-play placeholder (see `DriveOnlyPlaceholder`) instead of
// silently downloading in the background with nothing to show for it until
// a second click. `downloadToDevice` is the separate, explicit "just fetch
// it, don't open anything" action (the hover icon, the menu item).
function useVideoOpenOrPlay(video, hostedOnYoutube, driveOnly, onPlay) {
  const [downloading, setDownloading] = useState(false);
  const openOrPlay = () => {
    if (hostedOnYoutube) { invoke("open_url", { url: video.local_path }).catch(() => window.open(video.local_path, "_blank")); return; }
    onPlay(video);
  };
  const downloadToDevice = async () => {
    if (downloading) return;
    setDownloading(true);
    try {
      await invoke("download_video_from_drive", { driveId: video.drive_id, name: video.name, modifiedMs: video.modified });
    } catch { /* best-effort, matches this file's other invoke() calls */ }
    finally { setDownloading(false); }
  };
  return { openOrPlay, downloadToDevice, downloading };
}

// Hover-preview: after a short intent delay, plays the (local-only) file muted/looping,
// scaled to its length. Unmounted on mouse-leave so only one card decodes at a time.
const HOVER_PREVIEW_DELAY_MS = 250;
function useHoverPreview(eligible) {
  const [active, setActive] = useState(false);
  const [ready, setReady] = useState(false);
  const videoRef = useRef(null);
  const timerRef = useRef(null);

  const onMouseEnter = () => {
    if (!eligible) return;
    clearTimeout(timerRef.current);
    timerRef.current = setTimeout(() => setActive(true), HOVER_PREVIEW_DELAY_MS);
  };
  const onMouseLeave = () => {
    clearTimeout(timerRef.current);
    setActive(false);
    setReady(false);
  };
  useEffect(() => () => clearTimeout(timerRef.current), []);

  return { active, ready, setReady, videoRef, onMouseEnter, onMouseLeave };
}

// No byte-level progress exists for a delete; `deleting` just covers the
// card while the promise is in flight so a slow Drive round trip isn't silent.
// `permanent` comes from whether the menu that's about to act was opened
// with Shift held (see `openMenu`) — bypasses the Recycle Bin for this call only.
function useVideoDelete(video, onDelete, onDeleteDriveCopy, onDeleteBoth, permanent, closeMenu) {
  const [deleting, setDeleting] = useState(false);
  const wrap = (action) => async () => {
    closeMenu?.();
    setDeleting(true);
    try { await action(); } finally { setDeleting(false); }
  };
  const handleDeleteClick = wrap(() => onDelete(video, permanent));
  const handleDeleteDriveCopyClick = wrap(() => onDeleteDriveCopy(video));
  const handleDeleteBothClick = wrap(() => onDeleteBoth(video, permanent));
  return { deleting, handleDeleteClick, handleDeleteDriveCopyClick, handleDeleteBothClick };
}

// Player modal's stand-in for a `drive_only` card — there's no local file to
// hand `<VideoPlayer>` yet. Pressing play downloads it (progress from the
// same shared `sync-transfers-changed` feed the card thumbnails use), then
// `onDownloaded` flips `playing` over to a normal local video so the modal
// switches straight to the real player and starts it.
function DriveOnlyPlaceholder({ video, t, transfer, onDownloaded }) {
  const [thumb, setThumb] = useState(() => (video.drive_id && driveThumbCache.get(video.drive_id)) ?? null);
  const [downloading, setDownloading] = useState(false);
  const [error, setError] = useState("");

  useEffect(() => {
    if (thumb || !video.drive_id) return undefined;
    let cancelled = false;
    fetchDriveThumbnail(video.drive_id).then((v) => { if (!cancelled && v) setThumb(v); });
    return () => { cancelled = true; };
  }, [video.drive_id, thumb]);

  const startDownload = async () => {
    setDownloading(true);
    setError("");
    try {
      const localPath = await invoke("download_video_from_drive", { driveId: video.drive_id, name: video.name, modifiedMs: video.modified });
      onDownloaded(localPath);
    } catch (e) {
      setError(String(e));
    } finally {
      setDownloading(false);
    }
  };

  const pct = transfer?.total > 0 ? Math.min(100, Math.round((transfer.sent / transfer.total) * 100)) : null;

  return (
    // `flex-1 min-h-0` (not just `h-full`) — its siblings in the modal
    // (`VideoPlayer`, `LiveRecordingPlayer`) get this from the caller, and
    // without it this box collapses to its own content size instead of
    // filling what's left below the header, which also shrinks its actual
    // clickable area away from where the button visually paints.
    <div className="relative flex h-full w-full flex-1 min-h-0 flex-col items-center justify-center gap-4 overflow-hidden bg-black p-8 text-center">
      {thumb && (
        <>
          {/* Blurred, darkened backdrop of the same thumbnail — `scale-110`
              keeps the blur's soft edges pushed past the container's own
              edges, so no lighter fringe shows around the border. */}
          <img src={thumb} alt="" className="absolute inset-0 h-full w-full scale-110 object-cover blur-2xl" />
          <div className="absolute inset-0 bg-black/65" />
        </>
      )}
      <div className="relative z-10 flex flex-col items-center gap-4">
        {thumb
          ? <img src={thumb} alt="" className="max-h-[45%] max-w-[70%] rounded-lg object-contain shadow-2xl" />
          : <Icon.Cloud size={56} className="text-stone-700" />}
        <div className="max-w-md truncate text-sm text-stone-400">{video.name}</div>
        {(video.duration_secs != null || (video.width && video.height) || video.size != null) && (
          <div className="flex flex-wrap items-center justify-center gap-x-1.5 gap-y-0.5 text-[11px] text-stone-500">
            {video.duration_secs != null && <span className="whitespace-nowrap">{fmtDuration(video.duration_secs)}</span>}
            {video.duration_secs != null && (video.width && video.height || video.size != null) && <span className="text-stone-700">•</span>}
            {video.width && video.height && <span className="whitespace-nowrap">{video.width}×{video.height}</span>}
            {video.width && video.height && video.size != null && <span className="text-stone-700">•</span>}
            {video.size != null && <span className="whitespace-nowrap">{fmtBytes(video.size)}</span>}
          </div>
        )}
        {downloading ? (
          <div className="flex w-64 flex-col items-center gap-2">
            <div className="h-1.5 w-full overflow-hidden rounded-full bg-white/10">
              <div className="h-full rounded-full bg-accent-500 transition-[width]" style={{ width: `${pct ?? 0}%` }} />
            </div>
            <div className="text-xs text-stone-500">{t("gallery.player.downloading")}{pct != null ? ` ${pct}%` : ""}</div>
          </div>
        ) : (
          <button
            onClick={startDownload}
            title={t("gallery.player.downloadAndPlay")}
            className="flex h-14 w-14 items-center justify-center rounded-full bg-white/90 text-stone-950 transition hover:scale-105 hover:bg-white"
          >
            <Icon.Play size={22} />
          </button>
        )}
        {error && <div className="max-w-md text-xs text-red-400">{error}</div>}
      </div>
    </div>
  );
}

function VideoCard({ video, t, lang, tags, onPlay, onDelete, onDeleteDriveCopy, onDeleteBoth, onToggleTag, onToggleFavorite, onUploadYoutube, onUploadDrive, transfer, driveConnected, driveEmail, youtubeChannelId, youtubeEmail, onEdit, viewMode, isSelected, hasSelection, bulkDeleting, onToggleSelect, onSelectOnly, onShowInFolderView, onShowDetails, onCopyDriveLink, onBulkContextMenu, highlighted }) {
  // Live upload progress for this card, from the shared `sync-transfers-changed`
  // feed (undefined once upload finishes and `drive_synced` takes over).
  const uploading = transfer?.status === "uploading" || transfer?.status === "queued";
  const uploadPct = transfer?.total > 0 ? Math.min(100, Math.round((transfer.sent / transfer.total) * 100)) : null;
  const { isLiveKind, isYoutubeOnly, isRecordingFinishing, isRecordingCard, hostedOnYoutube, driveOnly, thumb, setThumb, duration, resolution, liveInfo, liveSecs, recSecs, liveSize } = useVideoMedia(video);
  const [menuOpen, setMenuOpen] = useState(false);
  const [menuPos, setMenuPos] = useState(null); // {x,y} — cursor for right-click, kebab button's own rect otherwise
  const [menuPermanent, setMenuPermanent] = useState(false); // menu opened with Shift held — bypass Recycle Bin
  const { openOrPlay, downloadToDevice, downloading } = useVideoOpenOrPlay(video, hostedOnYoutube, driveOnly, onPlay);
  const appIcon = useAppIcon(video.app);
  const previewEligible = !hostedOnYoutube && !driveOnly && !isRecordingCard;
  const { active: previewActive, ready: previewReady, setReady: setPreviewReady, videoRef: previewRef, onMouseEnter: onPreviewEnter, onMouseLeave: onPreviewLeave } = useHoverPreview(previewEligible);
  const cardTags = tagsFor(video, tags);

  const openMenu = (pos, permanent = false) => { setMenuPos(pos); setMenuPermanent(permanent); setMenuOpen(true); };
  const closeMenu = () => { setMenuOpen(false); setMenuPos(null); };
  const { deleting: ownDeleting, handleDeleteClick, handleDeleteDriveCopyClick, handleDeleteBothClick } = useVideoDelete(video, onDelete, onDeleteDriveCopy, onDeleteBoth, menuPermanent, closeMenu);
  // Own single-item delete in flight, or this card's swept up in a bulk
  // "Delete" — either way it should read as "going away" the same way.
  const deleting = ownDeleting || bulkDeleting;
  // "large" and narrower reserve too little width for extra inline action
  // buttons next to the title; below "xl" only the kebab renders inline.
  const richActions = !viewMode || viewMode === "xl" || viewMode === "2xl";

  // A recording still being written has no menu/favorite/upload/tag actions
  // or hover preview, but does open in the real player on click. Hooks above
  // all ran identically regardless, so branching here doesn't change hook order.
  if (isRecordingCard) {
    return (
      <div className="group relative cursor-pointer rounded-xl bg-stone-900 overflow-hidden transition hover:bg-stone-800/80"
        onClick={() => openOrPlay()}>
        <div className="relative aspect-video flex items-center justify-center bg-stone-950 overflow-hidden">
          {thumb
            ? <img src={thumb} alt="" className="h-full w-full object-cover" />
            : appIcon
              ? <img src={appIcon} alt="" className="h-12 w-12 rounded-lg object-cover opacity-80" />
              : <Icon.Monitor size={32} className="text-stone-700" />}
          <div className="absolute inset-0 flex items-center justify-center bg-black/0 transition group-hover:bg-black/25">
            <div className="flex h-11 w-11 items-center justify-center rounded-full bg-white/90 opacity-0 transition group-hover:opacity-100">
              <div className="ml-0.5 h-0 w-0 border-y-[7px] border-y-transparent border-l-[12px] border-l-stone-900" />
            </div>
          </div>
          <span className={`absolute left-2 top-2 flex items-center gap-1 rounded px-1.5 py-0.5 text-[9px] font-bold uppercase tracking-wider text-white ${isRecordingFinishing ? "bg-stone-700/90" : "bg-red-600/90"}`}>
            <span className="relative flex h-1.5 w-1.5">
              {!isRecordingFinishing && <span className="absolute inline-flex h-full w-full animate-ping rounded-full bg-white opacity-70" />}
              <span className="relative inline-flex h-1.5 w-1.5 rounded-full bg-white" />
            </span>
            {isRecordingFinishing ? t("gallery.card.recordingFinishing") : t("gallery.card.recordingInProgress")}
          </span>
          <span className="absolute bottom-2 right-2 rounded bg-black/75 px-1.5 py-0.5 text-[11px] font-semibold tabular-nums text-white">
            {fmtDuration(recSecs)}
          </span>
        </div>
        <div className="vc-info flex h-[76px] items-start gap-2.5 px-3 pt-2.5 pb-2">
          {appIcon && <img src={appIcon} alt="" className="mt-0.5 h-9 w-9 shrink-0 rounded-lg object-cover" />}
          <div className="min-w-0">
            <div className="truncate text-sm font-semibold text-stone-100">{displayTitle(video)}</div>
            <div className="vc-meta mt-1 flex flex-wrap items-center gap-x-1.5 gap-y-0.5 text-[11px] text-stone-500">
              {video.app && <span className="max-w-[90px] truncate" title={video.app}>{video.app}</span>}
              {video.app && liveSize != null && <span className="text-stone-700">•</span>}
              {liveSize != null && <span className="whitespace-nowrap">{fmtBytes(liveSize)}</span>}
            </div>
          </div>
        </div>
      </div>
    );
  }

  // Ctrl/Cmd-click toggles this card into the selection without opening it;
  // a plain click while something is selected narrows to just this card.
  const handleThumbClick = (e) => {
    if (deleting) return;
    if (e.ctrlKey || e.metaKey) { e.preventDefault(); onToggleSelect(video.name); return; }
    if (hasSelection) { onSelectOnly(video.name); return; }
    openOrPlay();
  };

  return (
    <div data-vm={viewMode} data-card-name={video.name}
      className={`group relative cursor-pointer rounded-xl bg-stone-900 overflow-hidden transition hover:bg-stone-800/80 animate-fade-in ${deleting ? "pointer-events-none opacity-70" : ""} ${isSelected ? "ring-2 ring-accent-400 bg-accent-500/10" : ""} ${highlighted ? "animate-highlight-flash" : ""}`}
      onClick={handleThumbClick}
      onContextMenu={(e) => {
        e.preventDefault();
        // Part of a 2+ selection → bulk menu; otherwise this card's own menu.
        if (onBulkContextMenu) { onBulkContextMenu(e.clientX, e.clientY); return; }
        openMenu({ x: e.clientX, y: e.clientY }, e.shiftKey);
      }}
      onMouseEnter={onPreviewEnter} onMouseLeave={onPreviewLeave}>
      <div className="relative aspect-video bg-stone-950">
        <div
          className={`absolute left-2 top-2 z-10 flex h-5 w-5 items-center justify-center rounded border shadow-md backdrop-blur-[2px] transition ${
            isSelected ? "border-accent-400 bg-accent-400 text-stone-950" : "border-stone-500 bg-stone-950/60 text-transparent opacity-0 group-hover:opacity-100"
          }`}
          onClick={(e) => { e.stopPropagation(); onToggleSelect(video.name); }}
        >
          <Icon.Check size={11} className="stroke-[3.5]" />
        </div>
        {thumb ? (
          <img src={thumb} alt={video.name} className={`h-full w-full object-cover ${driveOnly ? "opacity-60" : ""}`}
            onError={() => setThumb(null)} referrerPolicy="no-referrer" />
        ) : (
          <div className="flex h-full w-full items-center justify-center">
            {hostedOnYoutube ? <SiYoutube size={34} className="text-red-600/70" />
              : driveOnly ? <Icon.Cloud size={32} className="text-stone-700" />
              : <Icon.Monitor size={32} className="text-stone-700" />}
          </div>
        )}
        {/* Hover preview (see `useHoverPreview`). Crossfades in once the
            first frame decodes, instead of popping in over a black gap. */}
        {previewActive && previewEligible && (
          <video
            ref={previewRef}
            src={convertFileSrc(video.local_path)}
            muted
            loop
            autoPlay
            playsInline
            className={`absolute inset-0 h-full w-full object-cover transition-opacity duration-200 ${previewReady ? "opacity-100" : "opacity-0"}`}
            onLoadedMetadata={(e) => { e.currentTarget.playbackRate = previewRateForDuration(e.currentTarget.duration); }}
            onPlaying={() => setPreviewReady(true)}
          />
        )}
        {driveOnly ? (
          <div className="absolute inset-0 flex items-center justify-center bg-black/35">
            <div className="flex h-11 w-11 items-center justify-center rounded-full bg-white/90">
              {downloading
                ? <div className="h-4 w-4 animate-spin rounded-full border-2 border-stone-400 border-t-stone-900" />
                : <Icon.Cloud size={18} className="text-stone-900" />}
            </div>
          </div>
        ) : (
          <div className="absolute inset-0 flex items-center justify-center bg-black/0 transition group-hover:bg-black/25">
            <div className="flex h-11 w-11 items-center justify-center rounded-full bg-white/90 opacity-0 transition group-hover:opacity-100">
              <div className="ml-0.5 h-0 w-0 border-y-[7px] border-y-transparent border-l-[12px] border-l-stone-900" />
            </div>
          </div>
        )}
        {isLiveKind ? (
          liveInfo?.live ? (
            <span className="absolute bottom-2 right-2 flex items-center gap-1.5 rounded bg-red-600/90 px-1.5 py-0.5 text-[11px] font-bold text-white">
              <span className="relative flex h-1.5 w-1.5">
                <span className="absolute inline-flex h-full w-full animate-ping rounded-full bg-white opacity-70" />
                <span className="relative inline-flex h-1.5 w-1.5 rounded-full bg-white" />
              </span>
              {t("gallery.card.liveNow")}
              {liveSecs != null && ` · ${fmtDuration(liveSecs)}`}
              {liveInfo.viewers != null && ` · 👁 ${liveInfo.viewers}`}
            </span>
          ) : (liveSecs ?? video.duration_secs) != null && (
            <span className="absolute bottom-2 right-2 rounded bg-black/75 px-1.5 py-0.5 text-[11px] font-semibold text-white">
              {fmtDuration(liveSecs ?? video.duration_secs)}
            </span>
          )
        ) : isYoutubeOnly ? (
          (liveInfo?.duration_secs ?? video.duration_secs) != null && (
            <span className="absolute bottom-2 right-2 rounded bg-black/75 px-1.5 py-0.5 text-[11px] font-semibold text-white">
              {fmtDuration(liveInfo?.duration_secs ?? video.duration_secs)}
            </span>
          )
        ) : (driveOnly ? video.duration_secs : duration) != null && (
          <span className="absolute bottom-2 right-2 rounded bg-black/75 px-1.5 py-0.5 text-[11px] font-semibold text-white">
            {fmtDuration(driveOnly ? video.duration_secs : duration)}
          </span>
        )}
        {video.kind === "clip" && (
          <span className="absolute left-2 top-2 rounded bg-accent-500/90 px-1.5 py-0.5 text-[9px] font-bold uppercase tracking-wider text-stone-950">
            {t("gallery.kindFilter.clipBadge")}
          </span>
        )}
        {isLiveKind && (
          <span className="absolute left-2 top-2 flex items-center gap-1 rounded bg-red-600/90 px-1.5 py-0.5 text-[9px] font-bold uppercase tracking-wider text-white">
            <SiYoutube size={9} /> {t("gallery.card.youtubeLive")}
          </span>
        )}
        <div className="absolute right-2 top-2 z-10 flex flex-col items-end gap-1">
          <button title={t(video.favorite ? "gallery.card.unfavorite" : "gallery.card.favorite")}
            onClick={(e) => { e.stopPropagation(); onToggleFavorite(video); }}
            className={`flex h-5 w-5 items-center justify-center rounded bg-black/60 backdrop-blur-[2px] transition ${
              video.favorite ? "text-amber-400" : "text-white/70 opacity-0 hover:text-amber-300 group-hover:opacity-100"
            }`}>
            <Icon.Star size={12} fill={video.favorite ? "currentColor" : "none"} />
          </button>
          {!isLiveKind && video.youtube_video_id && (
            <span title={t("gallery.card.uploadedToYoutube")}
              className="flex items-center gap-1 rounded bg-black/75 px-1.5 py-0.5 text-[9px] font-bold uppercase tracking-wider text-white">
              <SiYoutube size={10} className="text-red-500" />
            </span>
          )}
        </div>
        {!hostedOnYoutube && !driveOnly && (
          uploading ? (
            <span title={uploadPct != null ? `${t("gallery.menu.uploadDrive")} ${uploadPct}%` : t("gallery.menu.uploadDrive")}
              className="absolute left-2 bottom-2 flex items-center gap-1 rounded bg-black/75 px-1.5 py-1 text-accent-400">
              <Icon.Cloud size={11} className="shrink-0" />
              {uploadPct != null && <span className="text-[10px] font-semibold tabular-nums">{uploadPct}%</span>}
            </span>
          ) : video.drive_synced && (
            <span title={t("gallery.card.synced")}
              className="absolute left-2 bottom-2 flex items-center rounded bg-black/75 p-1 text-emerald-400">
              <Icon.Cloud size={11} />
            </span>
          )
        )}
        {/* Upload progress bar: pulses full-width until real byte counts arrive, then tracks them. */}
        {uploading && (
          <div className="absolute inset-x-0 bottom-0 h-1 bg-black/40">
            <div
              className={`h-full bg-accent-400 ${uploadPct == null ? "w-full animate-pulse" : ""}`}
              style={uploadPct != null ? { width: `${uploadPct}%`, transition: "width 300ms ease" } : undefined}
            />
          </div>
        )}
        {/* Deletion has no byte count to track, just an indeterminate pulse —
            red instead of upload's accent so the two never look alike. */}
        {deleting && (
          <>
            <div className="absolute inset-0 flex items-center justify-center bg-black/50">
              <div className="h-5 w-5 animate-spin rounded-full border-2 border-red-400/30 border-t-red-400" />
            </div>
            <div className="absolute inset-x-0 bottom-0 h-1 bg-black/40">
              <div className="h-full w-full animate-pulse bg-red-500" />
            </div>
          </>
        )}
      </div>

      {/* Fixed height so every card stays the same size regardless of title/meta length. */}
      <div className="vc-info flex h-[76px] items-start justify-between gap-2 overflow-hidden px-3 pt-2.5 pb-2">
        <div className="flex min-w-0 items-start gap-2.5">
          {appIcon && (
            <img src={appIcon} alt="" title={video.app} className="vc-appicon mt-0.5 h-9 w-9 shrink-0 rounded-lg object-cover" />
          )}
          <div className="min-w-0">
            <div className="flex items-center gap-1.5">
              <div className="vc-title min-w-0 truncate text-sm font-semibold text-stone-100">{displayTitle(video)}</div>
              <TagDots cardTags={cardTags} />
            </div>
            <div className="vc-meta mt-1 flex flex-wrap items-center gap-x-1.5 gap-y-0.5 text-[11px] text-stone-500">
              {!appIcon && video.app && (
                <>
                  <span className="max-w-[90px] truncate" title={video.app}>{video.app}</span>
                  <span className="text-stone-700">•</span>
                </>
              )}
              {video.folder && (
                <>
                  <span className="flex max-w-[90px] items-center gap-0.5 truncate text-stone-500" title={video.folder}>
                    <Icon.Folder size={10} className="shrink-0" />
                    {video.folder}
                  </span>
                  <span className="text-stone-700">•</span>
                </>
              )}
              {hostedOnYoutube && (
                <>
                  <SiYoutube size={11} className="shrink-0 text-red-500" />
                  <span className="whitespace-nowrap">YouTube</span>
                  <span className="text-stone-700">•</span>
                </>
              )}
              {driveOnly && (
                <>
                  <Icon.Cloud size={11} className="shrink-0 text-emerald-400" />
                  <span className="whitespace-nowrap">{t("gallery.card.synced")}</span>
                  <span className="text-stone-700">•</span>
                </>
              )}
              {video.size != null && <><span className="whitespace-nowrap">{fmtBytes(video.size)}</span><span className="text-stone-700">•</span></>}
              {resolution && <><span className="whitespace-nowrap">{resolution.width}×{resolution.height}</span><span className="text-stone-700">•</span></>}
              <span className="flex items-center gap-1 whitespace-nowrap" title={fmtFullDate(video.modified, lang)}>
                <Icon.Zap size={10} className="text-accent-400" />
                {fmtWhen(video.modified, lang)}
              </span>
              {video.stream_info && <><span className="text-stone-700">•</span><span className="whitespace-nowrap">{video.stream_info}</span></>}
            </div>
          </div>
        </div>
        <div className="relative flex shrink-0 items-center gap-0.5">
          {/* Compact icon-only hover actions next to the menu. */}
          {hostedOnYoutube ? (
            <button title={t("videoEditor.youtubeModal.openVideo")}
              onClick={(e) => { e.stopPropagation(); openOrPlay(); }}
              className="rounded p-1.5 text-red-500 opacity-0 transition hover:bg-stone-700/60 hover:text-red-400 group-hover:opacity-100">
              <SiYoutube size={14} />
            </button>
          ) : driveOnly ? (
            <button title={t("gallery.menu.downloadToDevice")}
              onClick={(e) => { e.stopPropagation(); downloadToDevice(); }}
              disabled={downloading}
              className="rounded p-1.5 text-emerald-400 opacity-0 transition hover:bg-stone-700/60 hover:text-emerald-300 group-hover:opacity-100 disabled:opacity-100">
              <Icon.Cloud size={14} />
            </button>
          ) : richActions && (
            <>
              {video.youtube_video_id ? (
                <button title={t("videoEditor.youtubeModal.openVideo")}
                  onClick={(e) => {
                    e.stopPropagation();
                    invoke("open_url", { url: `https://youtube.com/watch?v=${video.youtube_video_id}` })
                      .catch(() => window.open(`https://youtube.com/watch?v=${video.youtube_video_id}`, "_blank"));
                  }}
                  className="rounded p-1.5 text-red-500 opacity-0 transition hover:bg-stone-700/60 hover:text-red-400 group-hover:opacity-100">
                  <SiYoutube size={14} />
                </button>
              ) : (
                <button title={t("videoEditor.uploadToYoutube")}
                  onClick={(e) => { e.stopPropagation(); onUploadYoutube(video); }}
                  className="rounded p-1.5 text-stone-500 opacity-0 transition hover:bg-stone-700/60 hover:text-red-400 group-hover:opacity-100">
                  <SiYoutube size={14} />
                </button>
              )}
              {EDITOR_ENABLED && (
                <button title={t("gallery.video.edit")}
                  onClick={(e) => { e.stopPropagation(); onEdit(video.local_path); }}
                  className="rounded p-1.5 text-stone-500 opacity-0 transition hover:bg-stone-700/60 hover:text-stone-100 group-hover:opacity-100">
                  <Icon.Pencil size={14} />
                </button>
              )}
            </>
          )}
          <button title={t("gallery.menu.more")}
            onClick={(e) => { e.stopPropagation(); menuOpen ? closeMenu() : openMenu(kebabMenuPos(e.currentTarget.getBoundingClientRect()), e.shiftKey); }}
            className="rounded p-1.5 text-stone-500 opacity-0 transition hover:bg-stone-700/60 hover:text-stone-100 group-hover:opacity-100">
            <Icon.MoreVertical size={15} />
          </button>
          {menuOpen && (
            <CardMenu
              t={t}
              tags={tags}
              assigned={video.tags || []}
              position={menuPos}
              permanent={menuPermanent}
              onToggleTag={(tagId) => onToggleTag(video, tagId)}
              onOpenEditor={EDITOR_ENABLED ? () => { closeMenu(); onEdit(video.local_path); } : undefined}
              onOpenExternal={hostedOnYoutube ? () => { closeMenu(); openOrPlay(); } : undefined}
              onUploadYoutube={() => { closeMenu(); onUploadYoutube(video); }}
              onOpenYoutube={video.youtube_video_id ? () => { closeMenu(); openYoutubeVideo(video.youtube_video_id); } : undefined}
              onOpenYoutubeStudio={video.youtube_video_id ? () => { closeMenu(); openYoutubeStudio(video.youtube_video_id, youtubeEmail); } : undefined}
              onCreateYoutubeClip={video.youtube_video_id ? () => { closeMenu(); openYoutubeClip(video.youtube_video_id, youtubeEmail); } : undefined}
              onUploadDrive={() => { closeMenu(); onUploadDrive(video); }}
              onDownload={driveOnly ? () => { closeMenu(); downloadToDevice(); } : undefined}
              onCopyDriveLink={() => { closeMenu(); onCopyDriveLink(video); }}
              onOpenDrive={video.drive_synced && video.drive_id ? () => { closeMenu(); openDriveFile(video.drive_id, driveEmail); } : undefined}
              driveConnected={driveConnected}
              driveSynced={video.drive_synced}
              driveOnly={driveOnly}
              backedUp={!!video.youtube_video_id || video.drive_synced}
              hasLocalFile={!hostedOnYoutube && !driveOnly}
              onReveal={() => { closeMenu(); invoke("reveal_item", { path: video.local_path }).catch(() => {}); }}
              onShowInFolderView={onShowInFolderView ? () => { closeMenu(); onShowInFolderView(); } : undefined}
              onShowDetails={() => { closeMenu(); onShowDetails(video); }}
              onDelete={handleDeleteClick}
              onDeleteDriveCopy={handleDeleteDriveCopyClick}
              onDeleteBoth={handleDeleteBothClick}
              onClose={closeMenu}
            />
          )}
        </div>
      </div>

    </div>
  );
}

// List view mode's fixed row height and column template. Leaner than
// VideoCard's thumbnail (no live/clip/upload overlays fit at 44px); the
// status column covers local/synced/YouTube at a glance instead.
const LIST_ROW_H = 44;
// Leading 28px column is the selection checkbox (its own column now, not
// overlaid on the thumbnail); then thumb, title, app, date, size, source, menu.
const LIST_COLS = "28px 64px 1fr 150px 110px 76px 64px 32px";

function VideoListRow({ video, t, lang, tags, onPlay, onDelete, onDeleteDriveCopy, onDeleteBoth, onToggleTag, onToggleFavorite, onUploadYoutube, onUploadDrive, transfer, driveConnected, driveEmail, youtubeChannelId, youtubeEmail, onEdit, isSelected, hasSelection, bulkDeleting, onToggleSelect, onSelectOnly, onShowInFolderView, onShowDetails, onCopyDriveLink, onBulkContextMenu, highlighted }) {
  const uploading = transfer?.status === "uploading" || transfer?.status === "queued";
  const { hostedOnYoutube, driveOnly, isRecordingFinishing, isRecordingCard, recSecs, liveSize, thumb, setThumb } = useVideoMedia(video);
  const [menuOpen, setMenuOpen] = useState(false);
  const [menuPos, setMenuPos] = useState(null);
  const [menuPermanent, setMenuPermanent] = useState(false); // menu opened with Shift held — bypass Recycle Bin
  const { openOrPlay, downloadToDevice, downloading } = useVideoOpenOrPlay(video, hostedOnYoutube, driveOnly, onPlay);
  const appIcon = useAppIcon(video.app);
  const cardTags = tagsFor(video, tags);

  const openMenu = (pos, permanent = false) => { setMenuPos(pos); setMenuPermanent(permanent); setMenuOpen(true); };
  const closeMenu = () => { setMenuOpen(false); setMenuPos(null); };
  const { deleting: ownDeleting, handleDeleteClick, handleDeleteDriveCopyClick, handleDeleteBothClick } = useVideoDelete(video, onDelete, onDeleteDriveCopy, onDeleteBoth, menuPermanent, closeMenu);
  const deleting = ownDeleting || bulkDeleting;

  // See `VideoCard`'s matching branch — a still-recording file has no menu
  // and no hover preview, but does open in the real player on click now.
  if (isRecordingCard) {
    return (
      <div style={{ display: "grid", gridTemplateColumns: LIST_COLS, height: LIST_ROW_H, alignItems: "center" }}
        className="group cursor-pointer gap-3 border-b border-stone-800/40 px-3 text-[12px] transition hover:bg-stone-900/60"
        onClick={() => openOrPlay()}>
        <div />
        <div className="relative flex h-8 w-11 shrink-0 items-center justify-center overflow-hidden rounded bg-stone-950">
          {thumb
            ? <img src={thumb} alt="" className="absolute inset-0 h-full w-full object-cover" />
            : appIcon
              ? <img src={appIcon} alt="" className="h-5 w-5 rounded object-cover opacity-80" />
              : <Icon.Monitor size={13} className="text-stone-700" />}
        </div>
        <div className="min-w-0 truncate font-medium text-stone-200">{displayTitle(video)}</div>
        <div className={`min-w-0 flex items-center gap-1.5 ${isRecordingFinishing ? "text-stone-500" : "text-red-400"}`}>
          <span className="relative flex h-1.5 w-1.5">
            {!isRecordingFinishing && <span className="absolute inline-flex h-full w-full animate-ping rounded-full bg-red-400 opacity-70" />}
            <span className={`relative inline-flex h-1.5 w-1.5 rounded-full ${isRecordingFinishing ? "bg-stone-500" : "bg-red-500"}`} />
          </span>
          <span className="truncate text-[11px] font-semibold uppercase tracking-wider">{isRecordingFinishing ? t("gallery.card.recordingFinishing") : t("gallery.card.recordingInProgress")}</span>
        </div>
        <div className="min-w-0 whitespace-nowrap text-stone-600 tabular-nums">{fmtDuration(recSecs)}</div>
        <div className="min-w-0 whitespace-nowrap text-right text-stone-600 tabular-nums">{liveSize != null ? fmtBytes(liveSize) : ""}</div>
        <div />
        <div />
      </div>
    );
  }

  const handleRowClick = (e) => {
    if (deleting) return;
    if (e.ctrlKey || e.metaKey) { e.preventDefault(); onToggleSelect(video.name); return; }
    if (hasSelection) { onSelectOnly(video.name); return; }
    openOrPlay();
  };

  return (
    <div data-card-name={video.name}
      style={{ display: "grid", gridTemplateColumns: LIST_COLS, height: LIST_ROW_H, alignItems: "center" }}
      className={`group cursor-pointer gap-3 border-b border-stone-800/40 px-3 text-[12px] transition hover:bg-stone-900/60 animate-fade-in ${deleting ? "pointer-events-none opacity-70" : ""} ${isSelected ? "bg-accent-500/10 ring-1 ring-inset ring-accent-400/50" : ""} ${highlighted ? "animate-highlight-flash" : ""}`}
      onClick={handleRowClick}
      onContextMenu={(e) => {
        e.preventDefault();
        if (onBulkContextMenu) { onBulkContextMenu(e.clientX, e.clientY); return; }
        openMenu({ x: e.clientX, y: e.clientY }, e.shiftKey);
      }}
    >
      {/* Selection checkbox — its own leftmost column (shows on hover or when
          something's already selected), no longer overlaid on the thumbnail. */}
      <div className="flex items-center justify-center">
        <div
          className={`flex h-4 w-4 items-center justify-center rounded border transition ${
            isSelected ? "border-accent-400 bg-accent-400 text-stone-950"
              : `border-stone-600 text-transparent ${hasSelection ? "" : "opacity-0 group-hover:opacity-100"}`
          }`}
          onClick={(e) => { e.stopPropagation(); onToggleSelect(video.name); }}
        >
          <Icon.Check size={9} className="stroke-[3.5]" />
        </div>
      </div>
      <div className="relative h-8 w-11 shrink-0 overflow-hidden rounded bg-stone-950">
        {thumb ? (
          <img src={thumb} alt="" className={`h-full w-full object-cover ${driveOnly ? "opacity-60" : ""}`}
            onError={() => setThumb(null)} referrerPolicy="no-referrer" />
        ) : (
          <div className="flex h-full w-full items-center justify-center">
            {hostedOnYoutube ? <SiYoutube size={14} className="text-red-600/70" />
              : driveOnly ? <Icon.Cloud size={13} className="text-stone-700" />
              : <Icon.Monitor size={13} className="text-stone-700" />}
          </div>
        )}
        {driveOnly && downloading && (
          <div className="absolute inset-0 flex items-center justify-center bg-black/50">
            <div className="h-3 w-3 animate-spin rounded-full border-2 border-stone-400 border-t-stone-900" />
          </div>
        )}
        {(uploading || deleting) && (
          <div className="absolute inset-x-0 bottom-0 h-0.5 bg-black/40">
            <div className={`h-full w-full animate-pulse ${deleting ? "bg-red-500" : "bg-accent-400"}`} />
          </div>
        )}
      </div>
      <div className="flex min-w-0 items-center gap-1.5">
        <span className="min-w-0 truncate font-medium text-stone-200">{displayTitle(video)}</span>
        <TagDots cardTags={cardTags} max={3} />
      </div>
      <div className="min-w-0">
        <div className="flex items-center gap-1.5 text-stone-500">
          {appIcon
            ? <img src={appIcon} alt="" className="h-3.5 w-3.5 shrink-0 rounded object-cover" />
            : video.app
              ? <Icon.Monitor size={12} className="shrink-0 text-stone-700" />
              : null}
          <span className="truncate">{video.app || ""}</span>
        </div>
      </div>
      <div className="min-w-0 truncate whitespace-nowrap text-stone-600" title={fmtFullDate(video.modified, lang)}>{fmtWhen(video.modified, lang)}</div>
      <div className="min-w-0 whitespace-nowrap text-right text-stone-600">{fmtBytes(video.size)}</div>
      <div className="flex min-w-0 items-center justify-center gap-1.5 text-stone-600">
        <button title={t(video.favorite ? "gallery.card.unfavorite" : "gallery.card.favorite")}
          onClick={(e) => { e.stopPropagation(); onToggleFavorite(video); }}
          className={`flex items-center justify-center transition ${
            video.favorite ? "text-amber-400" : "text-stone-700 opacity-0 hover:text-amber-300 group-hover:opacity-100"
          }`}>
          <Icon.Star size={12} fill={video.favorite ? "currentColor" : "none"} />
        </button>
        {hostedOnYoutube ? <SiYoutube size={12} className="text-red-500" title="YouTube" />
          : driveOnly ? <Icon.Cloud size={12} className="text-emerald-400" title={t("gallery.card.synced")} />
          : (
            <>
              <Icon.Monitor size={12} title={t("gallery.card.local")} />
              {video.drive_synced && <Icon.Cloud size={12} className="text-emerald-400" title={t("gallery.card.synced")} />}
              {video.youtube_video_id && <SiYoutube size={12} className="text-red-500" title="YouTube" />}
            </>
          )}
      </div>
      <div className="relative flex shrink-0 items-center justify-end">
        <button title={t("gallery.menu.more")}
          onClick={(e) => { e.stopPropagation(); menuOpen ? closeMenu() : openMenu(kebabMenuPos(e.currentTarget.getBoundingClientRect()), e.shiftKey); }}
          className="rounded p-1 text-stone-500 opacity-0 transition hover:bg-stone-700/60 hover:text-stone-100 group-hover:opacity-100">
          <Icon.MoreVertical size={14} />
        </button>
        {menuOpen && (
          <CardMenu
            t={t}
            tags={tags}
            assigned={video.tags || []}
            position={menuPos}
            permanent={menuPermanent}
            onToggleTag={(tagId) => onToggleTag(video, tagId)}
            onOpenEditor={EDITOR_ENABLED ? () => { closeMenu(); onEdit(video.local_path); } : undefined}
            onOpenExternal={hostedOnYoutube ? () => { closeMenu(); openOrPlay(); } : undefined}
            onUploadYoutube={() => { closeMenu(); onUploadYoutube(video); }}
            onOpenYoutube={video.youtube_video_id ? () => { closeMenu(); openYoutubeVideo(video.youtube_video_id); } : undefined}
            onOpenYoutubeStudio={video.youtube_video_id ? () => { closeMenu(); openYoutubeStudio(video.youtube_video_id, youtubeEmail); } : undefined}
              onCreateYoutubeClip={video.youtube_video_id ? () => { closeMenu(); openYoutubeClip(video.youtube_video_id, youtubeEmail); } : undefined}
            onUploadDrive={() => { closeMenu(); onUploadDrive(video); }}
            onDownload={driveOnly ? () => { closeMenu(); openOrPlay(); } : undefined}
            onCopyDriveLink={() => { closeMenu(); copyDriveLink(video); }}
            onOpenDrive={video.drive_synced && video.drive_id ? () => { closeMenu(); openDriveFile(video.drive_id, driveEmail); } : undefined}
            driveConnected={driveConnected}
            driveSynced={video.drive_synced}
            driveOnly={driveOnly}
            backedUp={!!video.youtube_video_id || video.drive_synced}
            hasLocalFile={!hostedOnYoutube && !driveOnly}
            onReveal={() => { closeMenu(); invoke("reveal_item", { path: video.local_path }).catch(() => {}); }}
            onShowInFolderView={onShowInFolderView ? () => { closeMenu(); onShowInFolderView(); } : undefined}
            onShowDetails={() => { closeMenu(); onShowDetails(video); }}
            onDelete={handleDeleteClick}
            onDeleteDriveCopy={handleDeleteDriveCopyClick}
            onDeleteBoth={handleDeleteBothClick}
            onClose={closeMenu}
          />
        )}
      </div>
    </div>
  );
}

// Thumbnails for a tile's cover videos, sharing the cache with regular cards.
// Returns thumb URLs matching `covers`' order, `null` where unresolved.
function useTileCoverThumbs(covers) {
  const cacheKeyOf = (c) => `${c.name}:${c.modified ?? 0}`;
  const fetcherFor = (c) => {
    const hostedOnYoutube = c.kind === "youtube_live" || c.kind === "youtube_only";
    if (hostedOnYoutube && c.youtubeId) {
      const fallback = `https://i.ytimg.com/vi/${c.youtubeId}/mqdefault.jpg`;
      return () => invoke("get_youtube_live_info", { videoId: c.youtubeId })
        .then((info) => info.thumbnail || fallback)
        .catch(() => fallback);
    }
    if (c.driveOnly && c.driveId) {
      return () => invoke("read_drive_video_thumbnail", { driveId: c.driveId }).then((b64) => `data:image/jpeg;base64,${b64}`);
    }
    return () => invoke("read_video_thumbnail", { name: c.name }).then((b64) => `data:image/jpeg;base64,${b64}`);
  };

  const key = covers.map((c) => cacheKeyOf(c)).join("|");
  const [thumbs, setThumbs] = useState(() => covers.map((c) => thumbCache.get(`t:${cacheKeyOf(c)}`) ?? null));
  useEffect(() => {
    let cancelled = false;
    setThumbs(covers.map((c) => thumbCache.get(`t:${cacheKeyOf(c)}`) ?? null));
    covers.forEach((c, i) => {
      fetchCached(thumbCache, `t:${cacheKeyOf(c)}`, fetcherFor(c)).then((v) => {
        if (cancelled || !v) return;
        setThumbs((prev) => { const next = [...prev]; next[i] = v; return next; });
      });
    });
    return () => { cancelled = true; };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key]);
  return thumbs;
}

// Cover collage: a few thumbnails instead of just the latest video, plus a
// "+N" badge when the tile holds more videos than were actually fetched.
function TileCoverCollage({ thumbs, totalCount, className }) {
  const slotCount = thumbs.length;
  if (slotCount === 0 || thumbs.every((t) => !t)) return null;
  const extra = Math.max(0, (totalCount ?? slotCount) - slotCount);

  // Real thumbnails, plus a trailing "+N" badge (over a dimmed thumbnail)
  // when there's more content than was fetched.
  const realCount = extra > 0 ? Math.min(slotCount, 3) : Math.min(slotCount, 4);
  const items = thumbs.slice(0, realCount).map((src) => ({ src, badge: null }));
  if (extra > 0) items.push({ src: thumbs[realCount] ?? null, badge: extra });

  const renderItem = (item, key, extraClass = "") => item.badge != null ? (
    <div key={key} className={`relative overflow-hidden ${extraClass}`}>
      {item.src && <img src={item.src} alt="" className="absolute inset-0 h-full w-full object-cover opacity-70" referrerPolicy="no-referrer" />}
      <div className="absolute inset-0 flex items-center justify-center bg-black/60 text-sm font-bold text-white">+{item.badge}</div>
    </div>
  ) : item.src ? (
    <img key={key} src={item.src} alt="" className={`h-full w-full object-cover opacity-70 ${extraClass}`} referrerPolicy="no-referrer" />
  ) : (
    <div key={key} className={`h-full w-full bg-stone-900/60 ${extraClass}`} />
  );

  // Layout picked from the actual cell count (1-4) so there's never a
  // dangling empty grid cell.
  if (items.length === 1) return <div className={`absolute inset-0 ${className ?? ""}`}>{renderItem(items[0], 0)}</div>;
  if (items.length === 2) {
    return <div className={`absolute inset-0 grid grid-cols-2 gap-0.5 ${className ?? ""}`}>{items.map((it, i) => renderItem(it, i))}</div>;
  }
  if (items.length === 3) {
    // Big + 2 stacked, not a 2x2 grid with a missing fourth cell.
    return (
      <div className={`absolute inset-0 grid grid-cols-2 grid-rows-2 gap-0.5 ${className ?? ""}`}>
        {renderItem(items[0], 0, "row-span-2")}
        {renderItem(items[1], 1)}
        {renderItem(items[2], 2)}
      </div>
    );
  }
  return <div className={`absolute inset-0 grid grid-cols-2 grid-rows-2 gap-0.5 ${className ?? ""}`}>{items.map((it, i) => renderItem(it, i))}</div>;
}

// Aggregate cloud status for a game/folder tile: "full" (every recording is
// backed up), "partial" (some are), or "none" (nothing on Drive yet — no
// badge at all, same as a plain local-only video card).
function TileDriveBadge({ status, t, size = 12 }) {
  if (status === "none") return null;
  return (
    <Icon.Cloud size={size}
      className={status === "full" ? "text-emerald-400" : "text-stone-500"}
      title={t(status === "full" ? "gallery.folders.driveFull" : "gallery.folders.drivePartial")} />
  );
}

// `tile` is a game or folder entry. Right-click on a folder opens its edit
// modal, on a game opens its per-game settings, instead of a context menu;
// title sits over the thumbnail gradient.
function TileCard({ tile, onOpen, onContextMenu, t }) {
  const thumbs = useTileCoverThumbs(tile.covers ?? []);
  const hasThumb = thumbs.some(Boolean);
  const displayName = tile.name;
  const appIcon = useAppIcon(tile.kind === "game" ? tile.name : null);
  const icon = (size, className) => appIcon
    ? <img src={appIcon} alt="" className={`rounded-md object-cover drop-shadow ${className}`} style={{ width: size, height: size }} />
    : tile.kind === "game"
      ? <Icon.Monitor size={size} className={`text-accent-300 drop-shadow ${className}`} />
      : <Icon.Folder size={size} className={`text-accent-300 drop-shadow ${className}`} />;
  return (
    <div className="group relative aspect-video cursor-pointer overflow-hidden rounded-xl bg-stone-900 transition hover:bg-stone-800/80 animate-fade-in"
      onClick={() => onOpen(tile)}
      onContextMenu={(e) => { e.preventDefault(); onContextMenu(tile, e.clientX, e.clientY); }}>
      <TileCoverCollage thumbs={thumbs} totalCount={tile.count} />
      {!hasThumb && (
        <div className="absolute inset-0 flex items-center justify-center">
          {icon(44, "opacity-70 transition group-hover:opacity-100 group-hover:text-accent-400")}
        </div>
      )}
      {tile.driveStatus !== "none" && (
        <div className="absolute right-1.5 top-1.5 flex h-5 w-5 items-center justify-center rounded-md bg-black/60">
          <TileDriveBadge status={tile.driveStatus} t={t} />
        </div>
      )}
      <div className="absolute inset-x-0 bottom-0 flex items-center gap-1.5 bg-gradient-to-t from-stone-950/95 via-stone-950/60 to-transparent px-3 pb-2 pt-6">
        {icon(16, "shrink-0")}
        <span className="truncate text-sm font-semibold text-stone-100 drop-shadow">{displayName}</span>
      </div>
    </div>
  );
}

function TileListRow({ tile, onOpen, onContextMenu, t }) {
  // No cover-collage fetch in list mode — a folder/app icon is shown instead.
  const displayName = tile.name;
  const appIcon = useAppIcon(tile.kind === "game" ? tile.name : null);
  return (
    <div style={{ display: "grid", gridTemplateColumns: LIST_COLS, height: LIST_ROW_H, alignItems: "center" }}
      className="group cursor-pointer gap-3 border-b border-stone-800/40 px-3 text-[12px] transition hover:bg-stone-900/60 animate-fade-in"
      onClick={() => onOpen(tile)}
      onContextMenu={(e) => { e.preventDefault(); onContextMenu(tile, e.clientX, e.clientY); }}>
      {/* Leading (checkbox) column — folders/games aren't selectable. */}
      <div />
      {/* List rows show a plain folder/app icon, not the video-collage
          thumbnail the grid tiles use — a folder reads as a folder at a glance. */}
      <div className="flex h-8 w-11 shrink-0 items-center justify-center">
        {tile.kind === "game" && appIcon
          ? <img src={appIcon} alt="" className="h-6 w-6 rounded object-cover" />
          : <Icon.Folder size={20} className="text-accent-400/90" />}
      </div>
      <div className="min-w-0 truncate font-medium text-stone-200">{displayName}</div>
      <div className="min-w-0 text-stone-600">{t("gallery.folders.itemCount")(tile.count)}</div>
      <div />
      <div />
      <div className="flex min-w-0 items-center justify-center">
        <TileDriveBadge status={tile.driveStatus} t={t} />
      </div>
      <div />
    </div>
  );
}

const SORTERS = {
  dateDesc: (a, b) => (b.modified ?? 0) - (a.modified ?? 0),
  dateAsc: (a, b) => (a.modified ?? 0) - (b.modified ?? 0),
  sizeDesc: (a, b) => (b.size ?? 0) - (a.size ?? 0),
  sizeAsc: (a, b) => (a.size ?? 0) - (b.size ?? 0),
  nameAsc: (a, b) => (a.title || a.name).localeCompare(b.title || b.name),
  nameDesc: (a, b) => (b.title || b.name).localeCompare(a.title || a.name),
  appAsc: (a, b) => (a.app || "").localeCompare(b.app || ""),
  appDesc: (a, b) => (b.app || "").localeCompare(a.app || ""),
};

// List-header column → its two sort keys (first = default when you click it
// fresh). Clicking the active column toggles between the pair; a new column
// jumps to its default. `SOURCE` has no meaningful order, so it's not here.
const SORT_COLUMNS = {
  title: ["nameAsc", "nameDesc"],
  app: ["appAsc", "appDesc"],
  date: ["dateDesc", "dateAsc"], // newest first by default
  size: ["sizeDesc", "sizeAsc"], // biggest first by default
};

// Clickable list-view column header: shows the current sort direction when
// active, toggles it on click, and defaults a fresh column to its first key.
function ListHeaderCell({ label, field, align, sortBy, onSortChange }) {
  const pair = SORT_COLUMNS[field];
  const active = pair.includes(sortBy);
  const asc = active && sortBy.endsWith("Asc");
  const next = sortBy === pair[0] ? pair[1] : pair[0];
  return (
    <button
      onClick={() => onSortChange(next)}
      className={`flex items-center gap-1 transition hover:text-stone-300 ${align === "right" ? "justify-end" : ""} ${active ? "text-stone-300" : ""}`}
    >
      <span className="truncate">{label}</span>
      {active && <span className="shrink-0 text-[9px]">{asc ? "▲" : "▼"}</span>}
    </button>
  );
}

export default function VideoGrid({ t, lang, tags, recording, tagFilter, kindFilter, favoritesOnly, selectedGame, selectedFolderId, rootOnly, tiles = [], tilesLabel, onOpenTile, onTileContextMenu, search, sortBy, onSortChange, groupBy, viewMode, onVideosChanged, onFilteredCount, onEdit, refreshToken, onRefreshingChange, onShowInFolderView, highlightName, onHighlightDone, openPlayerRequest, onOpenPlayerDone, closePlayerToken }) {
  const [videos, setVideos] = useState(null); // null = loading
  const [playing, setPlaying] = useState(null);
  // Dev-only: opens an arbitrary local file directly in the player, bypassing
  // the loaded `videos` list entirely — see store_screenshots.rs's
  // "goto-player" action, used to screenshot the trim tool against a
  // dedicated demo clip that doesn't need to exist as a real library card.
  useEffect(() => {
    if (!openPlayerRequest) return;
    setPlaying({
      name: openPlayerRequest.name,
      local_path: openPlayerRequest.path,
      modified: Date.now(),
      tags: [],
      drive_only: false,
      drive_synced: false,
    });
    onOpenPlayerDone?.();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [openPlayerRequest]);
  // Dev-only: closes the player opened above — `view` alone (`goto-view`)
  // doesn't do this, since "folders"/"gallery" are both this same mounted
  // component with `playing` as its own internal state, not tied to `view`.
  // See store_screenshots.rs's "close-player" action.
  useEffect(() => {
    if (!closePlayerToken) return;
    setPlaying(null);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [closePlayerToken]);
  const [playerMenuOpen, setPlayerMenuOpen] = useState(false);
  const [playerMenuPos, setPlayerMenuPos] = useState(null);
  const [youtubeTarget, setYoutubeTarget] = useState(null); // video or null
  const [detailsVideo, setDetailsVideo] = useState(null); // video or null — card menu's "Details"
  // Lightweight in-gallery toast (e.g. "Copied to clipboard") — confined to
  // this window, unlike `crate::toast`'s always-on-top overlay meant for
  // recording/game events that need to show over a fullscreen game.
  const [toastMsg, setToastMsg] = useState(null);
  const toastTimerRef = useRef(null);
  const showToast = useCallback((message) => {
    clearTimeout(toastTimerRef.current);
    setToastMsg(message);
    toastTimerRef.current = setTimeout(() => setToastMsg(null), 2000);
  }, []);
  useEffect(() => () => clearTimeout(toastTimerRef.current), []);
  const [driveConnected, setDriveConnected] = useState(false);
  const [driveEmail, setDriveEmail] = useState(null); // connected account — pins Google links to the right authuser
  const [youtubeChannelId, setYoutubeChannelId] = useState(null); // for channel-scoped Studio links
  const [youtubeEmail, setYoutubeEmail] = useState(null); // account that owns the upload channel — pins Studio authuser
  const [transfers, setTransfers] = useState(new Map()); // file name -> TransferInfo
  const [highlightedName, setHighlightedName] = useState(null); // "Show in Folder View" target, briefly ringed
  const scrollRef = useRef(null);
  const [containerWidth, setContainerWidth] = useState(0);
  // Group scrubber (drag-like-a-phone-gallery side rail) state.
  const [scrollRatio, setScrollRatio] = useState(0);
  const [scrollMetrics, setScrollMetrics] = useState({ clientH: 1, scrollH: 1 });
  const [scrubDragging, setScrubDragging] = useState(false);
  const [scrubLabelText, setScrubLabelText] = useState("");
  const scrubTrackRef = useRef(null);
  const scrubTrackRectRef = useRef(null);
  const scrubberSectionsRef = useRef([]);
  // Bulk selection (checkbox + drag-marquee select) state.
  const [selectedNames, setSelectedNames] = useState(new Set());
  const [bulkMenu, setBulkMenu] = useState(null); // {x,y} — right-click on a multi-selection
  const [dragStart, setDragStart] = useState(null); // {x,y} in viewport coords, content-anchored
  const [dragEnd, setDragEnd] = useState(null);
  const [isDragging, setIsDragging] = useState(false);
  const [dragInitialSelected, setDragInitialSelected] = useState(new Set());
  const [confirm, setConfirm] = useState(null); // {message, action} | null
  // Names currently being removed by the bulk "Delete" action — cards for
  // these get the same dimmed/pulsing "deleting" treatment single-item
  // delete already has, instead of just sitting still until `reload()`
  // suddenly updates the whole list at once.
  const [deletingNames, setDeletingNames] = useState(new Set());
  // Live progress from the backend's Drive file listing (paginated 1000/page
  // — see `drive::api::list_files`) — `null` once nothing's actively
  // scanning (cache hit, offline, or the scan just finished).
  const [driveScanProgress, setDriveScanProgress] = useState(null);
  const scrollTopRef = useRef(0);
  const dragStartScrollTopRef = useRef(0);
  const dragCtrlRef = useRef(false);

  // `force` bypasses the backend's 30s Drive-listing cache — only the
  // explicit refresh button passes it; every other trigger below (events,
  // post-delete cleanup) keeps the cache so they don't hammer the Drive API.
  const reload = (force) => invoke("list_videos", { force })
    .then((v) => { setVideos(v); onVideosChanged?.(v); })
    .catch(() => setVideos([]))
    .finally(() => setDriveScanProgress(null));

  // Paint the last-known list from the Rust cache instantly on mount, then
  // refresh in the background — avoids a loading flash on every webview reload.
  // eslint-disable-next-line react-hooks/exhaustive-deps
  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const cached = await invoke("get_cached_videos");
        if (!cancelled && cached) { setVideos(cached); onVideosChanged?.(cached); }
      } catch { /* no cache yet — first launch */ }
      if (cancelled) return;
      onRefreshingChange?.(true);
      reload(false).finally(() => { if (!cancelled) onRefreshingChange?.(false); });
    })();
    return () => { cancelled = true; };
  }, []);

  // The breadcrumb's refresh button bumps `refreshToken` — a real "get me
  // current data" click, so this forces past the Drive cache. Skips its own
  // first run (initial mount is handled above, and shouldn't force).
  const didInitRef = useRef(false);
  // eslint-disable-next-line react-hooks/exhaustive-deps
  useEffect(() => {
    if (!didInitRef.current) { didInitRef.current = true; return; }
    onRefreshingChange?.(true);
    reload(true).finally(() => onRefreshingChange?.(false));
  }, [refreshToken]);

  useEffect(() => {
    let unlisten;
    listen("drive-scan-progress", (e) => setDriveScanProgress(e.payload)).then((u) => { unlisten = u; });
    return () => unlisten?.();
  }, []);

  // Drop selected names for videos that no longer exist, so the floating
  // bar's count never outlives its items.
  useEffect(() => {
    if (!videos) return;
    setSelectedNames((prev) => {
      if (prev.size === 0) return prev;
      const valid = new Set(videos.map((v) => v.name));
      const next = new Set([...prev].filter((n) => valid.has(n)));
      return next.size === prev.size ? prev : next;
    });
  }, [videos]);

  const toggleSelectItem = useCallback((name) => {
    setSelectedNames((prev) => {
      const next = new Set(prev);
      if (next.has(name)) next.delete(name); else next.add(name);
      return next;
    });
  }, []);
  const selectOnly = useCallback((name) => setSelectedNames(new Set([name])), []);

  // Tracks the scroll container's real size so row-chunking and the
  // scrubber's thumb size stay accurate regardless of window size.
  useEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    const compute = () => {
      // Use the border-box width (getBoundingClientRect), not clientWidth:
      // clientWidth shrinks by the scrollbar's width whenever a vertical
      // scrollbar appears, so a short folder (no scrollbar) measured wider
      // than a full one — enough to bump the grid up a whole column and shrink
      // every card/tile in the same view mode. Border-box width is fixed by
      // the flex layout regardless of the scrollbar; subtracting a constant
      // reserve for it keeps the column count identical whether or not a
      // folder actually scrolls.
      const w = el.getBoundingClientRect().width - SCROLLBAR_RESERVE;
      setContainerWidth(w);
      setScrollMetrics({ clientH: el.clientHeight, scrollH: el.scrollHeight });
    };
    compute();
    const ro = new ResizeObserver(compute);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const handleScroll = (e) => {
    const el = e.currentTarget;
    const max = el.scrollHeight - el.clientHeight;
    setScrollRatio(max > 0 ? el.scrollTop / max : 0);
    setScrollMetrics({ clientH: el.clientHeight, scrollH: el.scrollHeight });
    scrollTopRef.current = el.scrollTop;
    if (dragStart) recomputeDragSelection();
  };

  // Live per-file upload progress, indexed by name. Only active/queued entries;
  // "done"/"error" ones live in `history` instead.
  useEffect(() => {
    const apply = (payload) => {
      const map = new Map();
      for (const t of [...(payload.active || []), ...(payload.queued || [])]) map.set(t.file, t);
      setTransfers(map);
    };
    invoke("get_transfers").then(apply).catch(() => {});
    let unlisten;
    listen("sync-transfers-changed", (e) => apply(e.payload)).then((u) => { unlisten = u; });
    return () => unlisten?.();
  }, []);

  // Drive connection status, fetched upfront for the card menu's "Upload to Drive" item.
  useEffect(() => {
    const load = () => invoke("get_drive_status")
      .then((s) => {
        setDriveConnected(!!s?.connected);
        setDriveEmail(s?.email || null);
        // The account+channel that own uploaded videos — for the Studio deep link.
        setYoutubeEmail(s?.youtube_email || null);
        if (s?.connected) invoke("get_youtube_channel").then((c) => setYoutubeChannelId(c?.id || null)).catch(() => {});
        else setYoutubeChannelId(null);
      })
      .catch(() => { setDriveConnected(false); setDriveEmail(null); setYoutubeChannelId(null); setYoutubeEmail(null); });
    load();
    let unlisten;
    listen("settings-changed", load).then((u) => { unlisten = u; });
    return () => unlisten?.();
  }, []);

  // "video-saved": recording finalized on disk. "item-synced": Drive upload done,
  // refresh cloud badge. "library-changed": everything else.
  useEffect(() => {
    let unlisten = [];
    listen("video-saved", () => reload()).then((u) => unlisten.push(u));
    listen("item-synced", () => reload()).then((u) => unlisten.push(u));
    listen("library-changed", () => reload()).then((u) => unlisten.push(u));
    return () => unlisten.forEach((u) => u());
  }, []);

  // `permanent` (Shift+right-click) bypasses the Recycle Bin for this one call.
  // Most common failure: the file is still open (e.g. actively loaded in the
  // player) — surfaced via toast instead of silently no-op'ing, since the
  // card staying put after a click otherwise looks like the delete did nothing.
  const handleDelete = async (video, permanent = false) => {
    try {
      await invoke("delete_video", { name: video.name, permanent });
      if (video.drive_only) {
        // Drive's file list can lag behind this delete; drop the card
        // directly instead of `reload()`, which could still hand it back.
        setVideos((prev) => {
          const next = (prev ?? []).filter((v) => v.name !== video.name);
          onVideosChanged?.(next);
          return next;
        });
      } else {
        reload();
      }
    } catch {
      showToast(t("gallery.menu.deleteFailed"));
    }
  };

  // Drops just the Drive backup, local file untouched.
  const handleDeleteDriveCopy = async (video) => {
    try {
      await invoke("delete_drive_copy", { name: video.name });
      reload();
    } catch {
      showToast(t("gallery.menu.deleteFailed"));
    }
  };

  // Both copies in one action — same order as the bulk-delete flow below:
  // dropping the Drive record first is what lets `delete_video` go on to
  // fully remove the item instead of soft-keeping it as a `drive_only` card.
  const handleDeleteBoth = async (video, permanent = false) => {
    if (video.drive_synced && !video.drive_only) {
      await invoke("delete_drive_copy", { name: video.name }).catch(() => {});
    }
    try {
      await invoke("delete_video", { name: video.name, permanent });
    } catch {
      showToast(t("gallery.menu.deleteFailed"));
      return;
    }
    // Not `reload()` — see `handleDelete`'s comment above.
    setVideos((prev) => {
      const next = (prev ?? []).filter((v) => v.name !== video.name);
      onVideosChanged?.(next);
      return next;
    });
  };

  const handleUploadDrive = async (video) => {
    try { await invoke("upload_items", { paths: [video.local_path] }); } catch {}
  };

  // Optimistic: patch just this one video in local state so the chip/star
  // flips instantly, then persist in the background. The old flow awaited the
  // write and then a full `reload()` — which, the first time (cold Drive
  // cache), does a slow paginated Drive scan, so the change only showed up
  // seconds later. Metadata lives locally, so there's nothing to re-fetch.
  // Also mirrors the change onto `playing` (the currently-open player's own
  // snapshot) — without this, toggling a tag/favorite from the player's own
  // "..." menu updates the card behind the modal but not the modal's own
  // menu, which reads its checkmarks off `playing` and looked unresponsive.
  const patchVideo = (name, changes) => {
    setVideos((vs) => vs?.map((v) => (v.name === name ? { ...v, ...changes } : v)) ?? vs);
    setPlaying((p) => (p && p.name === name ? { ...p, ...changes } : p));
  };

  const handleToggleTag = (video, tagId) => {
    const current = video.tags || [];
    const next = current.includes(tagId) ? current.filter((id) => id !== tagId) : [...current, tagId];
    patchVideo(video.name, { tags: next });
    invoke("set_video_tags", { name: video.name, tags: next }).catch(() => {});
  };

  const handleToggleFavorite = (video) => {
    patchVideo(video.name, { favorite: !video.favorite });
    invoke("set_video_favorite", { name: video.name, favorite: !video.favorite }).catch(() => {});
  };

  const openYoutubeUpload = async (video) => {
    try {
      const status = await invoke("get_drive_status");
      setDriveConnected(!!status?.connected);
    } catch {
      setDriveConnected(false);
    }
    setYoutubeTarget(video);
  };

  // Bulk actions for the floating selection bar AND the multi-select
  // right-click menu. Each filters the selection down to videos it actually
  // applies to (e.g. "Upload" only touches local-not-yet-synced ones), same
  // gating as the single-card menu.
  const selectedVideos = () => (videos ?? []).filter((v) => selectedNames.has(v.name));

  // Right-click on a card that's part of a 2+ selection opens the bulk menu
  // at the cursor instead of that one card's own menu. The card decides
  // whether to call this (it knows if it's selected) — see `VideoCard`.
  const requestBulkMenu = (x, y) => setBulkMenu({ x, y });

  const uploadSelected = async () => {
    const list = selectedVideos().filter((v) => v.local_path && !v.drive_only && !v.drive_synced);
    if (!list.length) return;
    setSelectedNames(new Set());
    try { await invoke("upload_items", { paths: list.map((v) => v.local_path) }); } catch {}
  };

  const downloadSelected = async () => {
    const list = selectedVideos().filter((v) => v.drive_only);
    if (!list.length) return;
    setSelectedNames(new Set());
    await Promise.all(list.map((v) => invoke("download_video_from_drive", { driveId: v.drive_id, name: v.name, modifiedMs: v.modified }).catch(() => {})));
    reload();
  };

  const removeDriveCopySelected = async () => {
    const list = selectedVideos().filter((v) => v.drive_synced && !v.drive_only);
    if (!list.length) return;
    setSelectedNames(new Set());
    await Promise.all(list.map((v) => invoke("delete_drive_copy", { name: v.name }).catch(() => {})));
    reload();
  };

  // Drops just the local file, keeping the Drive backup where one exists (a
  // synced item becomes a `drive_only` card; an unsynced one is fully gone —
  // hence the confirm). Same per-item contract as the single card's "Delete
  // local copy" (`delete_video`), applied across the selection.
  const deleteLocalSelected = () => {
    const list = selectedVideos().filter((v) => v.local_path && !v.drive_only);
    if (!list.length) return;
    setConfirm({
      message: t("gallery.confirm.multi")(list.length, t("gallery.confirm.fromLocal")),
      action: async () => {
        setSelectedNames(new Set());
        setDeletingNames(new Set(list.map((v) => v.name)));
        await Promise.all(list.map((v) => invoke("delete_video", { name: v.name }).catch(() => {})));
        setDeletingNames(new Set());
        reload();
      },
    });
  };

  const deleteSelected = () => {
    const list = selectedVideos();
    if (!list.length) return;
    const hasLocal = list.some((v) => !v.drive_only);
    const hasDrive = list.some((v) => v.drive_synced || v.drive_only);
    const from = hasLocal && hasDrive ? t("gallery.confirm.fromBoth") : hasDrive ? t("gallery.confirm.fromDrive") : t("gallery.confirm.fromLocal");
    setConfirm({
      message: t("gallery.confirm.multi")(list.length, from),
      action: async () => {
        setSelectedNames(new Set());
        setDeletingNames(new Set(list.map((v) => v.name)));
        await Promise.all(list.map(async (v) => {
          // `delete_video` alone keeps the Drive backup, so drop it first
          // to fully remove the item instead of leaving a `drive_only` card.
          if (v.drive_synced && !v.drive_only) {
            await invoke("delete_drive_copy", { name: v.name }).catch(() => {});
          }
          await invoke("delete_video", { name: v.name }).catch(() => {});
        }));
        setDeletingNames(new Set());
        // Not `reload()` — see `handleDelete`'s comment above.
        const deletedNames = new Set(list.map((v) => v.name));
        setVideos((prev) => {
          const next = (prev ?? []).filter((v) => !deletedNames.has(v.name));
          onVideosChanged?.(next);
          return next;
        });
      },
    });
  };

  // Recomputes which mounted cards intersect the drag rectangle (off-screen cards
  // can't be dragged over until scrolled into view). Start corner is content-anchored
  // so the rectangle stays put if the list scrolls mid-drag.
  const recomputeDragSelection = useCallback((endPt) => {
    if (!dragStart) return;
    const anchoredStartY = dragStart.y - (scrollTopRef.current - dragStartScrollTopRef.current);
    const end = endPt || dragEnd;
    if (!end) return;

    const rect = {
      left: Math.min(dragStart.x, end.x),
      top: Math.min(anchoredStartY, end.y),
      right: Math.max(dragStart.x, end.x),
      bottom: Math.max(anchoredStartY, end.y),
    };

    const intersected = new Set();
    scrollRef.current?.querySelectorAll("[data-card-name]").forEach((el) => {
      const name = el.getAttribute("data-card-name");
      const elRect = el.getBoundingClientRect();
      const intersects = !(
        elRect.right < rect.left || elRect.left > rect.right ||
        elRect.bottom < rect.top || elRect.top > rect.bottom
      );
      if (intersects) intersected.add(name);
    });

    setSelectedNames(() => {
      const next = new Set(dragInitialSelected);
      if (dragCtrlRef.current) {
        intersected.forEach((name) => { if (dragInitialSelected.has(name)) next.delete(name); else next.add(name); });
      } else {
        next.clear();
        intersected.forEach((name) => next.add(name));
      }
      return next;
    });
  }, [dragStart, dragEnd, dragInitialSelected]);

  useEffect(() => {
    if (!dragStart) return;
    const handleMouseMove = (e) => {
      if (!isDragging) {
        const dist = Math.hypot(e.clientX - dragStart.x, e.clientY - dragStart.y);
        if (dist < 4) return;
        setIsDragging(true);
      }
      const current = { x: e.clientX, y: e.clientY };
      setDragEnd(current);
      recomputeDragSelection(current);
    };
    const handleMouseUp = () => { setDragStart(null); setDragEnd(null); setIsDragging(false); };
    window.addEventListener("mousemove", handleMouseMove);
    window.addEventListener("mouseup", handleMouseUp);
    return () => {
      window.removeEventListener("mousemove", handleMouseMove);
      window.removeEventListener("mouseup", handleMouseUp);
    };
  }, [dragStart, isDragging, recomputeDragSelection]);

  // Marquee-select starts only from empty background — clicking a card,
  // a button, the scrubber, or any fixed overlay (player/upload modal) must
  // never be hijacked into a drag. The player modal is `.absolute`, not
  // `.fixed` (see its own comment), so it needs its own marker attribute.
  const handleMouseDown = (e) => {
    if (e.button !== 0) return;
    if (
      e.target.closest("button") || e.target.closest("input") ||
      e.target.closest("[data-card-name]") || e.target.closest(".fixed") ||
      e.target.closest("[data-player-modal]") ||
      (scrubTrackRef.current && scrubTrackRef.current.contains(e.target))
    ) return;
    e.preventDefault();
    const startPos = { x: e.clientX, y: e.clientY };
    dragStartScrollTopRef.current = scrollTopRef.current;
    dragCtrlRef.current = e.ctrlKey;
    setDragStart(startPos);
    setDragEnd(startPos);
    setIsDragging(false);
    setDragInitialSelected(new Set(e.ctrlKey ? selectedNames : []));
    if (!e.ctrlKey) setSelectedNames(new Set());
  };

  // Synthetic card for the file currently being written; `list_videos` won't have it
  // until "video-saved" fires. Disappears once the real entry arrives via reload.
  const inProgressEntry = useMemo(() => {
    if (!recording?.local || !recording?.current_local_name) return null;
    const app = recording.target?.window?.app ?? null;
    return {
      name: recording.current_local_name,
      title: null,
      app,
      kind: "recording",
      modified: recording.started_at * 1000,
      size: null,
      // The real, currently-growing file — lets the card open in the normal player.
      local_path: recording.current_local_path ?? null,
      tags: [],
      favorite: false,
      drive_synced: false,
      drive_only: false,
      youtube_video_id: null,
      folder: null,
      folder_id: null,
      recordingStartedAt: recording.started_at,
    };
  }, [recording?.local, recording?.current_local_name, recording?.current_local_path, recording?.started_at, recording?.target]);

  // Freezes a last-known snapshot as a "recording_finishing" card to bridge the gap
  // between `inProgressEntry` disappearing and the finished file's reload landing.
  const [finishingEntry, setFinishingEntry] = useState(null);
  const prevInProgressRef = useRef(null);
  useEffect(() => {
    if (inProgressEntry) {
      prevInProgressRef.current = inProgressEntry;
      return;
    }
    const prev = prevInProgressRef.current;
    prevInProgressRef.current = null;
    if (prev) {
      const frozenSecs = prev.recordingStartedAt != null
        ? Math.max(0, Math.floor(Date.now() / 1000) - prev.recordingStartedAt)
        : null;
      setFinishingEntry({ ...prev, kind: "recording_finishing", frozenSecs });
    }
  }, [inProgressEntry]);
  useEffect(() => {
    if (finishingEntry && (videos ?? []).some((v) => v.name === finishingEntry.name)) {
      setFinishingEntry(null);
    }
  }, [videos, finishingEntry]);
  const liveEntry = inProgressEntry ?? finishingEntry;

  const needle = (search ?? "").trim().toLowerCase();
  // Excludes a same-named real entry rather than just prepending, since a
  // reload mid-recording could otherwise hand back both, duplicating the
  // React key.
  const filtered = useMemo(() => (liveEntry
    ? [liveEntry, ...(videos ?? []).filter((v) => v.name !== liveEntry.name)]
    : (videos ?? [])
  ).filter((v) => {
    if (kindFilter === "clip" && v.kind !== "clip") return false;
    if (kindFilter === "video" && v.kind === "clip") return false;
    if (favoritesOnly && !v.favorite) return false;
    // Folders match by id (unambiguous across same-named folders); games
    // match by `app`.
    const matchesGame = (video) => video.app === selectedGame;
    if (selectedFolderId != null) {
      if (v.folder_id !== selectedFolderId) return false;
    } else if (selectedGame != null) {
      // Drilled into a game but not one of its folders: only that game's own
      // loose recordings — its folders' videos only show once you actually
      // open that folder's tile (same rule the true root uses, below).
      if (!matchesGame(v) || v.folder_id != null) return false;
    } else if (rootOnly) {
      // True root of the folder explorer (nothing drilled into): only the
      // loose, unclassified recordings (no detected game, no folder) — every
      // game/folder's own videos only show once you actually open its tile.
      if (v.app || v.folder_id) return false;
    }
    if (tagFilter && !(v.tags || []).includes(tagFilter)) return false;
    if (needle && !(v.title || v.name).toLowerCase().includes(needle)) return false;
    return true;
  }), [videos, liveEntry, kindFilter, favoritesOnly, selectedGame, selectedFolderId, rootOnly, tagFilter, needle]);

  const sortedFiltered = useMemo(
    () => [...filtered].sort(SORTERS[sortBy] ?? SORTERS.dateDesc),
    [filtered, sortBy]
  );

  // Player modal's prev/next — steps through this same list/order, not the
  // whole library, so it matches whatever's actually on screen right now.
  const playingIndex = playing ? sortedFiltered.findIndex((v) => v.name === playing.name) : -1;
  const goToPlayingOffset = (delta) => {
    if (playingIndex < 0) return;
    const next = sortedFiltered[playingIndex + delta];
    if (next) { setPlaying(next); setPlayerMenuOpen(false); }
  };

  useEffect(() => { onFilteredCount?.(sortedFiltered.length); });

  // Escape clears the selection, Ctrl+A selects everything currently
  // filtered/sorted into view (not the whole library).
  useEffect(() => {
    const onKeyDown = (e) => {
      const inField = document.activeElement?.tagName === "INPUT" || document.activeElement?.tagName === "TEXTAREA";
      if (e.key === "Escape" && selectedNames.size > 0) setSelectedNames(new Set());
      if ((e.ctrlKey || e.metaKey) && (e.key === "a" || e.key === "A") && !inField) {
        e.preventDefault();
        setSelectedNames(new Set(sortedFiltered.map((v) => v.name)));
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selectedNames.size, sortedFiltered]);

  // Buckets the sorted list into named groups (day/month/year/app/source/tag).
  // "none" is a single unlabeled bucket so the flattening below stays one
  // code path either way.
  const groups = useMemo(() => {
    if (groupBy === "none" || !groupBy) return [{ label: null, items: sortedFiltered }];
    const dateLocale = lang === "tr" ? "tr-TR" : "en-US";
    const dayFmt = new Intl.DateTimeFormat(dateLocale, { day: "numeric", month: "long", year: "numeric" });
    const monthFmt = new Intl.DateTimeFormat(dateLocale, { month: "long", year: "numeric" });
    const tagMap = Object.fromEntries(tags.map((tg) => [tg.id, tg]));
    const map = new Map();
    for (const it of sortedFiltered) {
      const ms = it.modified;
      let keys;
      if (groupBy === "day") keys = [ms ? dayFmt.format(new Date(ms)) : t("gallery.groupLabels.unknown")];
      else if (groupBy === "month") keys = [ms ? monthFmt.format(new Date(ms)) : t("gallery.groupLabels.unknown")];
      else if (groupBy === "year") keys = [ms ? String(new Date(ms).getFullYear()) : t("gallery.groupLabels.unknown")];
      else if (groupBy === "app") keys = [it.app || t("gallery.groupLabels.general")];
      else if (groupBy === "tag") {
        const itTags = (it.tags || []).filter((id) => tagMap[id]);
        keys = itTags.length > 0 ? itTags.map((id) => tagMap[id].name) : [t("gallery.groupLabels.untagged")];
      } else { // source
        keys = [
          it.kind === "youtube_live" || it.kind === "youtube_only" ? "YouTube"
            : it.drive_only ? t("gallery.groupLabels.driveOnly")
            : it.drive_synced ? t("gallery.groupLabels.synced")
            : t("gallery.groupLabels.localOnly"),
        ];
      }
      for (const key of keys) {
        if (!map.has(key)) map.set(key, []);
        if (!map.get(key).includes(it)) map.get(key).push(it);
      }
    }
    return [...map.entries()].map(([label, items]) => ({ label, items }));
  }, [sortedFiltered, groupBy, lang, t, tags]);

  // Card geometry computed from the real container width so virtual rows
  // can be chunked and pre-sized, replacing a static CSS auto-fill grid.
  const GAP = 20; // gap-5
  const MIN_CARD_W = { "2xl": 500, xl: 340, large: 240, medium: 170, small: 110 }[viewMode] ?? 340;
  // "medium"/"small" have a fixed column count so they stay predictable
  // (medium 4 / small 5) instead of the min-width auto-fit packing in ever
  // more, ever-tinier cards on a wide window. Everything else — including
  // "large" — stays min-width responsive. "large" used to be fixed at 3
  // columns too, which at common container widths (~1100-1460px) happened
  // to compute to the same column count — and thus the identical card
  // size — as "xl"'s own responsive math, making the two modes look
  // indistinguishable. Its min-width (240, below "xl"'s 340) instead
  // guarantees at least as many columns as "xl" at any given width, so the
  // two only ever coincide in the same narrow low-width band any two
  // different min-widths inherently can.
  const FIXED_COLS = { medium: 4, small: 5 };
  // "medium"/"small" collapse the info panel to a title-only strip (see the
  // `[data-vm]` rules in styles.css) — `INFO_H` must match that CSS so the
  // virtualizer's row height stays exact. "large" keeps the full panel (size,
  // date+time, folder, etc.) like xl/2xl.
  const INFO_H = viewMode === "small" || viewMode === "medium" ? 44 : 76;
  const usableWidth = Math.max(0, containerWidth - GAP * 2); // p-5 on the scroll container
  const cols = FIXED_COLS[viewMode] ?? Math.max(1, Math.floor((usableWidth + GAP) / (MIN_CARD_W + GAP)));
  const cardW = usableWidth > 0 ? (usableWidth - (cols - 1) * GAP) / cols : MIN_CARD_W;
  // Rounded so integer slot heights keep every translateY offset integer;
  // the 20px inter-row gap swallows any sub-pixel thumb/slot mismatch.
  const cardRowH = Math.round(cardW * 9 / 16 /* aspect-video thumb */) + INFO_H;
  // Tile rows have no `vc-info` panel below them, so they're just the
  // aspect-video box without `INFO_H`.
  const tileRowH = Math.round(cardW * 9 / 16);

  // Flattens groups into header rows plus grid-chunk or list-item rows for the
  // virtualizer. Tiles get the same treatment, prepended ahead of other groups.
  const virtualRows = useMemo(() => {
    let firstHeader = true;
    const rows = [];
    // Skip the tiles section header entirely when there's nothing under it.
    if (tilesLabel && tiles.length > 0) {
      if (viewMode === "list") {
        // No "FOLDERS" header row in list view — the folder/game rows read as
        // folders on their own (icon + name), so the label is just noise.
        for (const tl of tiles) rows.push({ type: "tile-list-item", tile: tl });
      } else {
        rows.push({ type: "header", label: tilesLabel, count: tiles.length, isFirst: true });
        firstHeader = false;
        for (let i = 0; i < tiles.length; i += cols) {
          rows.push({ type: "tile-row", items: tiles.slice(i, i + cols) });
        }
      }
    }
    for (const { label, items: gItems } of groups) {
      if (label) { rows.push({ type: "header", label, count: gItems.length, isFirst: firstHeader }); firstHeader = false; }
      if (viewMode === "list") {
        for (const it of gItems) rows.push({ type: "list-item", it });
      } else {
        for (let i = 0; i < gItems.length; i += cols) {
          rows.push({ type: "grid-row", items: gItems.slice(i, i + cols) });
        }
      }
    }
    return rows;
  }, [groups, cols, viewMode, tiles, tilesLabel]);

  const rowHeight = (row) => {
    if (!row) return viewMode === "list" ? LIST_ROW_H : cardRowH + GAP;
    if (row.type === "header") return row.isFirst ? 28 : 48;
    if (row.type === "list-item" || row.type === "tile-list-item") return LIST_ROW_H;
    if (row.type === "tile-row") return tileRowH + GAP;
    return cardRowH + GAP;
  };

  // Re-bucketing rows on a view/filter change makes the old scroll offset land
  // somewhere unrelated, so snap to top. Not keyed on `tiles` — it's a fresh array
  // every render and would reset scroll on every background poll.
  useEffect(() => {
    if (scrollRef.current) scrollRef.current.scrollTop = 0;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [viewMode, groupBy, sortBy, tagFilter, kindFilter, favoritesOnly, selectedGame, selectedFolderId]);

  // Rows are never measured — every row type has a deterministic height, so
  // `estimateSize` IS the layout.
  const virtualizer = useVirtualizer({
    count: virtualRows.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: (i) => rowHeight(virtualRows[i]),
    overscan: 3,
  });
  // Row heights/boundaries shift whenever the grouping, column count,
  // container width (cardRowH), or filtered set changes — flush the cached
  // sizes so positions are recomputed from the new estimates.
  useEffect(() => { virtualizer.measure(); }, [virtualRows, cardRowH]); // eslint-disable-line react-hooks/exhaustive-deps

  // Scrolls to and briefly rings `highlightName`'s card; re-checks on every
  // `virtualRows` change since the target row isn't ready immediately.
  useEffect(() => {
    if (!highlightName) return;
    const idx = virtualRows.findIndex((row) =>
      row.type === "list-item" ? row.it.name === highlightName
      : row.type === "grid-row" ? row.items.some((v) => v.name === highlightName)
      : false
    );
    if (idx === -1) return;
    virtualizer.scrollToIndex(idx, { align: "center" });
    setHighlightedName(highlightName);
    onHighlightDone?.();
    const timer = setTimeout(() => setHighlightedName(null), 1600);
    return () => clearTimeout(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [highlightName, virtualRows]);

  // Scrubber sections, one per group header with its pixel start offset. Stays in
  // pixel space end to end since row heights are uneven (28/48px headers vs ~300px cards).
  const scrubberSections = useMemo(() => {
    if (groupBy === "none" || !groupBy || !virtualRows.length) return [];
    const sections = [];
    let offset = 0;
    for (const row of virtualRows) {
      if (row.type === "header") sections.push({ label: row.label, count: row.count, startPx: offset });
      offset += rowHeight(row);
    }
    return sections;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [virtualRows, groupBy, cardRowH]);

  // Year boundary markers along the scrubber track (day/month/year grouping
  // only). Same pixel space as the thumb, so a marker aligns with the
  // thumb-top when that section reaches the viewport top.
  const yearMarkers = useMemo(() => {
    if ((groupBy !== "day" && groupBy !== "month" && groupBy !== "year") || scrubberSections.length <= 1) return [];
    const denom = Math.max(1, scrollMetrics.scrollH - scrollMetrics.clientH);
    const markers = [];
    let lastYear = null;
    let lastTopPct = -Infinity;
    for (const sec of scrubberSections) {
      const m = sec.label.match(/\b(19|20)\d{2}\b/);
      if (!m) continue;
      const year = m[0];
      const topPct = Math.min(100, (sec.startPx / denom) * 100);
      if (year !== lastYear && topPct - lastTopPct >= 4) {
        markers.push({ year, topPct });
        lastYear = year;
        lastTopPct = topPct;
      }
    }
    return markers;
  }, [scrubberSections, groupBy, scrollMetrics]);

  // Keep a ref current so the drag closure (set up once per mouse-down)
  // always reads the latest sections without being re-created every render.
  scrubberSectionsRef.current = scrubberSections;

  const handleScrubMouseDown = (e) => {
    e.preventDefault();
    e.stopPropagation();
    const track = scrubTrackRef.current;
    if (!track) return;
    scrubTrackRectRef.current = track.getBoundingClientRect();
    setScrubDragging(true);

    const updateFromY = (clientY) => {
      const el = scrollRef.current;
      if (!el) return;
      const rect = track.getBoundingClientRect();
      const ratio = Math.max(0, Math.min(1, (clientY - rect.top) / rect.height));
      // Straight pixel mapping — handleScroll recomputes this same ratio, so
      // the thumb never fights the cursor.
      const targetTop = ratio * (el.scrollHeight - el.clientHeight);
      el.scrollTop = targetTop;
      setScrollRatio(ratio);
      const sections = scrubberSectionsRef.current;
      let label = sections.length ? sections[0].label : "";
      for (const sec of sections) {
        if (sec.startPx <= targetTop) label = sec.label;
        else break;
      }
      setScrubLabelText(label);
    };

    updateFromY(e.clientY);
    const onMove = (ev) => updateFromY(ev.clientY);
    const onUp = () => {
      setScrubDragging(false);
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
    };
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
  };

  const showScrubber = groupBy !== "none" && scrubberSections.length > 1;

  return (
    <div className="flex flex-1 min-h-0">
      <div ref={scrollRef} onScroll={handleScroll} onMouseDown={handleMouseDown}
        className={`flex-1 min-h-0 overflow-y-auto p-5 select-none ${showScrubber ? "no-scrollbar" : ""}`}>
        {videos === null ? (
          <div>
            <div className="mb-4 flex flex-wrap items-center gap-x-5 gap-y-1.5 text-xs text-stone-500">
              <span className="flex items-center gap-1.5">
                <span className="h-1.5 w-1.5 rounded-full bg-emerald-400" />
                {t("gallery.loadingSteps.local")}
              </span>
              {driveConnected && (
                <span className="flex items-center gap-1.5">
                  <span className="h-3 w-3 animate-spin rounded-full border-2 border-stone-700 border-t-accent-400" />
                  {t("gallery.loadingSteps.drive")}
                  {": "}
                  {driveScanProgress
                    ? `${t("gallery.loadingSteps.scanning")} ${t("gallery.loadingSteps.items")(driveScanProgress.files, lang)}${
                        driveScanProgress.page > 1 ? t("gallery.loadingSteps.page")(driveScanProgress.page) : ""
                      }`
                    : t("gallery.loadingSteps.connecting")}
                </span>
              )}
            </div>
            {/* Skeleton rows/cards sized/spaced like the real thing below, so
                the eventual swap-in doesn't shift anything. */}
            {viewMode === "list" ? (
              <div>
                {Array.from({ length: 10 }).map((_, i) => (
                  <div key={i} style={{ display: "grid", gridTemplateColumns: LIST_COLS, height: LIST_ROW_H }} className="items-center gap-3 border-b border-stone-800/40 px-3">
                    <div className="skeleton-shimmer h-7 w-7 rounded" />
                    <div className="skeleton-shimmer h-3 w-2/3 rounded" />
                    <div className="skeleton-shimmer h-3 w-1/2 rounded" />
                    <div className="skeleton-shimmer h-3 w-3/4 rounded" />
                    <div className="skeleton-shimmer h-3 w-1/2 justify-self-end rounded" />
                    <div className="skeleton-shimmer h-3 w-1/2 justify-self-center rounded" />
                    <div />
                  </div>
                ))}
              </div>
            ) : (
              <div className="grid" style={{ gridTemplateColumns: `repeat(${cols}, minmax(0, 1fr))`, gap: GAP }}>
                {Array.from({ length: cols * 3 }).map((_, i) => (
                  <div key={i} className="overflow-hidden rounded-xl bg-stone-900">
                    <div className="skeleton-shimmer aspect-video" />
                    <div className="flex flex-col justify-center gap-2 px-3" style={{ height: INFO_H }}>
                      <div className="skeleton-shimmer h-3 w-3/4 rounded" />
                      <div className="skeleton-shimmer h-2.5 w-1/2 rounded" />
                    </div>
                  </div>
                ))}
              </div>
            )}
          </div>
        ) : sortedFiltered.length === 0 && tiles.length === 0 ? (
          <div className="flex items-center justify-center h-40 text-stone-600 text-sm">
            {videos.length === 0 ? t("gallery.video.empty") : t("gallery.noMatch")}
          </div>
        ) : (
          <>
            {/* List mode's column header: rendered once outside the
                virtualizer's rows, so plain `position: sticky` works fine here. */}
            {viewMode === "list" && (
              <div
                style={{ display: "grid", gridTemplateColumns: LIST_COLS }}
                className="sticky top-0 z-10 gap-3 border-b border-stone-800 bg-stone-950/95 px-3 py-2 text-[10px] font-semibold uppercase tracking-widest text-stone-600 backdrop-blur-sm"
              >
                <div />
                <div />
                <ListHeaderCell label={t("gallery.listHeader.title")} field="title" sortBy={sortBy} onSortChange={onSortChange} />
                <ListHeaderCell label={t("gallery.listHeader.app")} field="app" sortBy={sortBy} onSortChange={onSortChange} />
                <ListHeaderCell label={t("gallery.listHeader.date")} field="date" sortBy={sortBy} onSortChange={onSortChange} />
                <ListHeaderCell label={t("gallery.listHeader.size")} field="size" align="right" sortBy={sortBy} onSortChange={onSortChange} />
                <div className="text-center">{t("gallery.listHeader.source")}</div>
                <div />
              </div>
            )}
            <div style={{ height: virtualizer.getTotalSize(), position: "relative" }}>
              {virtualizer.getVirtualItems().map((vRow) => {
                const row = virtualRows[vRow.index];
                return (
                  <div
                    key={vRow.key}
                    style={{ position: "absolute", top: 0, left: 0, right: 0, height: vRow.size, transform: `translateY(${vRow.start}px)` }}
                  >
                    {row.type === "header" ? (
                      // Fixed height to match `rowHeight()`'s estimate exactly, avoiding scroll stutter.
                      <div className={`flex items-center gap-3 px-0.5 pb-2 ${row.isFirst ? "h-7 pt-0" : "h-12 pt-5"}`}>
                        <span className="text-[11px] font-bold uppercase tracking-widest text-stone-400">{row.label}</span>
                        <div className="flex-1 border-t border-stone-800/60" />
                        <span className="text-[10px] text-stone-600">{viewMode === "list" ? row.count : t("gallery.itemCount")(row.count, null)}</span>
                      </div>
                    ) : row.type === "list-item" ? (
                      <VideoListRow video={row.it} t={t} lang={lang} tags={tags}
                        onPlay={setPlaying} onDelete={handleDelete} onDeleteDriveCopy={handleDeleteDriveCopy} onDeleteBoth={handleDeleteBoth} onToggleTag={handleToggleTag}
                        onToggleFavorite={handleToggleFavorite}
                        onUploadYoutube={openYoutubeUpload} onUploadDrive={handleUploadDrive}
                        transfer={transfers.get(row.it.name)}
                        driveConnected={driveConnected} driveEmail={driveEmail} youtubeChannelId={youtubeChannelId} youtubeEmail={youtubeEmail} onEdit={onEdit}
                        isSelected={selectedNames.has(row.it.name)} hasSelection={selectedNames.size > 0}
                        bulkDeleting={deletingNames.has(row.it.name)}
                        onToggleSelect={toggleSelectItem} onSelectOnly={selectOnly}
                        onShowInFolderView={rootOnly ? undefined : () => onShowInFolderView?.(row.it)}
                        onShowDetails={setDetailsVideo}
                        onCopyDriveLink={(v) => copyDriveLink(v, () => showToast(t("common.copiedToClipboard")))}
                        onBulkContextMenu={selectedNames.size > 1 && selectedNames.has(row.it.name) ? requestBulkMenu : undefined}
                        highlighted={row.it.name === highlightedName} />
                    ) : row.type === "tile-list-item" ? (
                      <TileListRow tile={row.tile} onOpen={onOpenTile} onContextMenu={onTileContextMenu} t={t} />
                    ) : row.type === "tile-row" ? (
                      <div className="grid" style={{ gridTemplateColumns: `repeat(${cols}, minmax(0, 1fr))`, gap: GAP, paddingBottom: GAP }}>
                        {row.items.map((tl) => (
                          <TileCard key={tl.id ?? tl.name} tile={tl} onOpen={onOpenTile} onContextMenu={onTileContextMenu} t={t} />
                        ))}
                      </div>
                    ) : (
                      <div className="grid" style={{ gridTemplateColumns: `repeat(${cols}, minmax(0, 1fr))`, gap: GAP, paddingBottom: GAP }}>
                        {row.items.map((v) => (
                          <VideoCard key={v.name} video={v} t={t} lang={lang} tags={tags} viewMode={viewMode}
                            onPlay={setPlaying} onDelete={handleDelete} onDeleteDriveCopy={handleDeleteDriveCopy} onDeleteBoth={handleDeleteBoth} onToggleTag={handleToggleTag}
                            onToggleFavorite={handleToggleFavorite}
                            onUploadYoutube={openYoutubeUpload} onUploadDrive={handleUploadDrive}
                            transfer={transfers.get(v.name)}
                            driveConnected={driveConnected} driveEmail={driveEmail} youtubeChannelId={youtubeChannelId} youtubeEmail={youtubeEmail} onEdit={onEdit}
                            isSelected={selectedNames.has(v.name)} hasSelection={selectedNames.size > 0}
                            bulkDeleting={deletingNames.has(v.name)}
                            onToggleSelect={toggleSelectItem} onSelectOnly={selectOnly}
                            onShowInFolderView={rootOnly ? undefined : () => onShowInFolderView?.(v)}
                            onShowDetails={setDetailsVideo}
                            onCopyDriveLink={(v2) => copyDriveLink(v2, () => showToast(t("common.copiedToClipboard")))}
                            onBulkContextMenu={selectedNames.size > 1 && selectedNames.has(v.name) ? requestBulkMenu : undefined}
                            highlighted={v.name === highlightedName} />
                        ))}
                      </div>
                    )}
                  </div>
                );
              })}
            </div>
          </>
        )}

        {playing && (
          // `absolute` (not `fixed`) so `inset-0` bounds against the content
          // area below the titlebar, not the whole viewport — `fixed` could
          // let a tall modal poke up underneath the titlebar in a short window.
          // `data-player-modal` is what `handleMouseDown`'s marquee-select
          // guard below matches on, since this isn't `.fixed` like the
          // other overlays it also skips.
          <div data-player-modal className="absolute inset-0 z-50 bg-black/90 flex items-center justify-center" onClick={() => setPlaying(null)}>
            {playingIndex > 0 && (
              <button
                onClick={(e) => { e.stopPropagation(); goToPlayingOffset(-1); }}
                title={t("gallery.player.previous")}
                className="absolute left-3 top-1/2 z-10 flex h-11 w-11 -translate-y-1/2 items-center justify-center rounded-full bg-black/50 text-white transition hover:bg-black/70"
              >
                <Icon.ChevronLeft size={22} />
              </button>
            )}
            {playingIndex >= 0 && playingIndex < sortedFiltered.length - 1 && (
              <button
                onClick={(e) => { e.stopPropagation(); goToPlayingOffset(1); }}
                title={t("gallery.player.next")}
                className="absolute right-3 top-1/2 z-10 flex h-11 w-11 -translate-y-1/2 items-center justify-center rounded-full bg-black/50 text-white transition hover:bg-black/70"
              >
                <Icon.ChevronRight size={22} />
              </button>
            )}
            <div
              onClick={(e) => e.stopPropagation()}
              className="flex h-full w-full flex-col overflow-hidden bg-stone-900"
            >
              <PlayerInfoHeader video={playing} tags={tags} t={t} lang={lang} onClose={() => setPlaying(null)} />
              {playing.kind === "recording" || playing.kind === "recording_finishing" ? (
                <LiveRecordingPlayer
                  src={convertFileSrc(playing.local_path)}
                  autoPlay
                  onClose={() => setPlaying(null)}
                  t={t}
                  recordingStartedAt={playing.recordingStartedAt}
                  frozenSecs={playing.frozenSecs}
                  className="w-full"
                />
              ) : playing.drive_only ? (
                // `drive_only`'s own `local_path` is Drive's web-view link
                // (or "" if Drive didn't return one) — never a real local
                // path, so `drive_only` alone is the only reliable signal
                // here; checking `!local_path` too let a populated
                // web-view-link fall through to the real `<VideoPlayer>`
                // below with a Drive URL as its "local" src.
                <div className="relative flex w-full flex-1 min-h-0 flex-col">
                  <DriveOnlyPlaceholder
                    video={playing}
                    t={t}
                    transfer={transfers.get(playing.name)}
                    onDownloaded={(localPath) => setPlaying({ ...playing, local_path: localPath, drive_only: false })}
                  />
                  <button
                    onClick={(e) => {
                      e.stopPropagation();
                      if (playerMenuOpen) { setPlayerMenuOpen(false); return; }
                      setPlayerMenuPos(kebabMenuPos(e.currentTarget.getBoundingClientRect()));
                      setPlayerMenuOpen(true);
                    }}
                    className="absolute bottom-3 right-3 flex h-8 w-8 items-center justify-center rounded-full text-white/80 transition hover:bg-white/10 hover:text-white"
                  >
                    <Icon.MoreVertical size={16} />
                  </button>
                </div>
              ) : (
                <VideoPlayer
                  src={convertFileSrc(playing.local_path)}
                  name={playing.name}
                  path={playing.local_path}
                  t={t}
                  autoPlay
                  onClose={() => setPlaying(null)}
                  onClipSaved={(clip) => setPlaying({ local_path: clip.path, name: clip.name, app: clip.app, kind: "clip" })}
                  className="w-full flex-1 min-h-0"
                  extraControls={
                    <button
                      onClick={(e) => {
                        e.stopPropagation();
                        if (playerMenuOpen) { setPlayerMenuOpen(false); return; }
                        setPlayerMenuPos(kebabMenuPos(e.currentTarget.getBoundingClientRect()));
                        setPlayerMenuOpen(true);
                      }}
                      className="flex h-8 w-8 items-center justify-center rounded-full text-white/80 transition hover:bg-white/10 hover:text-white"
                    >
                      <Icon.MoreVertical size={16} />
                    </button>
                  }
                />
              )}
            </div>
          </div>
        )}

        {playerMenuOpen && playing && (
          <CardMenu
            t={t}
            tags={tags}
            assigned={playing.tags || []}
            position={playerMenuPos}
            onToggleTag={(tagId) => handleToggleTag(playing, tagId)}
            onUploadYoutube={() => { setPlayerMenuOpen(false); openYoutubeUpload(playing); }}
            onOpenYoutube={playing.youtube_video_id ? () => { setPlayerMenuOpen(false); openYoutubeVideo(playing.youtube_video_id); } : undefined}
            onOpenYoutubeStudio={playing.youtube_video_id ? () => { setPlayerMenuOpen(false); openYoutubeStudio(playing.youtube_video_id, youtubeEmail); } : undefined}
            onCreateYoutubeClip={playing.youtube_video_id ? () => { setPlayerMenuOpen(false); openYoutubeClip(playing.youtube_video_id, youtubeEmail); } : undefined}
            onUploadDrive={() => { setPlayerMenuOpen(false); handleUploadDrive(playing); }}
            onCopyDriveLink={() => { setPlayerMenuOpen(false); copyDriveLink(playing, () => showToast(t("common.copiedToClipboard"))); }}
            onOpenDrive={playing.drive_synced && playing.drive_id ? () => { setPlayerMenuOpen(false); openDriveFile(playing.drive_id, driveEmail); } : undefined}
            driveConnected={driveConnected}
            driveSynced={playing.drive_synced}
            driveOnly={!!playing.drive_only}
            backedUp={!!playing.youtube_video_id || playing.drive_synced}
            hasLocalFile={!(playing.kind === "youtube_live" || playing.kind === "youtube_only") && !playing.drive_only}
            onReveal={() => { setPlayerMenuOpen(false); invoke("reveal_item", { path: playing.local_path }).catch(() => {}); }}
            onShowInFolderView={!rootOnly && onShowInFolderView ? () => { setPlayerMenuOpen(false); setPlaying(null); onShowInFolderView(playing); } : undefined}
            onShowDetails={() => { setPlayerMenuOpen(false); setDetailsVideo(playing); }}
            onDelete={() => { setPlayerMenuOpen(false); setPlaying(null); handleDelete(playing, false); }}
            onDeleteDriveCopy={() => { setPlayerMenuOpen(false); handleDeleteDriveCopy(playing); }}
            onDeleteBoth={() => { setPlayerMenuOpen(false); setPlaying(null); handleDeleteBoth(playing, false); }}
            onClose={() => setPlayerMenuOpen(false)}
          />
        )}

        {youtubeTarget && (
          <YouTubeUploadModal
            t={t}
            path={youtubeTarget.local_path}
            defaultTitle={youtubeTarget.title || fileNameFromPath(youtubeTarget.local_path)}
            connected={driveConnected}
            onOpenSettings={() => invoke("open_settings")}
            onClose={() => setYoutubeTarget(null)}
          />
        )}

        {detailsVideo && (
          <VideoDetailsModal video={detailsVideo} tags={tags} t={t} lang={lang} onClose={() => setDetailsVideo(null)} />
        )}

        {toastMsg && (
          <div className="pointer-events-none fixed bottom-6 right-6 z-40 animate-fade-in rounded-full border border-stone-700/60 bg-stone-900/95 px-4 py-2 text-xs font-medium text-stone-200 shadow-2xl backdrop-blur-md">
            {toastMsg}
          </div>
        )}
      </div>

      {/* Group scrubber: drag anywhere on the track to jump to that fraction
          of the list; while dragging, year markers and the current
          section's label float next to it. */}
      {showScrubber && (() => {
        const viewFrac = scrollMetrics.scrollH > scrollMetrics.clientH
          ? scrollMetrics.clientH / scrollMetrics.scrollH : 1;
        const thumbPct = Math.max(4, viewFrac * 100);
        const thumbTopPct = scrollRatio * (100 - thumbPct);
        return (
          <div
            ref={scrubTrackRef}
            className="relative my-5 mr-2 w-5 shrink-0 cursor-pointer select-none"
            onMouseDown={handleScrubMouseDown}
          >
            <div className="absolute inset-y-2 left-1/2 w-[3px] -translate-x-1/2 rounded-full bg-stone-800/60" />
            {scrubDragging && scrubTrackRectRef.current && yearMarkers.map(({ year, topPct }) => {
              const tr = scrubTrackRectRef.current;
              return (
                <div
                  key={year}
                  className="pointer-events-none whitespace-nowrap rounded-md border border-stone-700 bg-stone-800/95 px-2.5 py-0.5 text-[11px] font-bold text-stone-200 shadow-lg"
                  style={{
                    position: "fixed",
                    top: tr.top + (topPct / 100) * tr.height,
                    right: window.innerWidth - tr.left + 10,
                    transform: "translateY(-50%)",
                    zIndex: 200,
                  }}
                >
                  {year}
                </div>
              );
            })}
            <div
              className={`pointer-events-none absolute left-1/2 -translate-x-1/2 rounded-full transition-[width,background-color] ${
                scrubDragging ? "w-[8px] bg-accent-400" : "w-[6px] bg-stone-600"
              }`}
              style={{ top: `${thumbTopPct}%`, height: `${thumbPct}%` }}
            />
            {scrubDragging && scrubLabelText && scrubTrackRectRef.current && (() => {
              const tr = scrubTrackRectRef.current;
              const thumbMidPct = thumbTopPct + thumbPct / 2;
              return (
                <div
                  className="pointer-events-none whitespace-nowrap rounded-lg border border-stone-600 bg-stone-900 px-3 py-1.5 text-[12px] font-semibold text-stone-100 shadow-xl"
                  style={{
                    position: "fixed",
                    top: tr.top + (thumbMidPct / 100) * tr.height - 14,
                    right: window.innerWidth - tr.left + 10,
                    zIndex: 300,
                  }}
                >
                  {scrubLabelText}
                </div>
              );
            })()}
          </div>
        );
      })()}

      {/* Drag-select rectangle — portaled to <body> like CardMenu: the app
          root's mount animation leaves a lingering `transform` that gives
          `position: fixed` descendants the wrong containing block, which
          would visibly misplace this box's exact pixel coordinates. */}
      {isDragging && dragStart && dragEnd && createPortal(
        (() => {
          const anchoredStartY = dragStart.y - (scrollTopRef.current - dragStartScrollTopRef.current);
          const bounds = scrollRef.current?.getBoundingClientRect();
          let left = Math.min(dragStart.x, dragEnd.x);
          let right = Math.max(dragStart.x, dragEnd.x);
          let top = Math.min(anchoredStartY, dragEnd.y);
          let bottom = Math.max(anchoredStartY, dragEnd.y);
          if (bounds) {
            left = Math.max(left, bounds.left);
            right = Math.min(right, bounds.right);
            top = Math.max(top, bounds.top);
            bottom = Math.min(bottom, bounds.bottom);
          }
          return (
            <div
              className="pointer-events-none fixed z-[60] rounded border border-accent-400/85 bg-accent-400/15"
              style={{ left, top, width: Math.max(0, right - left), height: Math.max(0, bottom - top) }}
            />
          );
        })(),
        document.body
      )}

      {bulkMenu && (() => {
        const sel = selectedVideos();
        return (
          <BulkMenu
            t={t}
            position={bulkMenu}
            count={selectedNames.size}
            canUpload={driveConnected && sel.some((v) => v.local_path && !v.drive_only && !v.drive_synced)}
            canDownload={sel.some((v) => v.drive_only)}
            canRemoveDrive={sel.some((v) => v.drive_synced && !v.drive_only)}
            canDeleteLocal={sel.some((v) => v.local_path && !v.drive_only)}
            onUpload={() => { setBulkMenu(null); uploadSelected(); }}
            onDownload={() => { setBulkMenu(null); downloadSelected(); }}
            onRemoveDrive={() => { setBulkMenu(null); removeDriveCopySelected(); }}
            onDeleteLocal={() => { setBulkMenu(null); deleteLocalSelected(); }}
            onDelete={() => { setBulkMenu(null); deleteSelected(); }}
            onClose={() => setBulkMenu(null)}
          />
        );
      })()}

      {/* Floating multi-selection action bar */}
      {selectedNames.size > 0 && (
        <div className="fixed bottom-6 left-1/2 z-30 flex -translate-x-1/2 items-center gap-4 rounded-full border border-stone-700/60 bg-stone-900/95 px-6 py-2.5 shadow-2xl backdrop-blur-md">
          <div className="flex items-center gap-2 border-r border-stone-800 pr-4">
            <button onClick={() => setSelectedNames(new Set())} title={t("gallery.selection.clearTitle")}
              className="flex h-5 w-5 items-center justify-center rounded-full text-stone-400 transition hover:bg-stone-800 hover:text-white">
              <Icon.X size={12} />
            </button>
            <span className="text-xs font-semibold text-stone-200">{t("gallery.selection.selected")(selectedNames.size)}</span>
          </div>
          <div className="flex items-center gap-1.5">
            {selectedVideos().some((v) => v.local_path && !v.drive_only && !v.drive_synced) && driveConnected && (
              <button onClick={uploadSelected} className="flex items-center gap-1.5 rounded-full bg-stone-800 px-3 py-1.5 text-xs font-medium text-stone-200 transition hover:bg-stone-700">
                <Icon.Upload size={13} /> {t("gallery.selection.uploadToDrive")}
              </button>
            )}
            {selectedVideos().some((v) => v.drive_only) && (
              <button onClick={downloadSelected} className="flex items-center gap-1.5 rounded-full bg-accent-500/20 px-3 py-1.5 text-xs font-medium text-accent-300 transition hover:bg-accent-500/30">
                <Icon.Check size={13} /> {t("gallery.selection.download")}
              </button>
            )}
            {selectedVideos().some((v) => v.drive_synced && !v.drive_only) && (
              <button onClick={removeDriveCopySelected} className="flex items-center gap-1.5 rounded-full bg-stone-800 px-3 py-1.5 text-xs font-medium text-stone-200 transition hover:bg-stone-700">
                <Icon.Cloud size={13} /> {t("gallery.menu.removeDriveCopy")}
              </button>
            )}
            <button onClick={deleteSelected} className="flex items-center gap-1.5 rounded-full bg-red-500/15 px-3 py-1.5 text-xs font-medium text-red-400 transition hover:bg-red-500/25">
              <Icon.Trash size={13} /> {t("gallery.selection.delete")}
            </button>
          </div>
        </div>
      )}

      {confirm && (
        <div className="fixed inset-0 z-[70] flex items-center justify-center bg-black/60 p-6" onClick={() => setConfirm(null)}>
          <div className="w-full max-w-sm rounded-xl border border-stone-800 bg-stone-900 p-5" onClick={(e) => e.stopPropagation()}>
            <div className="mb-1 text-sm font-semibold text-stone-100">{t("gallery.confirm.title")}</div>
            <div className="mb-4 text-[13px] text-stone-400">{confirm.message}</div>
            <div className="flex justify-end gap-2">
              <button onClick={() => setConfirm(null)}
                className="rounded-lg px-3.5 py-1.5 text-[13px] font-medium text-stone-300 transition hover:bg-stone-800">
                {t("gallery.confirm.cancel")}
              </button>
              <button
                onClick={async () => { const action = confirm.action; setConfirm(null); await action(); }}
                className="rounded-lg bg-red-500/15 px-3.5 py-1.5 text-[13px] font-medium text-red-400 transition hover:bg-red-500/25">
                {t("gallery.confirm.delete")}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
