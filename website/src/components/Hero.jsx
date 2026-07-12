import { useEffect, useState } from "react";
import { FiChevronDown } from "react-icons/fi";
import { detectArch, sortByArch, MS_STORE_URL, msStoreBadgeUrl } from "../lib/platform.js";
import { useLanguage } from "../lib/LanguageContext.jsx";
import Screenshot from "./Screenshot.jsx";

const ARCH_LABELS = { x64: "x64", arm64: "ARM64" };
// Windows-only for now — see `Download.jsx`'s matching `PLATFORM_KEYS`.
const platform = "windows";

export default function Hero({ downloads = [] }) {
  const { t, lang } = useLanguage();
  const [arch, setArch] = useState(null);

  useEffect(() => {
    // Real arch detection is Chromium-only and unreliable under emulation,
    // so it can only upgrade the default x64 pick — never assumed upfront.
    detectArch().then((a) => a && setArch(a));
  }, []);

  const assets = sortByArch(downloads.filter((d) => d.platform === platform));
  const selected = (arch && assets.find((d) => d.arch === arch)) ?? assets[0];
  const alternatives = [...new Set(assets.map((d) => d.arch))]
    .filter((a) => a !== selected?.arch)
    .map((a) => assets.find((d) => d.arch === a));

  return (
    <section className="max-w-6xl mx-auto px-6 pt-20 pb-24">
      <div className="max-w-2xl">
        <h1 className="text-5xl sm:text-6xl lg:text-7xl font-bold tracking-tight leading-[1.03]">
          {t("hero.titleLine1")}
          <br />
          <span className="text-stone-500">{t("hero.titleLine2")}</span>
        </h1>
        <p className="mt-6 text-stone-400 text-lg leading-relaxed max-w-lg">{t("hero.desc")}</p>
        <div className="mt-8 flex flex-wrap items-center gap-3">
          <div className="inline-flex items-stretch">
            <a
              href={selected?.url || "#download"}
              className={`inline-flex items-center gap-2 text-sm font-medium px-4 py-2.5 ${alternatives.length > 0 ? "rounded-l-md" : "rounded-md"} bg-accent-500 text-stone-950 hover:bg-accent-400 transition-colors`}
            >
              {t("hero.downloadFor")(t(`platform.${platform}`))}
              {selected?.arch && selected.arch !== "universal" && (
                <span className="text-[10px] font-semibold uppercase tracking-wide text-stone-950/60">
                  {ARCH_LABELS[selected.arch] ?? selected.arch}
                </span>
              )}
            </a>
            {alternatives.length > 0 && (
              <details className="relative">
                <summary
                  className="list-none flex h-full items-center px-2 rounded-r-md bg-accent-500 text-stone-950 hover:bg-accent-400 transition-colors cursor-pointer border-l border-stone-950/20"
                  aria-label={t("hero.otherArch")}
                >
                  <FiChevronDown size={16} />
                </summary>
                <div className="absolute right-0 mt-2 w-44 rounded-md border border-stone-700 bg-stone-900 shadow-lg overflow-hidden z-10">
                  {alternatives.map((a) => (
                    <a
                      key={a.url}
                      href={a.url}
                      className="block px-3 py-2 text-sm text-stone-300 hover:bg-stone-800 hover:text-stone-100 transition-colors"
                    >
                      {t(`platform.${platform}`)} ({ARCH_LABELS[a.arch] ?? a.arch})
                    </a>
                  ))}
                </div>
              </details>
            )}
          </div>
          <a href={MS_STORE_URL} target="_blank" rel="noreferrer">
            <img src={msStoreBadgeUrl(lang)} alt={t("download.msStore")} width={180} className="h-auto" />
          </a>
        </div>
        <p className="mt-4 text-xs text-stone-500">{t("hero.license")}</p>
      </div>

      <div className="mt-16">
        <Screenshot
          src="screenshots/replay.webp"
          alt={t("hero.screenshotAlt")}
          placeholder={t("hero.screenshotPlaceholder")}
          note={t("features.placeholderLabel")}
          plain
          className="rounded-lg border border-stone-800"
        />
      </div>
    </section>
  );
}
