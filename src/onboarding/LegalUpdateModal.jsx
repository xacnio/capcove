import { useEffect, useState } from "react";
import { invoke } from "../lib/tauri.js";
import { LEGAL_VERSION, LEGAL_DOCS } from "../lib/legal.js";
import { openExternalLinks } from "../lib/links.js";
import { translateHtml } from "../lib/translate.js";

// Shown when accepted_legal_version is stale, so the user re-accepts after either doc changes.
export default function LegalUpdateModal({ onAccept, t, lang }) {
  const [tab, setTab] = useState("terms");
  const [translated, setTranslated] = useState({ terms: false, privacy: false });
  const [translating, setTranslating] = useState(false);
  const [html, setHtml] = useState({ terms: LEGAL_DOCS.terms.html, privacy: LEGAL_DOCS.privacy.html });
  const data = LEGAL_DOCS[tab];
  const canTranslate = lang && lang !== "en";

  const accept = async () => {
    const s = await invoke("get_settings");
    await invoke("save_settings", { settings: { ...s, accepted_legal_version: LEGAL_VERSION } });
    onAccept();
  };

  const toggleTranslate = async () => {
    if (translated[tab]) {
      setHtml((h) => ({ ...h, [tab]: LEGAL_DOCS[tab].html }));
      setTranslated((s) => ({ ...s, [tab]: false }));
      return;
    }
    setTranslating(true);
    try {
      setHtml((h) => ({ ...h, [tab]: data.html })); // reset in case a previous translation lingers
      const result = await translateHtml(LEGAL_DOCS[tab].html, lang);
      setHtml((h) => ({ ...h, [tab]: result }));
      setTranslated((s) => ({ ...s, [tab]: true }));
    } finally {
      setTranslating(false);
    }
  };

  return (
    <div className="absolute inset-0 z-[100] flex items-center justify-center bg-black/85 p-4">
      <div className="relative flex flex-col w-full max-w-[560px] max-h-[85vh] rounded-2xl border border-stone-700/60 bg-stone-950 shadow-2xl shadow-black/80 text-left">
        <div className="px-6 pt-6 pb-3 shrink-0">
          <h2 className="text-lg font-semibold text-stone-100">{t("legalUpdate.title")}</h2>
          <p className="mt-1 text-sm text-stone-400">{t("legalUpdate.body")}</p>
        </div>

        <div className="flex items-center justify-between px-6 shrink-0">
          <div className="flex gap-1.5">
            {["terms", "privacy"].map((id) => (
              <button key={id} onClick={() => setTab(id)}
                className={`rounded-lg px-3 py-1.5 text-xs font-medium transition ${tab === id ? "bg-accent-500 text-stone-950" : "bg-stone-800 text-stone-400 hover:bg-stone-700"}`}>
                {t(`settings.about.${id}`)}
              </button>
            ))}
          </div>
          {canTranslate && (
            <button onClick={toggleTranslate} disabled={translating}
              className="text-xs text-accent-400 hover:text-accent-300 disabled:opacity-50 transition-colors">
              {translating ? t("legal.translating") : translated[tab] ? t("legal.showOriginal") : t("legal.translate")}
            </button>
          )}
        </div>

        <div className="overflow-y-auto flex-1 px-6 py-4">
          {data.updated && <p className="text-xs text-stone-500 mb-3">{data.updated}</p>}
          {translated[tab] && <p className="text-xs text-stone-500 mb-3 italic">{t("legal.machineTranslated")}</p>}
          <div
            className="select-text text-sm text-stone-300 leading-relaxed space-y-4
              [&_h2]:text-base [&_h2]:font-semibold [&_h2]:text-stone-100 [&_h2]:mt-6 [&_h2]:mb-2 [&_h2]:first:mt-0
              [&_a]:text-accent-400 [&_a]:hover:underline
              [&_strong]:text-stone-100 [&_strong]:font-semibold
              [&_code]:text-accent-300 [&_code]:bg-stone-900 [&_code]:px-1 [&_code]:py-0.5 [&_code]:rounded [&_code]:text-[13px]
              [&_ul]:list-disc [&_ul]:pl-5 [&_ul]:space-y-1.5"
            onClick={openExternalLinks}
            dangerouslySetInnerHTML={{ __html: html[tab] }}
          />
        </div>

        <div className="px-6 py-4 border-t border-stone-800/60 shrink-0">
          <button onClick={accept}
            className="w-full rounded-lg bg-accent-400 px-3.5 py-2 text-sm font-medium text-stone-950 hover:bg-accent-300 transition">
            {t("legalUpdate.accept")}
          </button>
        </div>
      </div>
    </div>
  );
}
