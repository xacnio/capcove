import { useEffect, useState } from "react";
import { useLanguage } from "../lib/LanguageContext.jsx";
import { detectPlatform } from "../lib/platform.js";

// App shortcuts use Ctrl (not Command) on macOS too; spelled out since
// the "⌃" glyph renders as a near-invisible sliver in most fonts.
const MAC_KEY = { Ctrl: "Control", Shift: "⇧", Alt: "⌥" };

function Kbd({ children }) {
  return (
    <kbd className="inline-flex items-center justify-center min-w-[1.6rem] px-1.5 h-6 rounded-md border border-stone-700 bg-stone-800 text-[11px] font-mono text-stone-300">
      {children}
    </kbd>
  );
}

function Combo({ keys, isMac }) {
  return (
    <div className="flex items-center gap-1">
      {keys.map((k, i) => (
        <span key={k} className="flex items-center gap-1">
          <Kbd>{isMac ? (MAC_KEY[k] ?? k) : k}</Kbd>
          {i < keys.length - 1 && <span className="text-stone-600 text-xs">+</span>}
        </span>
      ))}
    </div>
  );
}

export default function Shortcuts() {
  const { t } = useLanguage();
  const rows = t("shortcuts.rows");
  const [platform, setPlatform] = useState("windows");
  useEffect(() => setPlatform(detectPlatform()), []);
  const isMac = platform === "macos";

  return (
    <section className="max-w-6xl mx-auto px-6 py-24 border-t border-stone-800/80">
      <h2 className="text-3xl font-bold tracking-tight">{t("shortcuts.title")}</h2>
      <p className="mt-3 text-stone-400 max-w-lg text-lg">{t("shortcuts.desc")}</p>
      <div className="mt-8 rounded-xl border border-stone-800 divide-y divide-stone-800 overflow-hidden">
        {rows.map((row) => (
          <div key={row.action + row.keys.join()} className="flex items-center gap-6 px-5 py-3.5 bg-stone-900/40">
            <div className="w-36 shrink-0">
              <Combo keys={row.keys} isMac={isMac} />
            </div>
            <div className="text-sm text-stone-200 w-44 shrink-0">{row.action}</div>
            <div className="text-sm text-stone-500">{row.result}</div>
          </div>
        ))}
      </div>
    </section>
  );
}
