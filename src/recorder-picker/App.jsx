import { useEffect, useState } from "react";
import { invoke } from "../lib/tauri.js";
import { createT } from "../lib/i18n.js";
import { MdClose } from "react-icons/md";

export default function App() {
  const [lang, setLang] = useState("en");
  const t = createT(lang);
  const [windows, setWindows] = useState(null); // null = still loading

  useEffect(() => {
    invoke("recorder_picker_ready").catch(() => {});
    invoke("get_settings").then((s) => setLang(s.language ?? "en")).catch(() => {});
    invoke("recorder_list_window_thumbs").then(setWindows).catch(() => setWindows([]));
  }, []);

  const cancel = () => invoke("recorder_cancel_picker").catch(() => {});

  useEffect(() => {
    const onKey = (e) => { if (e.key === "Escape") cancel(); };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const pick = (w) => invoke("recorder_pick_window_select", { hwnd: w.hwnd, title: w.title, appName: w.app }).catch(() => {});

  return (
    <div className="flex h-screen flex-col">
      <div data-tauri-drag-region className="flex items-center justify-between border-b border-white/[0.06] px-4 py-3">
        <span className="text-sm font-semibold text-stone-300">{t("recorder.pickWindowTitle")}</span>
        <button
          onClick={cancel}
          title={t("common.close")}
          className="flex h-7 w-7 items-center justify-center rounded-md text-stone-500 transition-colors hover:bg-white/8 hover:text-stone-200"
        >
          <MdClose size={16} />
        </button>
      </div>
      <div className="flex-1 overflow-y-auto p-4">
        {windows === null ? (
          <div className="flex h-full items-center justify-center text-sm text-stone-500">{t("recorder.loadingWindows")}</div>
        ) : windows.length === 0 ? (
          <div className="flex h-full items-center justify-center text-sm text-stone-500">{t("recorder.noWindows")}</div>
        ) : (
          <div className="grid grid-cols-4 gap-3">
            {windows.map((w) => (
              <button
                key={w.hwnd}
                onClick={() => pick(w)}
                className="group flex flex-col overflow-hidden rounded-lg border border-stone-800 bg-stone-900 text-left transition-colors hover:border-accent-500"
              >
                <div className="flex aspect-video items-center justify-center overflow-hidden bg-black">
                  {w.thumbnail ? (
                    <img src={w.thumbnail} alt="" className="h-full w-full object-contain" />
                  ) : (
                    <span className="text-xs text-stone-600">{t("recorder.noPreview")}</span>
                  )}
                </div>
                <div className="truncate px-2 py-1.5 text-xs text-stone-300 group-hover:text-accent-400">
                  {w.title || w.app}
                </div>
              </button>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
