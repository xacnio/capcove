import { useEffect, useState } from "react";
import { createPortal } from "react-dom";
import { MdRefresh } from "react-icons/md";
import { invoke, listen, emit } from "../lib/tauri.js";
import { Toggle, Row, Card, Button, inputCls } from "./settingsUI.jsx";
import { relativeTime } from "../lib/relativeTime.js";

function fmtBytes(bytes) {
  if (bytes == null) return "";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let v = bytes, i = 0;
  while (v >= 1024 && i < units.length - 1) { v /= 1024; i++; }
  return `${v.toFixed(i === 0 ? 0 : 2)} ${units[i]}`;
}

// In MB throughout (matches `storage_limit_mb`) — the sub-1GB steps exist
// mainly to make the auto-delete-when-over-limit behavior easy to trigger
// and verify without needing gigabytes of real recordings on disk, up
// through multi-TB for the drives a video library actually needs.
const LIMIT_PRESETS_MB = [
  100, 250, 500, 750,
  5 * 1024, 10 * 1024, 25 * 1024, 50 * 1024, 100 * 1024, 250 * 1024, 500 * 1024, 750 * 1024,
  1000 * 1024, 2000 * 1024, 5000 * 1024,
];

function formatLimitOption(mb, t) {
  if (mb < 1024) return t("settings.storage.mb")(mb);
  const gb = mb / 1024;
  if (gb < 1000) return t("settings.storage.gb")(Number.isInteger(gb) ? gb : gb.toFixed(1));
  const tb = gb / 1000;
  return t("settings.storage.tb")(Number.isInteger(tb) ? tb : tb.toFixed(1));
}

function driveLetterFromPath(path) {
  const m = /^([A-Za-z]):/.exec(path || "");
  return m ? m[1].toUpperCase() : null;
}

// The storage-limit/folder-age auto-delete otherwise only leaves a line in
// the app log — this makes it visible and verifiable from Settings itself.
function DeletionLogCard({ t, lang }) {
  const [entries, setEntries] = useState(null);
  const [clearing, setClearing] = useState(false);

  useEffect(() => {
    const load = () => invoke("get_deletion_log").then(setEntries).catch(() => setEntries([]));
    load();
    let unlisten = [];
    (async () => {
      unlisten.push(await listen("video-saved", load));
      unlisten.push(await listen("library-changed", load));
    })();
    return () => unlisten.forEach((u) => u());
  }, []);

  const clear = async () => {
    setClearing(true);
    try { await invoke("clear_deletion_log"); setEntries([]); }
    finally { setClearing(false); }
  };

  return (
    <Card title={t("settings.storage.deletionLog.title")}
      right={entries?.length > 0 && (
        <button onClick={clear} disabled={clearing}
          className="text-xs text-stone-500 transition hover:text-red-400 disabled:opacity-50">
          {t("settings.storage.deletionLog.clear")}
        </button>
      )}
    >
      <div className="py-2">
        {!entries ? (
          <div className="animate-pulse py-1 text-xs text-stone-700">{t("common.loading")}</div>
        ) : entries.length === 0 ? (
          <div className="py-2 text-xs text-stone-600">{t("settings.storage.deletionLog.empty")}</div>
        ) : (
          <div className="flex max-h-56 flex-col divide-y divide-stone-800/70 overflow-y-auto">
            {entries.map((entry, i) => (
              <div key={`${entry.name}-${entry.deleted_at}-${i}`} className="flex items-center justify-between gap-3 py-2">
                <div className="min-w-0">
                  <div className="truncate text-xs text-stone-300" title={entry.name}>
                    {entry.name.split("/").pop()}
                  </div>
                  <div className="mt-0.5 truncate text-[11px] text-stone-500">
                    {entry.reason.kind === "storage_limit"
                      ? t("settings.storage.deletionLog.reasonStorageLimit")
                      : t("settings.storage.deletionLog.reasonFolderAge")(entry.reason.folder, entry.reason.days)}
                  </div>
                </div>
                <div className="shrink-0 text-right text-[11px] text-stone-500">
                  <div>{fmtBytes(entry.size)}</div>
                  <div>{relativeTime(entry.deleted_at * 1000, lang)}</div>
                </div>
              </div>
            ))}
          </div>
        )}
      </div>
    </Card>
  );
}

// Fixed display order for the cache categories (backend returns them in an
// arbitrary map order); "unused" last since it's the special orphan-only one.
const CACHE_ORDER = ["video_thumbs", "waveforms", "playable", "drive_thumbs", "editor_previews", "icons", "unused"];

// Cache cleanup + "free up disk space" (delete Drive-backed local copies).
// Its own card so the heavier per-category / reclaimable-file scans only run
// when this section is on screen, and refresh independently of the main
// storage numbers.
function DataCleanupCard({ t, onChanged }) {
  const [cats, setCats] = useState(null);       // [{id, bytes}] | null
  const [selected, setSelected] = useState(new Set());
  const [clearing, setClearing] = useState(false);
  const [reclaim, setReclaim] = useState(null); // [{name, size}] | null
  const [deleting, setDeleting] = useState(false);
  const [confirm, setConfirm] = useState(false);
  const [toast, setToast] = useState(null);

  const loadCats = () => invoke("get_cache_breakdown").then(setCats).catch(() => setCats([]));
  const loadReclaim = () => invoke("get_reclaimable_files").then(setReclaim).catch(() => setReclaim([]));

  useEffect(() => {
    loadCats();
    loadReclaim();
    let unlisten = [];
    (async () => {
      unlisten.push(await listen("video-saved", () => { loadCats(); loadReclaim(); }));
      unlisten.push(await listen("library-changed", () => { loadCats(); loadReclaim(); }));
    })();
    return () => unlisten.forEach((u) => u());
  }, []);

  const flashToast = (msg) => { setToast(msg); setTimeout(() => setToast(null), 2500); };

  const clearable = (cats ?? []).filter((c) => c.bytes > 0);
  const toggle = (id) => setSelected((s) => { const n = new Set(s); n.has(id) ? n.delete(id) : n.add(id); return n; });
  const allSelected = clearable.length > 0 && clearable.every((c) => selected.has(c.id));
  const toggleAll = () => setSelected(allSelected ? new Set() : new Set(clearable.map((c) => c.id)));
  const selectedBytes = clearable.filter((c) => selected.has(c.id)).reduce((s, c) => s + c.bytes, 0);

  const clearSelected = async () => {
    if (selected.size === 0) return;
    setClearing(true);
    try {
      const freed = await invoke("clear_cache_categories", { categories: [...selected] }).catch(() => 0);
      flashToast(t("settings.storage.cleanup.freedToast")(fmtBytes(freed)));
      setSelected(new Set());
      loadCats();
      onChanged?.();
    } finally { setClearing(false); }
  };

  const reclaimBytes = (reclaim ?? []).reduce((s, r) => s + r.size, 0);
  const deleteLocalCopies = async () => {
    setConfirm(false);
    setDeleting(true);
    try {
      const freed = await invoke("delete_local_copies", { names: (reclaim ?? []).map((r) => r.name) }).catch(() => 0);
      flashToast(t("settings.storage.cleanup.freedToast")(fmtBytes(freed)));
      loadReclaim();
      onChanged?.();
    } finally { setDeleting(false); }
  };

  return (
    <Card title={t("settings.storage.cleanup.title")}>
      {/* Caches */}
      <div className="py-3">
        <div className="mb-1 flex items-center justify-between gap-3">
          <span className="text-sm text-stone-200">{t("settings.storage.cleanup.cacheTitle")}</span>
          {clearable.length > 0 && (
            <button onClick={toggleAll} className="text-[11px] text-stone-500 transition hover:text-stone-300">
              {t("settings.storage.cleanup.selectAll")}
            </button>
          )}
        </div>
        <div className="mb-2 text-[11px] text-stone-500">{t("settings.storage.cleanup.cacheHint")}</div>
        {!cats ? (
          <div className="animate-pulse py-1 text-xs text-stone-700">{t("common.loading")}</div>
        ) : clearable.length === 0 ? (
          <div className="py-1 text-xs text-stone-600">{t("settings.storage.cleanup.empty")}</div>
        ) : (
          <div className="flex flex-col divide-y divide-stone-800/70">
            {CACHE_ORDER.filter((id) => clearable.some((c) => c.id === id)).map((id) => {
              const c = clearable.find((x) => x.id === id);
              const hint = t(`settings.storage.cleanup.categoryHints.${id}`, "");
              return (
                <label key={id} className="flex cursor-pointer items-center gap-3 py-2">
                  <input type="checkbox" checked={selected.has(id)} onChange={() => toggle(id)}
                    className="h-4 w-4 shrink-0 cursor-pointer accent-accent-500" />
                  <div className="min-w-0 flex-1">
                    <div className="text-[13px] text-stone-200">{t(`settings.storage.cleanup.categories.${id}`)}</div>
                    {hint && hint !== `settings.storage.cleanup.categoryHints.${id}` && (
                      <div className="mt-0.5 text-[11px] text-stone-500">{hint}</div>
                    )}
                  </div>
                  <span className="shrink-0 text-xs tabular-nums text-stone-400">{fmtBytes(c.bytes)}</span>
                </label>
              );
            })}
          </div>
        )}
        {clearable.length > 0 && (
          <div className="mt-3 flex justify-end">
            <Button variant="danger" onClick={clearSelected} disabled={selected.size === 0 || clearing}>
              {clearing ? t("settings.storage.cleanup.clearing")
                : selectedBytes > 0 ? `${t("settings.storage.cleanup.clearSelected")} · ${fmtBytes(selectedBytes)}`
                : t("settings.storage.cleanup.clearSelected")}
            </Button>
          </div>
        )}
      </div>

      {/* Free up disk space: delete Drive-backed local copies */}
      <div className="py-3">
        <div className="mb-1 text-sm text-stone-200">{t("settings.storage.cleanup.reclaimTitle")}</div>
        <div className="mb-2 text-[11px] text-stone-500">{t("settings.storage.cleanup.reclaimHint")}</div>
        {!reclaim ? (
          <div className="animate-pulse py-1 text-xs text-stone-700">{t("common.loading")}</div>
        ) : reclaim.length === 0 ? (
          <div className="py-1 text-xs text-stone-600">{t("settings.storage.cleanup.reclaimNone")}</div>
        ) : (
          <>
            <div className="flex items-center justify-between gap-3">
              <span className="text-xs text-stone-400">{t("settings.storage.cleanup.reclaimSummary")(reclaim.length, fmtBytes(reclaimBytes))}</span>
              <Button variant="danger" onClick={() => setConfirm(true)} disabled={deleting}>
                {deleting ? t("settings.storage.cleanup.deleting") : t("settings.storage.cleanup.deleteLocalCopies")}
              </Button>
            </div>
            <div className="mt-2 flex max-h-44 flex-col divide-y divide-stone-800/70 overflow-y-auto">
              {reclaim.slice(0, 200).map((r) => (
                <div key={r.name} className="flex items-center justify-between gap-3 py-1.5">
                  <span className="min-w-0 truncate text-[12px] text-stone-400" title={r.name}>{r.name.split("/").pop()}</span>
                  <span className="shrink-0 text-[11px] tabular-nums text-stone-500">{fmtBytes(r.size)}</span>
                </div>
              ))}
            </div>
          </>
        )}
      </div>

      {/* Portaled to <body>: the Settings scroll container / card transforms
          otherwise break `position: fixed`, anchoring these to the card
          instead of the viewport. */}
      {toast && createPortal(
        <div className="pointer-events-none fixed bottom-6 right-6 z-[80] animate-fade-in rounded-full border border-stone-700/60 bg-stone-900/95 px-4 py-2 text-xs font-medium text-stone-200 shadow-2xl backdrop-blur-md">
          {toast}
        </div>,
        document.body
      )}

      {confirm && createPortal(
        <div className="fixed inset-0 z-[80] flex items-center justify-center bg-black/60 p-6" onClick={() => setConfirm(false)}>
          <div className="w-full max-w-sm rounded-xl border border-stone-800 bg-stone-900 p-5" onClick={(e) => e.stopPropagation()}>
            <div className="mb-1 text-sm font-semibold text-stone-100">{t("settings.storage.cleanup.confirmTitle")}</div>
            <div className="mb-4 text-[13px] text-stone-400">{t("settings.storage.cleanup.confirmBody")(reclaim.length, fmtBytes(reclaimBytes))}</div>
            <div className="flex justify-end gap-2">
              <button onClick={() => setConfirm(false)}
                className="rounded-lg px-3.5 py-1.5 text-[13px] font-medium text-stone-300 transition hover:bg-stone-800">
                {t("settings.storage.cleanup.confirmCancel")}
              </button>
              <button onClick={deleteLocalCopies}
                className="rounded-lg bg-red-500/15 px-3.5 py-1.5 text-[13px] font-medium text-red-400 transition hover:bg-red-500/25">
                {t("settings.storage.cleanup.confirmDelete")}
              </button>
            </div>
          </div>
        </div>,
        document.body
      )}
    </Card>
  );
}

// Session-lifetime cache: `get_storage_info` walks the whole recordings
// folder plus a network round trip for cloud quota, so re-opening this page
// shows cached numbers instantly and refreshes quietly underneath.
let cachedStorageInfo = null;

export function StorageSettingsCard({ settings, apply, t, lang, onOpenDrive }) {
  const [info, setInfo] = useState(() => cachedStorageInfo);
  const [refreshing, setRefreshing] = useState(!cachedStorageInfo);
  // Dev-only override for the displayed folder path (see the `set-storage-demo`
  // listener below) — the real `resolved_dir` is the automation's isolated
  // temp dir, which reads as an ugly (and username-revealing) path in a screenshot.
  const [demoPath, setDemoPath] = useState(null);
  const video = settings.video ?? {};

  useEffect(() => {
    let cancelled = false;
    const load = () => {
      setRefreshing(true);
      invoke("get_storage_info")
        .then((v) => {
          if (cancelled) return;
          cachedStorageInfo = v;
          setInfo(v);
        })
        .catch(() => {})
        .finally(() => { if (!cancelled) setRefreshing(false); });
    };
    load();
    let unlisten = [];
    (async () => {
      unlisten.push(await listen("video-saved", load));
      unlisten.push(await listen("library-changed", load));

      // Dev-only hook for store_screenshots.rs; stripped from prod builds.
      if (import.meta.env.DEV) {
        unlisten.push(await listen("store-screenshot-cmd", ({ payload }) => {
          if (payload?.action === "set-storage-demo") {
            setDemoPath(payload.path ?? null);
            requestAnimationFrame(() => setTimeout(() => emit("store-screenshot-ready", {}), 50));
          }
        }));
      }
    })();
    return () => { cancelled = true; unlisten.forEach((u) => u()); };
  }, []);

  const browseRecordings = async () => {
    const picked = await invoke("pick_folder");
    if (picked) apply({ video: { ...video, recordings_dir: picked } });
  };

  const limitMb = settings.storage_limit_mb ?? null;
  const hasLimit = limitMb != null;
  const displayedDir = demoPath ?? info?.resolved_dir;
  const driveLetter = driveLetterFromPath(displayedDir);

  const clips = info?.clips_bytes ?? 0;
  const recordings = info?.recordings_bytes ?? 0;
  const local = info?.local_bytes ?? clips + recordings;
  const hasDiskInfo = info?.disk_total != null && info?.disk_free != null;

  // With a limit set, the bar measures against it (not the real disk, which
  // would dwarf a small cap). Falls back to a whole-disk view with no limit,
  // or a bare local-only view if disk info couldn't be read.
  const limitBytes = hasLimit ? limitMb * 1024 * 1024 : null;
  const other = hasDiskInfo ? Math.max(0, info.disk_total - info.disk_free - local) : 0;
  const segments = hasLimit
    ? [
        { key: "clips", bytes: clips, cls: "bg-accent-400" },
        { key: "recordings", bytes: recordings, cls: "bg-accent-700" },
        { key: "free", bytes: Math.max(0, limitBytes - local), cls: "bg-stone-800" },
      ]
    : hasDiskInfo
    ? [
        { key: "clips", bytes: clips, cls: "bg-accent-400" },
        { key: "recordings", bytes: recordings, cls: "bg-accent-700" },
        { key: "other", bytes: other, cls: "bg-stone-600" },
        { key: "free", bytes: info.disk_free, cls: "bg-stone-800" },
      ]
    : [
        { key: "clips", bytes: clips, cls: "bg-accent-400" },
        { key: "recordings", bytes: recordings, cls: "bg-accent-700" },
      ];
  const barTotal = hasLimit
    ? Math.max(limitBytes, local, 1)
    : segments.reduce((s, x) => s + x.bytes, 0) || 1;
  const overLimit = hasLimit && local > limitBytes;

  return (
    <>
      <Card title={t("settings.storage.settingsTitle")}
        right={refreshing && info && (
          <MdRefresh size={14} className="animate-spin text-stone-600" title={t("settings.storage.refreshing")} />
        )}
      >
        <div className="py-3">
          <div className="mb-1 text-sm text-stone-200">{t("settings.storage.folderLocation")}</div>
          <div className="flex items-center gap-2">
            {info ? (
              <input type="text" readOnly value={displayedDir || video.recordings_dir || ""}
                className={`${inputCls} flex-1`} />
            ) : (
              <div className={`${inputCls} flex-1 animate-pulse text-stone-700`}>{t("common.loading")}</div>
            )}
            <Button onClick={browseRecordings}>{t("settings.record.browse")}</Button>
          </div>
        </div>

        <div className="py-3">
          <div className="mb-2 flex items-center justify-between gap-3">
            <span className="text-sm text-stone-200">{t("settings.storage.usageTitle")}</span>
            <select value={limitMb ?? ""} title={t("settings.storage.localLimitHint")}
              onChange={(e) => apply({ storage_limit_mb: e.target.value === "" ? null : Number(e.target.value) })}
              className={`${inputCls} cursor-pointer py-1 text-xs`}>
              <option value="">{t("settings.storage.noLimit")}</option>
              {LIMIT_PRESETS_MB.map((mb) => (
                <option key={mb} value={mb}>{formatLimitOption(mb, t)}</option>
              ))}
            </select>
          </div>

          {info ? (
            <>
              {driveLetter && (
                <div className="mb-1 text-xs text-stone-500">{t("settings.storage.driveLabel")(driveLetter)}</div>
              )}
              <div className="flex h-2 w-full overflow-hidden rounded-full bg-stone-800">
                {segments.map((s) => s.bytes > 0 && (
                  <div key={s.key} className={s.cls} style={{ width: `${(s.bytes / barTotal) * 100}%` }} />
                ))}
              </div>
              <div className="mt-2 flex flex-wrap items-center gap-x-4 gap-y-1 text-[11px] text-stone-500">
                <span className="flex items-center gap-1.5">
                  <span className="h-2 w-2 rounded-full bg-accent-400" />{t("settings.storage.clips")} {fmtBytes(clips)}
                </span>
                <span className="flex items-center gap-1.5">
                  <span className="h-2 w-2 rounded-full bg-accent-700" />{t("settings.storage.recordings")} {fmtBytes(recordings)}
                </span>
                {hasLimit ? (
                  <span className={overLimit ? "text-red-400" : ""}>
                    {overLimit
                      ? t("settings.storage.overLimit")(fmtBytes(local - limitBytes))
                      : t("settings.storage.ofLimit")(fmtBytes(local), fmtBytes(limitBytes))}
                  </span>
                ) : hasDiskInfo && (
                  <>
                    <span className="flex items-center gap-1.5">
                      <span className="h-2 w-2 rounded-full bg-stone-600" />{t("settings.storage.other")} {fmtBytes(other)}
                    </span>
                    <span>{t("settings.storage.availableOf")(fmtBytes(info.disk_free), fmtBytes(info.disk_total))}</span>
                  </>
                )}
              </div>
            </>
          ) : (
            <div className="animate-pulse">
              <div className="h-2 w-full rounded-full bg-stone-800" />
            </div>
          )}
        </div>

        {/* Google Drive capacity — from the same `get_storage_info` payload. */}
        <div className="py-3">
          <div className="mb-1.5 text-sm text-stone-200">{t("settings.storage.drive.title")}</div>
          {!info ? (
            <div className="h-2 w-full animate-pulse rounded-full bg-stone-800" />
          ) : info.drive_usage > 0 || info.drive_limit != null ? (
            <>
              {info.drive_limit != null ? (
                <>
                  <div className="h-2 w-full overflow-hidden rounded-full bg-stone-800">
                    <div className={`h-full rounded-full transition-all ${
                      info.drive_usage / info.drive_limit > 0.9 ? "bg-red-500"
                        : info.drive_usage / info.drive_limit > 0.7 ? "bg-amber-500" : "bg-emerald-500"
                    }`} style={{ width: `${Math.min(100, (info.drive_usage / info.drive_limit) * 100).toFixed(1)}%` }} />
                  </div>
                  <div className="mt-2 text-[11px] text-stone-500">
                    {t("settings.storage.drive.usedOf")(fmtBytes(info.drive_usage), fmtBytes(info.drive_limit))}
                  </div>
                </>
              ) : (
                <div className="text-[11px] text-stone-500">
                  {t("settings.storage.drive.used")(fmtBytes(info.drive_usage))} · {t("settings.storage.drive.unlimited")}
                </div>
              )}
            </>
          ) : (
            <div className="text-[11px] text-stone-600">{t("settings.storage.drive.notConnected")}</div>
          )}
        </div>

        <Row label={t("settings.storage.cloudSyncTitle")} hint={t("settings.storage.cloudSyncDesc")}>
          <Button onClick={onOpenDrive}>{t("settings.storage.cloudSyncButton")}</Button>
        </Row>
      </Card>

      <Card title={t("settings.storage.managementTitle")}>
        <Row label={t("settings.storage.autoDeleteTitle")}
          hint={hasLimit ? (settings.auto_delete_oldest ? t("settings.storage.autoDeleteWarning") : t("settings.storage.autoDeleteDesc")) : t("settings.storage.configureLimitHint")}>
          <Toggle checked={!!settings.auto_delete_oldest} onChange={(v) => apply({ auto_delete_oldest: v })} />
        </Row>
        <Row label={t("settings.storage.onlyLongTitle")}
          hint={hasLimit ? t("settings.storage.onlyLongDesc") : t("settings.storage.configureLimitHint")}>
          <Toggle checked={!!settings.only_delete_long_recordings} onChange={(v) => apply({ only_delete_long_recordings: v })} />
        </Row>
        <Row label={t("settings.storage.recycleBinTitle")} hint={t("settings.storage.recycleBinDesc")}>
          <Toggle checked={settings.use_recycle_bin ?? true} onChange={(v) => apply({ use_recycle_bin: v })} />
        </Row>
        <Row label={t("settings.storage.keepFavoritesTitle")} hint={t("settings.storage.keepFavoritesDesc")}>
          <Toggle checked={!!settings.keep_favorites} onChange={(v) => apply({ keep_favorites: v })} />
        </Row>
      </Card>

      <DataCleanupCard t={t} />

      <DeletionLogCard t={t} lang={lang} />
    </>
  );
}
