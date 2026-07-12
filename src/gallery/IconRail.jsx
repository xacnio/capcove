import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { SiGoogledrive } from "react-icons/si";
import { invoke, listen } from "../lib/tauri.js";
import * as Icon from "./icons.jsx";

// Rail button: the active item's circle is a shared, separately-animated
// pill (see `IconRail`'s `pillTop`) that slides behind whichever button is
// active, rather than each button drawing its own background — this button
// only ever needs to swap its glyph color. `z-10` keeps the glyph itself
// (and the hover background of any other, non-active button) painting above
// that shared pill regardless of DOM order.
function RailButton({ buttonRef, title, active, onClick, children }) {
  return (
    <button
      ref={buttonRef}
      title={title}
      onClick={onClick}
      className={`relative z-10 flex h-11 w-11 items-center justify-center rounded-full transition-colors duration-200 active:scale-[0.94] active:duration-75 ${
        active ? "text-stone-900" : "text-stone-500 hover:bg-stone-800 hover:text-stone-200"
      }`}
    >
      {children}
    </button>
  );
}

function fmtBytes(bytes) {
  if (!bytes) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let v = bytes, i = 0;
  while (v >= 1024 && i < units.length - 1) { v /= 1024; i++; }
  return `${v.toFixed(i === 0 ? 0 : 1)} ${units[i]}`;
}

// One labeled usage ring (local disk, or Google Drive). Visible track, a
// usage-colored fill (green→amber→red), the percentage centered in the ring
// when there's a finite limit, and used/limit text below. `icon` distinguishes
// the two at a glance.
function StorageRing({ icon: Glyph, usedBytes, limitBytes, limitLabel, onClick, title }) {
  const ratio = limitBytes ? Math.min(1, usedBytes / limitBytes) : 0;
  const fillCls = limitBytes == null ? "text-accent-400"
    : ratio > 0.9 ? "text-red-500" : ratio > 0.7 ? "text-amber-500" : "text-emerald-500";
  const R = 16;
  const C = 2 * Math.PI * R;
  return (
    <button type="button" onClick={onClick} title={title}
      className="flex w-full flex-col items-center gap-0.5 rounded-lg py-1 text-center transition hover:bg-stone-800/60">
      <div className="relative">
        <svg width="42" height="42" viewBox="0 0 42 42" className="-rotate-90">
          <circle cx="21" cy="21" r={R} fill="none" stroke="currentColor" strokeWidth="3.5" className="text-stone-700" />
          <circle cx="21" cy="21" r={R} fill="none" stroke="currentColor" strokeWidth="3.5"
            strokeLinecap="round" strokeDasharray={C} strokeDashoffset={C * (1 - ratio)}
            className={`${fillCls} transition-[stroke-dashoffset] duration-500`} />
        </svg>
        {/* Icon centered in the ring — distinguishes local vs Drive without
            crowding the size text; the colored arc already shows how full. */}
        <span className="absolute inset-0 flex items-center justify-center text-stone-400">
          <Glyph size={15} />
        </span>
      </div>
      <div className="text-[10px] font-semibold text-stone-200">{fmtBytes(usedBytes)}</div>
      <div className="text-[9px] text-stone-500">{limitLabel}</div>
    </button>
  );
}

// Bottom-left storage widget: two rings — local recordings usage (against the
// Settings → Storage limit) and Google Drive quota usage. Both open the
// Storage settings page when clicked.
function StorageIndicator({ t, settings, onOpenStorage }) {
  const [info, setInfo] = useState(null);

  useEffect(() => {
    const load = () => invoke("get_storage_info").then(setInfo).catch(() => {});
    load();
    let unlisten = [];
    (async () => {
      unlisten.push(await listen("video-saved", load));
      unlisten.push(await listen("drive-capacity", load));
      unlisten.push(await listen("library-changed", load));
    })();
    return () => unlisten.forEach((u) => u?.());
  }, []);

  if (!info) return null;
  const limitMb = settings?.storage_limit_mb ?? null;
  const localLimit = limitMb != null ? limitMb * 1024 * 1024 : null;
  const driveConnected = info.drive_usage > 0 || info.drive_limit != null;

  return (
    <div className="flex w-full flex-col items-center gap-1">
      <StorageRing
        icon={Icon.HardDrive}
        usedBytes={info.local_bytes}
        limitBytes={localLimit}
        limitLabel={localLimit != null ? fmtBytes(localLimit) : t("gallery.rail.noLimit")}
        title={`${t("gallery.rail.local")}: ${fmtBytes(info.local_bytes)}${localLimit != null ? ` / ${fmtBytes(localLimit)}` : ""}`}
        onClick={onOpenStorage}
      />
      {driveConnected && (
        <StorageRing
          icon={SiGoogledrive}
          usedBytes={info.drive_usage}
          limitBytes={info.drive_limit}
          limitLabel={info.drive_limit != null ? fmtBytes(info.drive_limit) : t("gallery.rail.noLimit")}
          title={`Drive: ${fmtBytes(info.drive_usage)}${info.drive_limit != null ? ` / ${fmtBytes(info.drive_limit)}` : ""}`}
          onClick={onOpenStorage}
        />
      )}
    </div>
  );
}

export default function IconRail({ t, view, settings, onNavigate, onOpenStorage }) {
  const railRef = useRef(null);
  const foldersRef = useRef(null);
  const galleryRef = useRef(null);
  const settingsRef = useRef(null);
  const [pillTop, setPillTop] = useState(null);

  const activeRef = view === "folders" ? foldersRef : view === "gallery" ? galleryRef : view === "settings" ? settingsRef : null;

  // Measured (not CSS-percentage-based) since the active button can be the
  // top group (Folders/Gallery) or, past the spacer and storage widget,
  // Settings at the bottom — a fixed offset can't span both.
  useLayoutEffect(() => {
    setPillTop(activeRef?.current ? activeRef.current.offsetTop : null);
  }, [view]);

  return (
    <div ref={railRef} className="relative flex w-16 shrink-0 flex-col items-center gap-2 border-r border-stone-800/60 py-4">
      {/* Shared active-item background: slides to whichever button is
          active instead of each button owning its own instantly-toggled
          background. Rendered first so it stacks under every button
          (`z-10` on `RailButton`) regardless of position. */}
      {pillTop != null && (
        <div
          className="pointer-events-none absolute left-1/2 top-0 h-11 w-11 rounded-full bg-stone-100 transition-transform duration-300 ease-out"
          style={{ transform: `translate(-50%, ${pillTop}px)` }}
        />
      )}
      {/* Distinct icons on purpose — this rail already has a plain single
          Folder glyph below for "Open Folder", so "Folders" (the
          games/folders explorer) uses the stacked-folders glyph instead.
          "Gallery" (flat, every video) gets the grid icon, matching the
          breadcrumb's own "All Videos" label. */}
      <RailButton buttonRef={foldersRef} title={t("gallery.rail.folders")} active={view === "folders"} onClick={() => onNavigate("folders")}>
        <Icon.Folders size={19} />
      </RailButton>
      <RailButton buttonRef={galleryRef} title={t("gallery.rail.gallery")} active={view === "gallery"} onClick={() => onNavigate("gallery")}>
        <Icon.LayoutGrid size={19} />
      </RailButton>
      <div className="flex-1" />
      <StorageIndicator t={t} settings={settings} onOpenStorage={onOpenStorage} />
      <RailButton title={t("gallery.sidebar.openFolder")} onClick={() => invoke("open_videos_folder")}>
        <Icon.Folder size={19} />
      </RailButton>
      <RailButton buttonRef={settingsRef} title={t("gallery.sidebar.settings")} active={view === "settings"} onClick={() => onNavigate("settings")}>
        <Icon.Gear size={19} />
      </RailButton>
    </div>
  );
}
