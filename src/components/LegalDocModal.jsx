import { useEffect, useState } from "react";
import { LEGAL_DOCS } from "../lib/legal.js";
import { openExternalLinks } from "../lib/links.js";
import { translateHtml, translateText } from "../lib/translate.js";

// Renders TERMS.md/PRIVACY.md/LICENSE in-app, with an optional machine-translated view.
export default function LegalDocModal({ doc, title, lang, t, onClose }) {
  const data = LEGAL_DOCS[doc];
  const [translated, setTranslated] = useState(false);
  const [translating, setTranslating] = useState(false);
  const [html, setHtml] = useState(data.html);
  const [text, setText] = useState(data.text);

  useEffect(() => {
    setTranslated(false);
    setHtml(data.html);
    setText(data.text);
  }, [doc]);

  const canTranslate = lang && lang !== "en";

  const toggleTranslate = async () => {
    if (translated) {
      setTranslated(false);
      setHtml(data.html);
      setText(data.text);
      return;
    }
    setTranslating(true);
    try {
      if (data.html) setHtml(await translateHtml(data.html, lang));
      else setText(await translateText(data.text, lang));
      setTranslated(true);
    } finally {
      setTranslating(false);
    }
  };

  return (
    <div className="absolute inset-0 z-[200] flex items-center justify-center bg-black/85 p-4" onClick={onClose}>
      <div
        className="relative flex flex-col w-full max-w-[560px] max-h-[85vh] rounded-2xl border border-stone-700/60 bg-stone-950 shadow-2xl shadow-black/80 text-left"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center justify-between px-6 py-4 border-b border-stone-800 shrink-0">
          <div>
            <h2 className="text-base font-semibold text-stone-100">{title}</h2>
            {data.updated && <p className="text-xs text-stone-500 mt-0.5">{data.updated}</p>}
          </div>
          <div className="flex items-center gap-3">
            {canTranslate && (
              <button onClick={toggleTranslate} disabled={translating}
                className="text-xs text-accent-400 hover:text-accent-300 disabled:opacity-50 transition-colors">
                {translating ? t("legal.translating") : translated ? t("legal.showOriginal") : t("legal.translate")}
              </button>
            )}
            <button onClick={onClose} className="text-stone-500 hover:text-stone-300 text-lg leading-none">✕</button>
          </div>
        </div>
        <div className="overflow-y-auto flex-1 px-6 py-4">
          {translated && (
            <p className="text-xs text-stone-500 mb-3 italic">{t("legal.machineTranslated")}</p>
          )}
          {html ? (
            <div
              className="select-text text-sm text-stone-300 leading-relaxed space-y-4
                [&_h2]:text-base [&_h2]:font-semibold [&_h2]:text-stone-100 [&_h2]:mt-6 [&_h2]:mb-2 [&_h2]:first:mt-0
                [&_a]:text-accent-400 [&_a]:hover:underline
                [&_strong]:text-stone-100 [&_strong]:font-semibold
                [&_code]:text-accent-300 [&_code]:bg-stone-900 [&_code]:px-1 [&_code]:py-0.5 [&_code]:rounded [&_code]:text-[13px]
                [&_ul]:list-disc [&_ul]:pl-5 [&_ul]:space-y-1.5"
              onClick={openExternalLinks}
              dangerouslySetInnerHTML={{ __html: html }}
            />
          ) : (
            <pre className="select-text text-xs text-stone-400 whitespace-pre-wrap font-mono">{text}</pre>
          )}
        </div>
      </div>
    </div>
  );
}
