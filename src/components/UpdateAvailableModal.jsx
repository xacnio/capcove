import { useEffect, useState } from "react";
import { marked } from "marked";
import { invoke, listen } from "../lib/tauri.js";
import { openExternalLinks } from "../lib/links.js";
import { translateHtml } from "../lib/translate.js";
import { extractChangelog } from "../lib/version.js";

// Shown once per update version (see Settings.last_notified_update_version)
// when the gallery opens — covers both the "gallery opened at startup" case
// and "user opened it later from the tray after an OS notification" case.
export default function UpdateAvailableModal({ info, lang, t, onClose }) {
  const [status, setStatus] = useState("idle"); // idle | downloading | error
  const [progress, setProgress] = useState(null);
  const [error, setError] = useState("");
  const original = marked.parse(extractChangelog(info.body || ""));
  const [html, setHtml] = useState(original);
  const [translated, setTranslated] = useState(false);
  const [translating, setTranslating] = useState(false);
  const canTranslate = lang && lang !== "en";

  const toggleTranslate = async (e) => {
    e.preventDefault();
    if (translated) {
      setHtml(original);
      setTranslated(false);
      return;
    }
    setTranslating(true);
    try {
      setHtml(await translateHtml(original, lang));
      setTranslated(true);
    } finally {
      setTranslating(false);
    }
  };

  useEffect(() => {
    let unlisten;
    (async () => {
      unlisten = await listen("update-download-progress", (e) => setProgress(e.payload));
    })();
    return () => unlisten?.();
  }, []);

  const install = async () => {
    setStatus("downloading");
    setError("");
    try {
      await invoke("download_and_install_update");
      // App restarts itself once the install finishes (see download_and_install_update).
    } catch (e) {
      setError(String(e));
      setStatus("error");
    }
  };

  return (
    <div className="absolute inset-0 z-[100] flex items-center justify-center bg-black/85 p-4">
      <div className="relative flex flex-col w-full max-w-[480px] max-h-[85vh] rounded-2xl border border-stone-700/60 bg-stone-950 shadow-2xl shadow-black/80 text-left">
        <div className="px-6 pt-6 pb-3 shrink-0">
          <h2 className="text-lg font-semibold text-stone-100">{t("updateModal.title")}</h2>
          <p className="mt-1 text-sm text-accent-400">
            {t("settings.about.versionAvailable").replace("{version}", info.version)}
          </p>
        </div>

        <div className="overflow-y-auto flex-1 px-6 py-4">
          {canTranslate && (
            <button onClick={toggleTranslate} disabled={translating}
              className="mb-2 text-xs text-accent-400 hover:text-accent-300 disabled:opacity-50 transition-colors">
              {translating ? t("legal.translating") : translated ? t("legal.showOriginal") : t("legal.translate")}
            </button>
          )}
          {translated && <p className="mb-2 text-xs text-stone-500 italic">{t("legal.machineTranslated")}</p>}
          <div
            className="select-text text-sm text-stone-300 leading-relaxed space-y-3
              [&_a]:text-accent-400 [&_a]:hover:underline [&_strong]:text-stone-100 [&_strong]:font-semibold
              [&_code]:text-accent-300 [&_code]:bg-stone-900 [&_code]:px-1 [&_code]:py-0.5 [&_code]:rounded [&_code]:text-[13px]
              [&_ul]:list-disc [&_ul]:pl-5 [&_ul]:space-y-1"
            onClick={openExternalLinks}
            dangerouslySetInnerHTML={{ __html: html }}
          />
        </div>

        <div className="px-6 py-4 border-t border-stone-800/60 shrink-0 flex flex-col gap-2">
          {status === "error" && <p className="text-xs text-red-400">{t("settings.about.updateError")}: {error}</p>}
          {status === "downloading" && (
            <div className="flex flex-col gap-2">
              <p className="text-xs text-stone-500">{t("settings.about.downloading")}</p>
              {progress?.total ? (
                <div className="h-1.5 w-full overflow-hidden rounded-full bg-stone-800">
                  <div
                    className="h-full bg-accent-500 transition-all"
                    style={{ width: `${Math.min(100, (progress.downloaded / progress.total) * 100)}%` }}
                  />
                </div>
              ) : null}
            </div>
          )}
          {status !== "downloading" && (
            <div className="flex gap-2">
              <button onClick={onClose}
                className="flex-1 rounded-lg border border-stone-700 bg-stone-800/60 px-3.5 py-2 text-sm text-stone-300 hover:bg-stone-700/60 hover:text-stone-100 transition">
                {t("updateModal.later")}
              </button>
              <button onClick={install}
                className="flex-1 rounded-lg bg-accent-500 px-3.5 py-2 text-sm font-medium text-stone-950 hover:bg-accent-400 transition">
                {t("settings.about.downloadUpdate")}
              </button>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
