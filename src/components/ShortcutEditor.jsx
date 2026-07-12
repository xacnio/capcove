import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { MdTune, MdDeleteOutline, MdReplay } from "react-icons/md";
import {
  Crop, Window, Camera, Monitor, Folder, Link, Cloud, Pencil, Tag,
  Gear, Refresh, Upload, External, Search, Copy, Check, Calendar, HardDrive, LayoutGrid,
} from "../gallery/icons.jsx";
import { Toggle, HotkeyInput, Row, inputCls } from "./settingsUI.jsx";

// A slot is either a recording trigger (capture set, actions = []) or an action
// trigger ("save_replay" / "open_wheel", capture ignored), as one segmented control.
export const SLOT_KINDS = ["record_window", "record_area", "record_monitor", "save_replay", "open_wheel"];

const KIND_ICON = { record_window: Window, record_area: Crop, record_monitor: Monitor, save_replay: MdReplay, open_wheel: LayoutGrid };

export function slotKind(slot) {
  if (slot.actions?.includes("save_replay")) return "save_replay";
  if (slot.actions?.includes("open_wheel")) return "open_wheel";
  return slot.capture;
}

export function applySlotKind(slot, kind) {
  return kind === "save_replay" || kind === "open_wheel"
    ? { ...slot, actions: [kind] }
    : { ...slot, capture: kind, actions: [] };
}

// Icon a shortcut can be tagged with, shown both here and as its button in
// the gallery sidebar. Keyed by string so it round-trips through the Rust
// config as plain JSON.
export const SHORTCUT_ICON_OPTIONS = [
  "crop", "window", "monitor", "replay", "folder", "link", "cloud", "pencil", "tag",
  "gear", "refresh", "upload", "external", "search", "copy", "check", "calendar", "harddrive", "layoutgrid",
];
export const SHORTCUT_ICON = {
  crop: Crop, window: Window, monitor: Monitor, replay: MdReplay, folder: Folder, link: Link, cloud: Cloud, pencil: Pencil, tag: Tag,
  gear: Gear, refresh: Refresh, upload: Upload, external: External, search: Search, copy: Copy, check: Check, calendar: Calendar, harddrive: HardDrive, layoutgrid: LayoutGrid,
};
// Sensible default per slot kind, used when a shortcut hasn't been given its own icon.
export const CAPTURE_TYPE_ICON = { record_window: "window", record_area: "crop", record_monitor: "monitor", save_replay: "replay", open_wheel: "layoutgrid" };

export function captureKey(kind) {
  return "capture" + kind.split("_").map((w) => w[0].toUpperCase() + w.slice(1)).join("");
}

// Shown as the menu-label placeholder so "Auto" isn't a mystery.
function autoLabel(slot, t) {
  return t(`settings.shortcuts.${captureKey(slotKind(slot))}`);
}

// What to actually display for a shortcut (e.g. as a button tooltip) —
// the custom label if set, the auto one otherwise.
export function shortcutLabel(slot, t) {
  return slot.label || autoLabel(slot, t);
}

export function ShortcutCard({ slot, onChange, onRemove, t }) {
  const kind = slotKind(slot);
  const [menuOpen, setMenuOpen] = useState(false);
  const [menuPos, setMenuPos] = useState(null);
  const menuBtnRef = useRef(null);
  const popoverRef = useRef(null);
  const MENU_WIDTH = 288; // w-72

  useEffect(() => {
    if (!menuOpen) return;
    const close = (e) => {
      if (menuBtnRef.current?.contains(e.target)) return;
      if (popoverRef.current?.contains(e.target)) return;
      setMenuOpen(false);
    };
    // Closing on scroll too: the popover is fixed-positioned (portaled to
    // <body>, to escape the shortcuts list's own overflow clipping), so it
    // wouldn't otherwise follow the button if the list scrolls under it.
    const closeOnScroll = () => setMenuOpen(false);
    document.addEventListener("mousedown", close);
    document.addEventListener("scroll", closeOnScroll, true);
    return () => {
      document.removeEventListener("mousedown", close);
      document.removeEventListener("scroll", closeOnScroll, true);
    };
  }, [menuOpen]);

  const handleToggleMenu = () => {
    if (!menuOpen) {
      const rect = menuBtnRef.current.getBoundingClientRect();
      const left = Math.max(8, Math.min(rect.right - MENU_WIDTH, window.innerWidth - MENU_WIDTH - 8));
      const estHeight = 260;
      let top = rect.bottom + 6;
      if (top + estHeight > window.innerHeight - 8) {
        top = rect.top - estHeight - 6;
      }
      top = Math.max(8, Math.min(top, window.innerHeight - estHeight - 8));
      setMenuPos({ left, top });
    }
    setMenuOpen((v) => !v);
  };

  return (
    <div className="rounded-xl border border-stone-700 bg-stone-900/60 p-2.5 flex items-center gap-2">
      {/* Slot kind — icon-only segmented control */}
      <div className="flex shrink-0 gap-0.5 rounded-lg bg-stone-950/60 p-0.5">
        {SLOT_KINDS.map((k) => {
          const Icon = KIND_ICON[k];
          return (
            <button key={k} type="button" title={t(`settings.shortcuts.${captureKey(k)}`)}
              onClick={() => onChange(applySlotKind(slot, k))}
              className={`flex h-7 w-7 items-center justify-center rounded-md transition ${
                kind === k
                  ? "bg-accent-500 text-stone-950"
                  : "text-stone-500 hover:bg-stone-800 hover:text-stone-300"
              }`}
            >
              <Icon size={15} />
            </button>
          );
        })}
      </div>

      <HotkeyInput
        value={slot.combo}
        onChange={(v) => onChange({ ...slot, combo: v })}
        placeholder={t("settings.shortcuts.hint")}
        className="w-28 shrink-0"
      />

      <div className="flex flex-1 items-center justify-center text-xs text-stone-500">
        {t(`settings.shortcuts.${captureKey(kind)}Hint`)}
      </div>

      {/* Advanced settings, tucked behind a single menu */}
      <div className="shrink-0">
        <button ref={menuBtnRef} type="button" title={t("settings.shortcuts.moreOptions")} onClick={handleToggleMenu}
          className={`flex h-7 w-7 items-center justify-center rounded-lg border transition ${
            menuOpen ? "border-stone-500 bg-stone-800 text-stone-200" : "border-stone-700 bg-stone-800 text-stone-400 hover:border-stone-500 hover:text-stone-200"
          }`}
        >
          <MdTune size={15} />
        </button>
        {menuOpen && menuPos && createPortal(
          <div ref={popoverRef}
            style={{ position: "fixed", left: menuPos.left, top: menuPos.top, width: MENU_WIDTH, maxHeight: "calc(100vh - 16px)", overflowY: "auto" }}
            className="z-50 rounded-xl border border-stone-700 bg-stone-900 p-1 shadow-2xl">
            <div className="divide-y divide-stone-800/70 px-2">
              <div className="py-2.5">
                <div className="mb-1.5 text-sm text-stone-200">{t("settings.shortcuts.icon")}</div>
                <div className="flex flex-wrap gap-1">
                  {SHORTCUT_ICON_OPTIONS.map((key) => {
                    const Ico = SHORTCUT_ICON[key];
                    const active = (slot.icon || CAPTURE_TYPE_ICON[kind]) === key;
                    return (
                      <button key={key} type="button" onClick={() => onChange({ ...slot, icon: key })}
                        className={`flex h-7 w-7 items-center justify-center rounded-lg border transition ${
                          active
                            ? "border-accent-500/60 bg-accent-500/20 text-accent-400"
                            : "border-stone-700 bg-stone-800 text-stone-400 hover:border-stone-500 hover:text-stone-200"
                        }`}
                      >
                        <Ico size={14} />
                      </button>
                    );
                  })}
                </div>
              </div>
              {/* Only `RecordWindow`'s own picker overlay actually reads this
                  (see `overlay::trigger`) — every other kind, including the
                  action-only slots, silently ignored it. */}
              {kind === "record_window" && (
                <Row label={t("settings.shortcuts.multiMonitor")}>
                  <Toggle checked={slot.multi_monitor ?? true} onChange={(v) => onChange({ ...slot, multi_monitor: v })} />
                </Row>
              )}
              <Row label={t("settings.shortcuts.showInMenu")}>
                <Toggle checked={slot.show_in_menu} onChange={(v) => onChange({ ...slot, show_in_menu: v })} />
              </Row>
              {slot.show_in_menu && (
                <div className="py-2.5">
                  <input type="text" value={slot.label}
                    placeholder={autoLabel(slot, t)}
                    onChange={(e) => onChange({ ...slot, label: e.target.value })}
                    className={`${inputCls} w-full text-sm`}
                  />
                </div>
              )}
            </div>
            <button type="button" onClick={onRemove}
              className="mt-1 flex w-full items-center gap-1.5 rounded-lg px-2.5 py-2 text-xs font-medium text-red-400/80 transition hover:bg-red-400/10 hover:text-red-400"
            >
              <MdDeleteOutline size={14} /> {t("settings.shortcuts.remove")}
            </button>
          </div>,
          document.body
        )}
      </div>
    </div>
  );
}
