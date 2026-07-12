import { useState } from "react";
import { MdVideocam } from "react-icons/md";
import { invoke } from "../lib/tauri.js";

// Shown once at startup when a replay buffer crash leaves leftover disk-mode segments.
// Asks rather than auto-saving since buffer footage usually wasn't requested; no
// backdrop-dismiss, since closing without a choice would strand the segments.
export default function ReplayCrashRecoveryModal({ result, t, onClose }) {
  const { segment_count } = result;
  const [busy, setBusy] = useState(null); // null | "recovering" | "discarding"
  const [error, setError] = useState(null);

  const recover = async () => {
    setBusy("recovering");
    setError(null);
    try {
      await invoke("recover_replay_buffer_crash");
      onClose();
    } catch (e) {
      setError(String(e));
      setBusy(null);
    }
  };

  const discard = async () => {
    setBusy("discarding");
    try {
      await invoke("discard_replay_buffer_crash");
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
            <h2 className="text-base font-semibold text-stone-100">{t("replayCrashRecovery.title")}</h2>
            <p className="mt-1.5 text-sm leading-relaxed text-stone-300">{t("replayCrashRecovery.body")(segment_count)}</p>
            {error && <p className="mt-2 text-sm text-red-400">{error}</p>}
          </div>
        </div>

        <div className="flex items-center justify-end gap-2 px-6 pb-6 pt-3">
          <button onClick={discard} disabled={!!busy}
            className="rounded-lg px-3.5 py-2 text-sm font-medium text-stone-300 hover:bg-stone-800 transition disabled:opacity-50">
            {busy === "discarding" ? t("replayCrashRecovery.discarding") : t("replayCrashRecovery.discard")}
          </button>
          <button onClick={recover} disabled={!!busy}
            className="rounded-lg bg-accent-400 px-3.5 py-2 text-sm font-medium text-stone-950 hover:bg-accent-300 transition disabled:opacity-50">
            {busy === "recovering" ? t("replayCrashRecovery.recovering") : t("replayCrashRecovery.recover")}
          </button>
        </div>
      </div>
    </div>
  );
}
