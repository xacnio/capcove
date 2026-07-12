import { useState } from "react";
import { MdVideocam } from "react-icons/md";
import { invoke } from "../lib/tauri.js";

// Shown when the detected game closes while Clips mode was buffering it and
// `confirm_save_on_close` is on — asks rather than silently discarding the
// buffer, since the last few minutes of gameplay are otherwise gone for
// good. No backdrop-dismiss, same reasoning as `ReplayCrashRecoveryModal`:
// closing without a choice would strand the staged segments.
export default function PendingClipModal({ result, t, onClose }) {
  const { game } = result;
  const [busy, setBusy] = useState(null); // null | "saving" | "discarding"
  const [error, setError] = useState(null);

  const save = async () => {
    setBusy("saving");
    setError(null);
    try {
      await invoke("confirm_pending_clip");
      onClose();
    } catch (e) {
      setError(String(e));
      setBusy(null);
    }
  };

  const discard = async () => {
    setBusy("discarding");
    try {
      await invoke("discard_pending_clip");
    } finally {
      onClose();
    }
  };

  return (
    <div className="absolute inset-0 z-[100] flex items-center justify-center bg-black/85 p-4">
      <div className="relative flex w-full max-w-[480px] flex-col rounded-2xl border border-stone-700/60 bg-stone-950 shadow-2xl shadow-black/80 text-left">
        <div className="flex items-start gap-3 px-6 pt-6 pb-4">
          <MdVideocam size={26} className="mt-0.5 shrink-0 text-accent-400" />
          <div className="min-w-0">
            <h2 className="text-base font-semibold text-stone-100">{t("pendingClip.title")}</h2>
            <p className="mt-1.5 text-sm leading-relaxed text-stone-300">
              {game ? t("pendingClip.bodyGame")(game) : t("pendingClip.body")}
            </p>
            {error && <p className="mt-2 text-sm text-red-400">{error}</p>}
          </div>
        </div>

        <div className="flex items-center justify-end gap-2 px-6 pb-6 pt-3">
          <button onClick={discard} disabled={!!busy}
            className="rounded-lg px-3.5 py-2 text-sm font-medium text-stone-300 hover:bg-stone-800 transition disabled:opacity-50">
            {busy === "discarding" ? t("pendingClip.discarding") : t("pendingClip.discard")}
          </button>
          <button onClick={save} disabled={!!busy}
            className="rounded-lg bg-accent-400 px-3.5 py-2 text-sm font-medium text-stone-950 hover:bg-accent-300 transition disabled:opacity-50">
            {busy === "saving" ? t("pendingClip.saving") : t("pendingClip.save")}
          </button>
        </div>
      </div>
    </div>
  );
}
