import { useState } from "react";
import { marked } from "marked";
import { FiChevronDown } from "react-icons/fi";
import { openExternalLinks } from "../lib/links.js";
import { translateHtml } from "../lib/translate.js";
import { extractChangelog } from "../lib/version.js";

const docClasses =
  "select-text text-sm text-stone-300 leading-relaxed space-y-3" +
  " [&_a]:text-accent-400 [&_a]:hover:underline [&_strong]:text-stone-100 [&_strong]:font-semibold" +
  " [&_code]:text-accent-300 [&_code]:bg-stone-900 [&_code]:px-1 [&_code]:py-0.5 [&_code]:rounded [&_code]:text-[13px]" +
  " [&_ul]:list-disc [&_ul]:pl-5 [&_ul]:space-y-1 [&_li]:marker:text-stone-600";

// One release's collapsible entry — translation is per-entry and lazy, so
// opening a long history doesn't fire a translate request for every release
// at once, only the one(s) actually expanded.
function ReleaseEntry({ release, defaultOpen, lang, t }) {
  const original = marked.parse(extractChangelog(release.body || ""));
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

  return (
    <details className="group" open={defaultOpen}>
      <summary className="flex cursor-pointer list-none items-center justify-between gap-2 py-1">
        <span className="text-sm font-semibold text-stone-100">{release.name || release.version}</span>
        <FiChevronDown size={15} className="text-stone-500 shrink-0 transition-transform group-open:rotate-180" />
      </summary>
      <div className="mt-2 pl-0.5">
        {canTranslate && (
          <button onClick={toggleTranslate} disabled={translating}
            className="mb-2 text-xs text-accent-400 hover:text-accent-300 disabled:opacity-50 transition-colors">
            {translating ? t("legal.translating") : translated ? t("legal.showOriginal") : t("legal.translate")}
          </button>
        )}
        {translated && <p className="mb-2 text-xs text-stone-500 italic">{t("legal.machineTranslated")}</p>}
        <div className={docClasses} onClick={openExternalLinks} dangerouslySetInnerHTML={{ __html: html }} />
      </div>
    </details>
  );
}

// `releases` is pre-filtered/sorted by the caller.
export default function WhatsNewModal({ releases, lang, t, onClose }) {
  return (
    <div className="absolute inset-0 z-[100] flex items-center justify-center bg-black/85 p-4" onClick={onClose}>
      <div
        className="relative flex flex-col w-full max-w-[560px] max-h-[85vh] rounded-2xl border border-stone-700/60 bg-stone-950 shadow-2xl shadow-black/80 text-left"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center justify-between px-6 py-4 border-b border-stone-800 shrink-0">
          <h2 className="text-base font-semibold text-stone-100">{t("whatsNew.title")}</h2>
          <button onClick={onClose} className="text-stone-500 hover:text-stone-300 text-lg leading-none">✕</button>
        </div>

        <div className="overflow-y-auto flex-1 px-6 py-4 divide-y divide-stone-800/70">
          {releases.map((r, i) => (
            <div key={r.version} className={i > 0 ? "pt-3" : ""}>
              <ReleaseEntry release={r} defaultOpen={i === 0} lang={lang} t={t} />
            </div>
          ))}
        </div>

        <div className="px-6 py-4 border-t border-stone-800/60 shrink-0">
          <button onClick={onClose}
            className="w-full rounded-lg bg-accent-400 px-3.5 py-2 text-sm font-medium text-stone-950 hover:bg-accent-300 transition">
            {t("whatsNew.close")}
          </button>
        </div>
      </div>
    </div>
  );
}
