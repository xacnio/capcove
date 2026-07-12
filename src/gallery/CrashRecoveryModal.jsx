import { useEffect, useState } from "react";
import { MdCheckCircle, MdErrorOutline } from "react-icons/md";
import { invoke, convertFileSrc } from "../lib/tauri.js";

// Shown once after `recording::check_crash_recovery` recovers a leftover marker from
// a recording active during the last crash. Modal (must be dismissed) with an inline
// player to confirm the recovered file plays.
export default function CrashRecoveryModal({ result, t, onClose, onGoToSettings }) {
  const { name, path, outcome } = result;
  const failed = outcome === "failed";
  const title = failed ? t("crashRecovery.titleFailed")
    : outcome === "repaired" ? t("crashRecovery.titleRepaired")
    : t("crashRecovery.titleDurable");
  const body = failed ? t("crashRecovery.bodyFailed")(name)
    : outcome === "repaired" ? t("crashRecovery.bodyRepaired")(name)
    : t("crashRecovery.bodyDurable")(name);

  const [thumb, setThumb] = useState(null);
  useEffect(() => {
    let cancelled = false;
    invoke("read_video_thumbnail", { name })
      .then((b64) => { if (!cancelled) setThumb(`data:image/jpeg;base64,${b64}`); })
      .catch(() => {});
    return () => { cancelled = true; };
  }, [name]);

  return (
    <div className="absolute inset-0 z-[100] flex items-center justify-center bg-black/85 p-4" onClick={onClose}>
      <div
        className="relative flex w-full max-w-[640px] flex-col rounded-2xl border border-stone-700/60 bg-stone-950 shadow-2xl shadow-black/80 text-left"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-start gap-3 px-6 pt-6 pb-4">
          {failed
            ? <MdErrorOutline size={26} className="mt-0.5 shrink-0 text-amber-400" />
            : <MdCheckCircle size={26} className="mt-0.5 shrink-0 text-emerald-400" />}
          <div className="min-w-0">
            <h2 className="text-base font-semibold text-stone-100">{title}</h2>
            <p className="mt-1.5 text-sm leading-relaxed text-stone-300">{body}</p>
            {failed && <p className="mt-2 text-sm leading-relaxed text-stone-500">{t("crashRecovery.failedHint")}</p>}
          </div>
        </div>

        <div className="px-6 pb-2">
          <div className="overflow-hidden rounded-xl border border-stone-800 bg-black">
            {/* Native controls, not the custom player: a broken file (`failed` case)
                should surface the browser's own inline error state. */}
            <video
              controls
              poster={thumb ?? undefined}
              src={convertFileSrc(path)}
              className="aspect-video w-full bg-black"
            />
          </div>
          <div className="mt-2 truncate text-xs text-stone-500" title={name}>{name}</div>
        </div>

        <div className="flex items-center justify-end gap-2 px-6 pb-6 pt-3">
          {failed && onGoToSettings && (
            <button onClick={() => { onGoToSettings(); onClose(); }}
              className="rounded-lg px-3.5 py-2 text-sm font-medium text-stone-300 hover:bg-stone-800 transition">
              {t("crashRecovery.goToSettings")}
            </button>
          )}
          <button onClick={onClose}
            className="rounded-lg bg-accent-400 px-3.5 py-2 text-sm font-medium text-stone-950 hover:bg-accent-300 transition">
            {t("crashRecovery.close")}
          </button>
        </div>
      </div>
    </div>
  );
}
