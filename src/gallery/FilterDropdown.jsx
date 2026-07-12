import { useEffect, useRef, useState } from "react";
import * as Icon from "./icons.jsx";

// Compact dropdown used across the gallery toolbar. In filter mode
// (allowNull) `value === null` means "all"; in sort mode it always has a value.
export default function FilterDropdown({ icon, allLabel, options, value, onChange, allowNull = true, title }) {
  const [open, setOpen] = useState(false);
  const ref = useRef(null);

  useEffect(() => {
    if (!open) return;
    const onClickOutside = (e) => { if (ref.current && !ref.current.contains(e.target)) setOpen(false); };
    document.addEventListener("mousedown", onClickOutside);
    return () => document.removeEventListener("mousedown", onClickOutside);
  }, [open]);

  const active = options.find((o) => o.value === value);
  const showLabel = allowNull && active;
  const Icn = icon;

  return (
    <div className="relative" ref={ref}>
      <button
        onClick={() => setOpen((o) => !o)}
        title={title ?? allLabel}
        className={`flex items-center gap-1 rounded-lg px-2 py-1.5 text-xs transition ${
          allowNull && value !== null
            ? "bg-accent-500/15 text-accent-300"
            : "text-stone-400 hover:bg-stone-800 hover:text-stone-200"
        }`}
      >
        <Icn size={15} />
        {showLabel && <span className="max-w-[90px] truncate">{active.label}</span>}
        <Icon.ChevronRight size={10} className={`transition-transform ${open ? "-rotate-90" : "rotate-90"}`} />
      </button>
      {open && (
        <div className="absolute left-0 top-9 z-30 max-h-72 w-48 overflow-y-auto rounded-lg border border-stone-700 bg-stone-900 p-1.5 shadow-xl">
          {allowNull && (
            <button
              onClick={() => { onChange(null); setOpen(false); }}
              className={`flex w-full items-center gap-2 truncate rounded-md px-2 py-1.5 text-left text-xs transition ${
                value === null ? "bg-accent-500/15 text-accent-300" : "text-stone-300 hover:bg-stone-800"
              }`}
            >
              {allLabel}
            </button>
          )}
          {options.length === 0 ? (
            <div className="px-2 py-1.5 text-[11px] text-stone-600">—</div>
          ) : (
            options.map((o) => (
              <button
                key={o.value}
                onClick={() => { onChange(o.value); setOpen(false); }}
                className={`flex w-full items-center gap-2 truncate rounded-md px-2 py-1.5 text-left text-xs transition ${
                  value === o.value ? "bg-accent-500/15 text-accent-300" : "text-stone-300 hover:bg-stone-800"
                }`}
              >
                {o.color && <span className="h-2 w-2 shrink-0 rounded-full" style={{ backgroundColor: o.color }} />}
                {o.icon && <img src={o.icon} alt="" className="h-4 w-4 rounded" />}
                <span className="truncate">{o.label}</span>
              </button>
            ))
          )}
        </div>
      )}
    </div>
  );
}
