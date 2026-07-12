import { useEffect, useRef } from "react";
import * as Icon from "./icons.jsx";
import { invoke } from "../lib/tauri.js";

function basename(p) {
  return p.split(/[\\/]/).pop();
}

// Colored direction icon shown on every row (in-progress AND history), so
// upload vs download is legible at a glance. Download = sky, Upload = accent.
function DirectionIcon({ isDownload }) {
  const Glyph = isDownload ? Icon.Download : Icon.Upload;
  return <Glyph size={12} className={`shrink-0 ${isDownload ? "text-sky-400" : "text-accent-400"}`} />;
}

function Row({ tr, t }) {
  const pct = tr.total > 0 ? Math.round((tr.sent / tr.total) * 100) : 0;
  const inProgress = tr.status === "uploading" || tr.status === "downloading";
  const isDownload = tr.direction === "download";
  const barColor = isDownload ? "bg-sky-400" : "bg-accent-400";
  return (
    <div className="border-b border-stone-800/40 px-3 py-2 last:border-0">
      <div className="flex items-center gap-2">
        <span className={`h-1.5 w-1.5 shrink-0 rounded-full ${
          inProgress ? `animate-pulse ${barColor}`
            : tr.status === "done" ? "bg-emerald-400"
            : tr.status === "error" ? "bg-red-400" : "bg-stone-600"
        }`} />
        <DirectionIcon isDownload={isDownload} />
        <span className="min-w-0 flex-1 truncate text-[12px] text-stone-300">{basename(tr.file)}</span>
        {inProgress && tr.total > 0 && (
          <span className="shrink-0 text-[10px] tabular-nums text-stone-500">{pct}%</span>
        )}
        {tr.status === "done" && <Icon.Check size={11} className="shrink-0 text-emerald-400" />}
        {tr.status === "error" && <Icon.X size={11} className="shrink-0 text-red-400" />}
      </div>
      {inProgress && tr.total > 0 && (
        <div className="ml-3.5 mt-1 h-0.5 overflow-hidden rounded-full bg-stone-800">
          <div className={`h-full rounded-full transition-all duration-200 ${barColor}`} style={{ width: `${pct}%` }} />
        </div>
      )}
      {tr.status === "error" && tr.message && (
        <p className="ml-3.5 mt-0.5 truncate text-[10px] text-red-400">{tr.message}</p>
      )}
    </div>
  );
}

// In-gallery transfer queue panel, anchored under the refresh/backup button.
// Renders the live `transfers` state App.jsx tracks via `sync-transfers-changed`.
export default function TransferPanel({ t, transfers, syncing, onRefresh, onClose }) {
  const ref = useRef(null);

  useEffect(() => {
    const onClickOutside = (e) => { if (ref.current && !ref.current.contains(e.target)) onClose(); };
    document.addEventListener("mousedown", onClickOutside);
    return () => document.removeEventListener("mousedown", onClickOutside);
  }, [onClose]);

  const active = transfers.active || [];
  const queued = transfers.queued || [];
  const history = transfers.history || [];
  const isPaused = transfers.is_paused || false;
  const waitingCount = transfers.queued_count ?? queued.length; // genuinely not-yet-started only
  const hasQueue = active.length > 0 || waitingCount > 0;
  const isEmpty = active.length === 0 && queued.length === 0 && history.length === 0;

  return (
    <div ref={ref} className="absolute right-0 top-9 z-40 flex max-h-[420px] w-80 flex-col overflow-hidden rounded-xl border border-stone-800 bg-stone-950 shadow-2xl">
      <div className="flex shrink-0 items-center justify-between border-b border-stone-800/60 px-3 py-2.5">
        <span className="text-xs font-semibold text-stone-200">{t("gallery.transfer.title")}</span>
        <div className="flex items-center gap-1">
          <button onClick={onRefresh} title={t("gallery.sidebar.refresh")}
            className="rounded p-1 text-stone-500 transition hover:bg-stone-800 hover:text-stone-200">
            <Icon.Refresh size={13} className={syncing ? "animate-spin" : ""} />
          </button>
          {hasQueue && (
            <button onClick={() => invoke("toggle_sync_pause").catch(() => {})}
              title={isPaused ? t("gallery.transfer.resume") : t("gallery.transfer.pause")}
              className="rounded p-1 text-stone-500 transition hover:bg-stone-800 hover:text-stone-200">
              {isPaused ? <Icon.Play size={13} /> : <Icon.Pause size={13} />}
            </button>
          )}
          {hasQueue && (
            <button onClick={() => invoke("clear_sync_queue").catch(() => {})}
              title={t("gallery.transfer.clear")}
              className="rounded p-1 text-stone-500 transition hover:bg-stone-800 hover:text-red-400">
              <Icon.Square size={13} />
            </button>
          )}
          <button onClick={onClose} className="rounded p-1 text-stone-500 transition hover:bg-stone-800 hover:text-stone-200">
            <Icon.X size={14} />
          </button>
        </div>
      </div>

      {!isEmpty && (
        <div className="flex shrink-0 items-center gap-2 border-b border-stone-800/40 px-3 py-1.5 text-[10px]">
          {isPaused && <span className="text-accent-400">{t("gallery.transfer.paused")}</span>}
          {waitingCount > 0 && <span className="text-stone-500">{t("gallery.transfer.waiting")(waitingCount)}</span>}
          {(transfers.total_done ?? 0) > 0 && <span className="text-emerald-500">{t("gallery.transfer.done")(transfers.total_done)}</span>}
          {(transfers.total_error ?? 0) > 0 && <span className="text-red-400">{t("gallery.transfer.error")(transfers.total_error)}</span>}
        </div>
      )}

      <div className="min-h-0 flex-1 overflow-y-auto">
        {isEmpty ? (
          <div className="flex h-24 items-center justify-center text-[11px] text-stone-600">{t("gallery.transfer.queueEmpty")}</div>
        ) : (
          <>
            {active.map((tr, i) => <Row key={`a${tr.file}${i}`} tr={tr} t={t} />)}
            {queued.map((tr, i) => <Row key={`q${tr.file}${i}`} tr={tr} t={t} />)}
            {history.slice(0, 8).map((tr, i) => <Row key={`h${tr.file}${i}`} tr={tr} t={t} />)}
          </>
        )}
      </div>
    </div>
  );
}
