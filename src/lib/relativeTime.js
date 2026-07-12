const UNITS_EN = [
  [60, "second", "seconds"],
  [60, "minute", "minutes"],
  [24, "hour", "hours"],
  [7, "day", "days"],
  [4.345, "week", "weeks"],
  [12, "month", "months"],
  [Infinity, "year", "years"],
];

const UNITS_TR = [
  [60, "saniye"],
  [60, "dakika"],
  [24, "saat"],
  [7, "gün"],
  [4.345, "hafta"],
  [12, "ay"],
  [Infinity, "yıl"],
];

export function relativeTime(ms, lang) {
  if (!ms) return "";
  let diff = Math.max(0, (Date.now() - ms) / 1000);
  if (diff < 5) return lang === "tr" ? "az önce" : "just now";

  // Calendar-day special cases ("Today"/"Yesterday").
  const then = new Date(ms);
  const today = new Date();
  const startOfToday = new Date(today.getFullYear(), today.getMonth(), today.getDate()).getTime();
  const dayMs = 24 * 60 * 60 * 1000;
  if (ms >= startOfToday && diff >= 3600) {
    return lang === "tr" ? "Bugün" : "Today";
  }
  if (ms >= startOfToday - dayMs && ms < startOfToday) {
    return lang === "tr" ? "Dün" : "Yesterday";
  }

  if (lang === "tr") {
    for (const [size, unit] of UNITS_TR) {
      if (diff < size) {
        const v = Math.max(1, Math.floor(diff));
        return `${v} ${unit} önce`;
      }
      diff /= size;
    }
  } else {
    for (const [size, singular, plural] of UNITS_EN) {
      if (diff < size) {
        const v = Math.max(1, Math.floor(diff));
        return `${v} ${v === 1 ? singular : plural} ago`;
      }
      diff /= size;
    }
  }
  return "";
}
