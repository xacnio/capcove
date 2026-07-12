import { MdCleaningServices, MdWarningAmber } from "react-icons/md";
import { invoke } from "../lib/tauri.js";

function fmtBytes(bytes) {
  if (!bytes) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let v = bytes, i = 0;
  while (v >= 1024 && i < units.length - 1) { v /= 1024; i++; }
  return `${v.toFixed(i === 0 ? 0 : 1)} ${units[i]}`;
}

// Shown once at startup: a summary of whatever the storage-limit/folder-age
// auto-delete cleaned up since the last time this was shown, plus a standing
// warning if the local limit is currently exceeded with auto-delete off.
// Either section can appear alone — see `check_storage_startup_summary`.
export default function StorageSummaryModal({ result, t, onGoToSettings, onClose }) {
  const { new_deletions: deletions, over_limit: overLimit, use_recycle_bin: recycleBinOn } = result;
  const totalFreed = (deletions ?? []).reduce((s, e) => s + e.size, 0);

  const close = async () => {
    await invoke("ack_deletion_summary").catch(() => {});
    onClose();
  };

  const openRecycleBin = () => invoke("open_recycle_bin").catch(() => {});

  return (
    <div className="absolute inset-0 z-[100] flex items-center justify-center bg-black/85 p-4">
      <div className="relative flex w-full max-w-[480px] flex-col rounded-2xl border border-stone-700/60 bg-stone-950 shadow-2xl shadow-black/80 text-left">
        <div className="flex flex-col gap-4 px-6 pt-6 pb-4">
          {deletions?.length > 0 && (
            <div className="flex items-start gap-3">
              <MdCleaningServices size={24} className="mt-0.5 shrink-0 text-accent-400" />
              <div className="min-w-0 flex-1">
                <h2 className="text-base font-semibold text-stone-100">{t("storageSummary.title")}</h2>
                <p className="mt-1 text-sm text-stone-300">
                  {t("storageSummary.deletedCount")(deletions.length)} · {t("storageSummary.totalFreed")(fmtBytes(totalFreed))}
                </p>
                <div className="mt-2.5 flex max-h-40 flex-col divide-y divide-stone-800/70 overflow-y-auto rounded-lg border border-stone-800 bg-stone-900/60">
                  {deletions.map((entry, i) => (
                    <div key={`${entry.name}-${entry.deleted_at}-${i}`} className="flex items-center justify-between gap-3 px-3 py-1.5 text-xs">
                      <span className="min-w-0 truncate text-stone-300" title={entry.name}>{entry.name.split("/").pop()}</span>
                      <span className="shrink-0 text-stone-500">{fmtBytes(entry.size)}</span>
                    </div>
                  ))}
                </div>
                <p className="mt-2.5 text-xs text-stone-500">
                  {recycleBinOn ? t("storageSummary.recycleBinNote") : t("storageSummary.permanentNote")}
                </p>
                {recycleBinOn && (
                  <button onClick={openRecycleBin}
                    className="mt-2 rounded-lg border border-stone-700 px-3 py-1.5 text-xs font-medium text-stone-300 transition hover:bg-stone-800">
                    {t("storageSummary.openRecycleBin")}
                  </button>
                )}
              </div>
            </div>
          )}

          {overLimit && (
            <div className="flex items-start gap-3">
              <MdWarningAmber size={24} className="mt-0.5 shrink-0 text-amber-400" />
              <div className="min-w-0 flex-1">
                <h2 className="text-base font-semibold text-stone-100">{t("storageSummary.overLimitTitle")}</h2>
                <p className="mt-1 text-sm text-stone-300">
                  {t("storageSummary.overLimitBody")(fmtBytes(overLimit.used_bytes), fmtBytes(overLimit.limit_bytes))}
                </p>
              </div>
            </div>
          )}
        </div>

        <div className="flex items-center justify-end gap-2 px-6 pb-6 pt-1">
          {overLimit && (
            <button onClick={() => { onGoToSettings(); onClose(); }}
              className="rounded-lg px-3.5 py-2 text-sm font-medium text-stone-300 hover:bg-stone-800 transition">
              {t("storageSummary.goToSettings")}
            </button>
          )}
          <button onClick={close}
            className="rounded-lg bg-accent-400 px-3.5 py-2 text-sm font-medium text-stone-950 hover:bg-accent-300 transition">
            {t("storageSummary.close")}
          </button>
        </div>
      </div>
    </div>
  );
}
