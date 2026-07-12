// Shared UI primitives reused by Settings and the onboarding wizard.

export const KEY_NAMES = {
  " ": "Space",
  ArrowUp: "Up", ArrowDown: "Down", ArrowLeft: "Left", ArrowRight: "Right",
};

const isMac = typeof navigator !== "undefined" && /mac/i.test(navigator.platform || navigator.userAgent || "");

export function comboFromEvent(e) {
  const parts = [];
  if (e.ctrlKey)  parts.push("Ctrl");
  if (e.altKey)   parts.push("Alt");
  if (e.shiftKey) parts.push("Shift");
  if (e.metaKey)  parts.push(isMac ? "Cmd" : "Super");
  let key = e.key;
  if (["Control", "Alt", "Shift", "Meta"].includes(key)) return null;
  if (KEY_NAMES[key]) key = KEY_NAMES[key];
  if (key.length === 1) key = key.toUpperCase();
  if (e.code && e.code.startsWith("Digit")) key = e.code.slice(5);
  if (e.code && e.code.startsWith("Key"))   key = e.code.slice(3);
  parts.push(key);
  return parts.join("+");
}

// Ctrl keeps its name instead of "⌃" — that glyph renders as a near-invisible
// sliver in most fonts.
const MAC_SYMBOLS = { Cmd: "⌘", Ctrl: "Control", Alt: "⌥", Shift: "⇧" };

// macOS: show modifier symbols instead of names (e.g. "⌘+⇧+1").
// Other platforms keep the textual "Ctrl+Shift+1" form.
export function formatCombo(combo) {
  if (!isMac || !combo) return combo;
  return combo
    .split("+")
    .map((part) => MAC_SYMBOLS[part] ?? part)
    .join("+");
}

export const inputCls =
  "rounded-lg border border-stone-700 bg-stone-800 px-3 py-1.5 text-sm text-stone-100 outline-none transition focus:border-accent-500 placeholder:text-stone-500";

// `labeled` is accepted (and ignored) for compatibility: it used to add an
// "ON"/"OFF" caption, dropped as visual noise since the switch's
// color/position already says the same thing.
export function Toggle({ checked, onChange, labeled: _labeled = false }) {
  return (
    <button type="button" role="switch" aria-checked={checked} onClick={() => onChange(!checked)}
      className={`relative h-6 w-11 shrink-0 rounded-full transition ${checked ? "bg-accent-500" : "bg-stone-700"}`}>
      <span className={`absolute left-0.5 top-0.5 h-5 w-5 rounded-full bg-white shadow transition ${checked ? "translate-x-5" : ""}`} />
    </button>
  );
}

export function Radio({ checked, onChange, className = "" }) {
  return (
    <button type="button" role="radio" aria-checked={checked} onClick={onChange}
      className={`flex h-4 w-4 shrink-0 items-center justify-center rounded-full border transition ${checked ? "border-accent-500" : "border-stone-600"} ${className}`}>
      <span className={`h-2 w-2 rounded-full bg-accent-500 transition-transform ${checked ? "scale-100" : "scale-0"}`} />
    </button>
  );
}

export function Row({ label, hint, children }) {
  return (
    <div className="flex items-center justify-between gap-4 py-3">
      <div className="min-w-0">
        <div className="text-sm text-stone-200">{label}</div>
        {hint && <div className="mt-0.5 text-xs text-stone-500">{hint}</div>}
      </div>
      <div className="flex shrink-0 items-center gap-2">{children}</div>
    </div>
  );
}

export function Card({ title, right, children }) {
  return (
    <section className="rounded-xl border border-stone-800 bg-stone-900">
      {title && (
        <div className="flex items-center justify-between border-b border-stone-800 px-4 py-3">
          <h2 className="text-sm font-semibold text-stone-100">{title}</h2>
          {right}
        </div>
      )}
      <div className="divide-y divide-stone-800/70 px-4">{children}</div>
    </section>
  );
}

export function Button({ variant = "default", className = "", ...props }) {
  const v = {
    default: "bg-stone-800 text-stone-200 hover:bg-stone-700",
    primary: "bg-accent-400 text-stone-950 hover:bg-accent-300",
    danger:  "bg-red-600/90 text-white hover:bg-red-600",
  }[variant];
  return (
    <button className={`rounded-lg px-3.5 py-2 text-sm font-medium transition disabled:opacity-50 ${v} ${className}`} {...props} />
  );
}

// One quality knob as a mini stat tile: dim "default" until overridden,
// accent when set. A picker via the invisible stretched <select>. Shared by
// the per-game overrides panel and the replay buffer's own video-override
// panel — same "default = inherit, pick a value = override" knob.
export function OverrideTile({ label, display, overridden, value, onChange, children, defaultLabel }) {
  return (
    <div className={`relative flex min-w-0 flex-col justify-center gap-px rounded-lg border px-2 py-1.5 transition ${
      overridden ? "border-accent-500/40 bg-accent-500/5" : "border-stone-800 bg-stone-950 hover:border-stone-600"
    }`}>
      <span className="truncate text-[9px] font-medium uppercase tracking-wider text-stone-600">{label}</span>
      <span className={`truncate text-[11px] font-semibold ${overridden ? "text-accent-300" : "text-stone-500"}`}>
        {overridden ? display : defaultLabel}
      </span>
      {/* colors matter even at opacity-0 — the popup list inherits them */}
      <select value={value ?? ""} onChange={(e) => onChange(e.target.value === "" ? null : e.target.value)}
        className="absolute inset-0 h-full w-full cursor-pointer bg-stone-800 text-stone-100 opacity-0">
        {children}
      </select>
    </div>
  );
}

export function HotkeyInput({ value, onChange, placeholder, className = "w-44" }) {
  const onKeyDown = (e) => {
    e.preventDefault();
    e.stopPropagation();
    if (e.key === "Backspace" || e.key === "Delete") { onChange(""); return; }
    if (e.key === "Escape") { e.target.blur(); return; }
    const combo = comboFromEvent(e);
    if (!combo) return;
    onChange(combo);
    e.target.blur();
  };
  return (
    <input readOnly value={formatCombo(value)} placeholder={placeholder} onKeyDown={onKeyDown}
      className={`${inputCls} ${className} cursor-pointer text-center caret-transparent focus:bg-stone-700`} />
  );
}
