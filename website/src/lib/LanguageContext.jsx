import { createContext, useContext, useMemo, useState } from "react";
import { useT } from "./i18n.js";
import { homePath } from "./routes.js";

const STORAGE_KEY = "capcove-website-lang";
const LanguageContext = createContext(null);

function detectBrowserLanguage() {
  if (typeof navigator === "undefined") return "en";
  return navigator.language?.toLowerCase().startsWith("tr") ? "tr" : "en";
}

function initialLanguage() {
  if (typeof localStorage === "undefined") return detectBrowserLanguage();
  // A manual choice always wins over the browser language on later visits.
  return localStorage.getItem(STORAGE_KEY) ?? detectBrowserLanguage();
}

// `routeLang` locks the provider to a static /en/ or /tr/ page, so switching
// language navigates to the sibling page instead of just swapping text.
// Pages without a fixed route fall back to the localStorage-backed switch.
export function LanguageProvider({ children, routeLang }) {
  const [lang, setLangState] = useState(routeLang ?? initialLanguage);
  const t = useT(lang);

  const setLang = (next) => {
    if (typeof localStorage !== "undefined") localStorage.setItem(STORAGE_KEY, next);
    if (routeLang) {
      window.location.href = homePath(next) + window.location.hash;
      return;
    }
    setLangState(next);
  };

  const value = useMemo(() => ({ lang, setLang, t }), [lang]);
  return <LanguageContext.Provider value={value}>{children}</LanguageContext.Provider>;
}

export function useLanguage() {
  const ctx = useContext(LanguageContext);
  if (!ctx) throw new Error("useLanguage must be used within LanguageProvider");
  return ctx;
}
