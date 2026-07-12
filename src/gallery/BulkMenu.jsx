import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import * as Icon from "./icons.jsx";

const Item = ({ icon: Glyph, danger, onClick, children }) => (
  <button onClick={onClick}
    className={`grid w-full grid-cols-[14px_minmax(0,1fr)] items-center gap-2.5 px-3.5 py-2 text-left text-[13px] transition hover:bg-stone-800 ${
      danger ? "text-red-400" : "text-stone-200"
    }`}
  >
    {Glyph && <Glyph size={14} className="shrink-0" />}
    <span className="min-w-0 truncate">{children}</span>
  </button>
);

const MENU_WIDTH = 224;
const MENU_EST_HEIGHT = 200;

// Right-click menu for a multi-selection — the same actions as the floating
// selection bar, reachable from the cards themselves. `position` is the
// cursor {x,y}; rendered via a portal onto <body> (gallery rows have their
// own `transform`, which breaks `position: fixed`).
export default function BulkMenu({
  t, position, count, canUpload, canDownload, canRemoveDrive, canDeleteLocal,
  onUpload, onDownload, onRemoveDrive, onDeleteLocal, onDelete, onClose,
}) {
  const ref = useRef(null);
  const [measuredHeight, setMeasuredHeight] = useState(null);

  useEffect(() => {
    const onClickOutside = (e) => { if (ref.current && !ref.current.contains(e.target)) onClose(); };
    document.addEventListener("mousedown", onClickOutside);
    return () => document.removeEventListener("mousedown", onClickOutside);
  }, [onClose]);

  useLayoutEffect(() => {
    if (ref.current) setMeasuredHeight(ref.current.getBoundingClientRect().height);
  }, []);

  useEffect(() => {
    const onScroll = () => onClose();
    window.addEventListener("scroll", onScroll, true);
    return () => window.removeEventListener("scroll", onScroll, true);
  }, [onClose]);

  const estHeight = measuredHeight ?? MENU_EST_HEIGHT;
  const left = Math.max(8, Math.min(position.x, window.innerWidth - MENU_WIDTH - 8));
  let top = position.y;
  if (top + estHeight > window.innerHeight - 8) top = Math.max(8, position.y - estHeight);
  top = Math.min(top, window.innerHeight - 8);

  return createPortal(
    <div ref={ref} style={{ position: "fixed", left, top, width: MENU_WIDTH }}
      className="z-[100] overflow-hidden rounded-xl border border-stone-800 bg-stone-900 py-1.5 shadow-2xl" onClick={(e) => e.stopPropagation()}>
      <div className="px-3.5 pb-1.5 pt-1 text-[11px] font-semibold uppercase tracking-wider text-stone-500">
        {t("gallery.selection.selected")(count)}
      </div>
      {(canUpload || canDownload) && (
        <>
          <div className="my-1 h-px bg-stone-800" />
          {canUpload && <Item icon={Icon.Upload} onClick={onUpload}>{t("gallery.selection.uploadToDrive")}</Item>}
          {canDownload && <Item icon={Icon.Download} onClick={onDownload}>{t("gallery.selection.download")}</Item>}
        </>
      )}
      <div className="my-1 h-px bg-stone-800" />
      {/* Same three delete distinctions as the single-card menu: local only
          (keeps Drive), Drive only (keeps local), or everywhere. Same red for
          all — a distinct icon each tells them apart. */}
      {canDeleteLocal && <Item icon={Icon.HardDrive} danger onClick={onDeleteLocal}>{t("gallery.menu.deleteLocal")}</Item>}
      {canRemoveDrive && <Item icon={Icon.Cloud} danger onClick={onRemoveDrive}>{t("gallery.menu.removeDriveCopy")}</Item>}
      <Item icon={Icon.Trash} danger onClick={onDelete}>{t("gallery.menu.deleteBoth")}</Item>
    </div>,
    document.body
  );
}
