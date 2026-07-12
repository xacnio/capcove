import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { SiYoutube } from "react-icons/si";
import { MdInfoOutline } from "react-icons/md";
import * as Icon from "./icons.jsx";

// Menu items: a leading glyph, text, and an optional trailing glyph on the
// right (submenu chevron, etc.), groups split by dividers. Grid, not flex —
// a flex row's middle child only shrinks below its own text width with
// `min-w-0` in place, and a stray missing one elsewhere kept pushing the
// trailing chevron out past the menu's fixed width. Explicit grid columns
// (fixed icon, `minmax(0,1fr)` text, content-sized trailing slot) can't
// have that problem: the text track is forced to fit, full stop.
const Item = ({ icon: Glyph, accent, danger, className, right, onClick, children }) => (
  <button
    onClick={onClick}
    className={`grid w-full grid-cols-[14px_minmax(0,1fr)_auto] items-center gap-2.5 px-3.5 py-2 text-left text-[13px] transition hover:bg-stone-800 ${
      className ?? (accent ? "text-accent-300 font-medium" : danger ? "text-red-400" : "text-stone-200")
    }`}
  >
    {Glyph && <Glyph size={14} className="shrink-0" />}
    <span className="min-w-0 truncate">{children}</span>
    {right}
  </button>
);

const Divider = () => <div className="my-1 h-px bg-stone-800" />;

const MENU_WIDTH = 224; // w-56
const MENU_EST_HEIGHT = 300; // generous first-paint guess; corrected once measured
// A little shorter than the main menu's own width reads as a clearly
// subordinate flyout rather than a second identical menu.
const TAGS_FLYOUT_WIDTH = 200;
const TAGS_FLYOUT_MAX_HEIGHT = 260;
// Grace period before actually closing on mouse-leave, so moving the cursor
// diagonally from the "Assign Tags" row into the flyout (which sits to its
// side, not directly below) doesn't close it mid-transit.
const HOVER_CLOSE_DELAY_MS = 150;

// `position` is the viewport {x,y} top-left corner. Rendered via a portal onto <body>
// since virtualized gallery rows have their own `transform`, which breaks `position: fixed`.
export default function CardMenu({
  t, tags, assigned, onToggleTag, onOpenEditor, onUploadYoutube, onOpenYoutube, onOpenYoutubeStudio, onCreateYoutubeClip, onReveal, onShowInFolderView, onShowDetails, onDelete, onDeleteDriveCopy, onDeleteBoth, onClose, position, onOpenExternal,
  onUploadDrive, onDownload, onCopyDriveLink, onOpenDrive, driveConnected, driveSynced, driveOnly, backedUp, hasLocalFile = true, permanent = false,
}) {
  const ref = useRef(null);
  const tagsRowRef = useRef(null);
  const flyoutRef = useRef(null);
  const [measuredHeight, setMeasuredHeight] = useState(null);
  // `null` = closed; otherwise the flyout's computed fixed-viewport position.
  const [tagsFlyout, setTagsFlyout] = useState(null);
  const hoverTimerRef = useRef(null);

  useEffect(() => {
    const onClickOutside = (e) => {
      if (ref.current?.contains(e.target) || flyoutRef.current?.contains(e.target)) return;
      onClose();
    };
    document.addEventListener("mousedown", onClickOutside);
    return () => document.removeEventListener("mousedown", onClickOutside);
  }, [onClose]);

  // `MENU_EST_HEIGHT` is only a worst-case guess for the first paint; once the
  // real menu has a measured size, use that so the menu isn't flipped upward
  // unnecessarily.
  useLayoutEffect(() => {
    if (ref.current) setMeasuredHeight(ref.current.getBoundingClientRect().height);
  }, []);

  useEffect(() => () => clearTimeout(hoverTimerRef.current), []);
  // The flyout is a separate `<body>` portal (not a child of the menu), so
  // the menu's own `overflow` can't clip it — the bug every CSS-only attempt
  // hit. Its position is measured from the tags row and clamped to the
  // viewport, opening on whichever side / vertical offset actually fits.
  const openTagsFlyout = () => {
    clearTimeout(hoverTimerRef.current);
    const row = tagsRowRef.current?.getBoundingClientRect();
    if (!row) return;
    const openLeft = row.right + TAGS_FLYOUT_WIDTH > window.innerWidth - 8;
    const fLeft = openLeft
      ? Math.max(8, row.left - TAGS_FLYOUT_WIDTH - 4)
      : Math.min(row.right + 4, window.innerWidth - TAGS_FLYOUT_WIDTH - 8);
    // Estimated from the actual tag count (not always the worst-case max
    // height) — clamping to the max height unconditionally pushed the
    // flyout way above the row whenever there were only a few tags,
    // leaving a gap the mouse had to cross (and often failed to, closing
    // the flyout) to get from the row to it.
    const estHeight = Math.min(TAGS_FLYOUT_MAX_HEIGHT, 12 + Math.max(1, tags.length) * 38);
    const fTop = Math.min(row.top, window.innerHeight - estHeight - 8);
    setTagsFlyout({ left: fLeft, top: Math.max(8, fTop) });
  };
  const scheduleCloseTagsFlyout = () => {
    clearTimeout(hoverTimerRef.current);
    hoverTimerRef.current = setTimeout(() => setTagsFlyout(null), HOVER_CLOSE_DELAY_MS);
  };
  const cancelCloseTagsFlyout = () => clearTimeout(hoverTimerRef.current);

  // The menu is a portal at a fixed screen position, so it won't move with
  // the gallery's scroll container; close on scroll like a native context
  // menu would. Capture phase since "scroll" doesn't bubble.
  useEffect(() => {
    const onScroll = () => onClose();
    window.addEventListener("scroll", onScroll, true);
    return () => window.removeEventListener("scroll", onScroll, true);
  }, [onClose]);

  // Clamp to stay fully on screen, flipping upward near the bottom edge —
  // same approach as the shortcuts editor's advanced-options popover.
  const estHeight = measuredHeight ?? MENU_EST_HEIGHT;
  const left = Math.max(8, Math.min(position.x, window.innerWidth - MENU_WIDTH - 8));
  let top = position.y;
  if (top + estHeight > window.innerHeight - 8) top = Math.max(8, position.y - estHeight);
  top = Math.min(top, window.innerHeight - 8);

  return createPortal(
    <>
    <div ref={ref} style={{ position: "fixed", left, top, width: MENU_WIDTH, maxHeight: "calc(100vh - 16px)", overflowX: "hidden", overflowY: "auto", scrollbarGutter: "stable" }}
      className="z-[100] rounded-xl border border-stone-800 bg-stone-900 py-1.5 shadow-2xl" onClick={(e) => e.stopPropagation()}>
          {/* Virtual YouTube-live entries and Drive-only cards have no local
              file: no upload/edit/reveal — just their one primary action
              (open on YouTube, or download from Drive), tags, and delete.
              A local file already uploaded to YouTube (`onOpenYoutube`) opens
              its video instead of offering to upload it again. */}
          {onOpenExternal
            ? <Item icon={SiYoutube} accent onClick={onOpenExternal}>{t("videoEditor.youtubeModal.openVideo")}</Item>
            : driveOnly
              ? <Item icon={Icon.Download} accent onClick={onDownload}>{t("gallery.menu.downloadToDevice")}</Item>
              : onOpenYoutube
                ? <Item icon={SiYoutube} accent onClick={onOpenYoutube}>{t("videoEditor.youtubeModal.openVideo")}</Item>
                : <Item icon={SiYoutube} accent onClick={onUploadYoutube}>{t("videoEditor.uploadToYoutube")}</Item>}
          {onOpenYoutubeStudio && (
            <Item icon={SiYoutube} onClick={onOpenYoutubeStudio}>{t("gallery.menu.openYoutubeStudio")}</Item>
          )}
          {onCreateYoutubeClip && (
            <Item icon={Icon.Crop} onClick={onCreateYoutubeClip}>{t("gallery.menu.createYoutubeClip")}</Item>
          )}
          {!onOpenExternal && !driveOnly && driveConnected && !driveSynced && (
            <Item icon={Icon.CloudUpload} onClick={onUploadDrive}>{t("gallery.menu.uploadDrive")}</Item>
          )}
          {onOpenDrive && (
            <Item icon={Icon.External} onClick={onOpenDrive}>{t("gallery.menu.openInDrive")}</Item>
          )}
          {!onOpenExternal && driveSynced && (
            <Item icon={Icon.Link} onClick={onCopyDriveLink}>{t("gallery.menu.copyDriveLink")}</Item>
          )}
          {/* Three delete actions can appear together (local / Drive / both) —
              same red (they're all destructive) but a distinct icon each so
              they're still tellable apart at a glance without turning the
              menu into a rainbow of unrelated colors. */}
          <Item icon={hasLocalFile && !driveOnly ? Icon.HardDrive : Icon.Trash} danger onClick={onDelete}>
            {(driveOnly ? t("gallery.menu.deleteFromDrive")
              : !hasLocalFile ? t("gallery.menu.removeFromList")
              : backedUp ? t("gallery.menu.deleteLocal")
              : t("common.delete")) + (permanent && hasLocalFile ? t("gallery.menu.permanentSuffix") : "")}
          </Item>
          {/* The inverse of "Delete local copy": drop just the Drive backup,
              keep the file on this machine. Only makes sense when both
              copies actually exist. */}
          {driveSynced && hasLocalFile && !driveOnly && (
            <Item icon={Icon.Cloud} danger onClick={onDeleteDriveCopy}>{t("gallery.menu.removeDriveCopy")}</Item>
          )}
          {/* Both copies in one action, instead of clicking the two above
              separately. Its own divider marks it as the bigger, final
              action without needing a different color to say so. */}
          {driveSynced && hasLocalFile && !driveOnly && (
            <>
              <Divider />
              <Item icon={Icon.Trash} danger onClick={onDeleteBoth}>
                {t("gallery.menu.deleteBoth") + (permanent ? t("gallery.menu.permanentSuffix") : "")}
              </Item>
            </>
          )}
          <Divider />
          {/* Hover (not click) opens the tags flyout beside this row,
              matching a native context menu's submenu — no more replacing
              the whole panel with a "back"-button tags-only screen. */}
          <div ref={tagsRowRef} onMouseEnter={openTagsFlyout} onMouseLeave={scheduleCloseTagsFlyout}>
            <Item icon={Icon.Tag} right={<Icon.ChevronRight size={13} className="shrink-0 text-stone-500" />}>
              {t("gallery.tags.assignTags")}
            </Item>
          </div>
          {/* Available regardless of `hasLocalFile` — a Drive-only card still
              has app/folder/date/size/tags worth surfacing. */}
          {onShowDetails && (
            <Item icon={MdInfoOutline} onClick={onShowDetails}>{t("gallery.menu.details")}</Item>
          )}
          {hasLocalFile && (
            <>
              <Divider />
              {onOpenEditor && <Item icon={Icon.Pencil} onClick={onOpenEditor}>{t("gallery.video.edit")}</Item>}
              <Item icon={Icon.Folder} onClick={onReveal}>{t("gallery.menu.reveal")}</Item>
              {/* Only offered from the flat "All Videos" view — jumps to
                  this video's actual game/folder in the Folders browser. */}
              {onShowInFolderView && (
                <Item icon={Icon.Folders} onClick={onShowInFolderView}>{t("gallery.menu.showInFolderView")}</Item>
              )}
            </>
          )}
    </div>

    {tagsFlyout && (
      <div ref={flyoutRef}
        style={{ position: "fixed", left: tagsFlyout.left, top: tagsFlyout.top, width: TAGS_FLYOUT_WIDTH, maxHeight: TAGS_FLYOUT_MAX_HEIGHT }}
        className="z-[101] overflow-x-hidden overflow-y-auto rounded-xl border border-stone-800 bg-stone-900 py-1.5 shadow-2xl"
        onMouseEnter={cancelCloseTagsFlyout} onMouseLeave={scheduleCloseTagsFlyout}
        onClick={(e) => e.stopPropagation()}
      >
        {tags.length === 0 ? (
          <div className="px-3.5 py-2 text-[12px] text-stone-600">{t("gallery.tags.noTags")}</div>
        ) : (
          tags.map((tag) => {
            const isOn = assigned.includes(tag.id);
            return (
              <button key={tag.id} onClick={() => onToggleTag(tag.id)}
                className="grid w-full grid-cols-[10px_minmax(0,1fr)_auto] items-center gap-2.5 px-3.5 py-2 text-left text-[13px] text-stone-200 transition hover:bg-stone-800">
                <span className="h-2.5 w-2.5 shrink-0 rounded-full" style={{ backgroundColor: tag.color }} />
                <span className="min-w-0 truncate">{tag.name}</span>
                {isOn && <Icon.Check2 size={13} className="shrink-0 text-accent-400" />}
              </button>
            );
          })
        )}
      </div>
    )}
    </>,
    document.body
  );
}
