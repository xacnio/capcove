// Inline SVG flags — Windows/WebView2 doesn't render country-flag emoji, so
// the language picker ships its own. Self-contained, no network/font needed.

function GbFlag() {
  return (
    <svg viewBox="0 0 60 30" className="h-full w-full" preserveAspectRatio="none">
      <clipPath id="flag-gb-t">
        <path d="M30,15 h30 v15 z v-15 h-30 z h-30 v-15 z v15 h30 z" />
      </clipPath>
      <path d="M0,0 v30 h60 v-30 z" fill="#012169" />
      <path d="M0,0 L60,30 M60,0 L0,30" stroke="#fff" strokeWidth="6" />
      <path d="M0,0 L60,30 M60,0 L0,30" clipPath="url(#flag-gb-t)" stroke="#C8102E" strokeWidth="4" />
      <path d="M30,0 v30 M0,15 h60" stroke="#fff" strokeWidth="10" />
      <path d="M30,0 v30 M0,15 h60" stroke="#C8102E" strokeWidth="6" />
    </svg>
  );
}

function TrFlag() {
  return (
    <svg viewBox="0 0 30 20" className="h-full w-full" preserveAspectRatio="none">
      <rect width="30" height="20" fill="#E30A17" />
      <circle cx="11.25" cy="10" r="5" fill="#fff" />
      <circle cx="12.5" cy="10" r="4" fill="#E30A17" />
      <polygon
        fill="#fff"
        points="17.5,7.8 18.03,9.27 19.59,9.32 18.36,10.28 18.79,11.78 17.5,10.9 16.21,11.78 16.64,10.28 15.41,9.32 16.97,9.27"
      />
    </svg>
  );
}

export function Flag({ code, className = "" }) {
  return (
    <span className={`inline-block h-3.5 w-5 shrink-0 overflow-hidden rounded-sm ring-1 ring-black/20 ${className}`}>
      {code === "tr" ? <TrFlag /> : <GbFlag />}
    </span>
  );
}
