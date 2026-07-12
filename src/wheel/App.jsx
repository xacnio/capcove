import { useEffect, useMemo, useRef, useState } from "react";
import { invoke, listen, emit, convertFileSrc } from "../lib/tauri.js";
import { createT } from "../lib/i18n.js";
import { useClockSeconds } from "../lib/useClockSeconds.js";
import { useAppIcon } from "../gallery/appIcons.js";
import logo from "../assets/logo.png";

// Radial menu: every capture action as a wedge, reflecting live record/buffer
// state. Root ring stays short — less-common choices and Settings live in
// branches (see `BRANCH_PARENT`/`branches`), swapping the same ring in place.

const SIZE = 560;
const CX = SIZE / 2;
const CY = SIZE / 2;
const R_OUTER = 258;
const R_INNER = 92;
const R_MID = (R_OUTER + R_INNER) / 2;

function polar(angleDeg, r) {
  const a = ((angleDeg - 90) * Math.PI) / 180;
  return [CX + r * Math.cos(a), CY + r * Math.sin(a)];
}

function wedgePath(startDeg, endDeg) {
  const [x1, y1] = polar(startDeg, R_OUTER);
  const [x2, y2] = polar(endDeg, R_OUTER);
  const [x3, y3] = polar(endDeg, R_INNER);
  const [x4, y4] = polar(startDeg, R_INNER);
  return `M ${x1} ${y1} A ${R_OUTER} ${R_OUTER} 0 0 1 ${x2} ${y2} L ${x3} ${y3} A ${R_INNER} ${R_INNER} 0 0 0 ${x4} ${y4} Z`;
}

// Full-length spoke at a wedge boundary — the only separator between wedges.
function divider(angleDeg) {
  const [ox, oy] = polar(angleDeg, R_OUTER);
  const [ix, iy] = polar(angleDeg, R_INNER);
  return [ix, iy, ox, oy];
}

function fmtClock(secs) {
  const m = Math.floor(secs / 60);
  const s = secs % 60;
  return `${m}:${s.toString().padStart(2, "0")}`;
}

function SaveIcon() {
  return <path d="M19 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h11l5 5v11a2 2 0 0 1-2 2zM17 21v-8H7v8M7 3v5h8" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" />;
}
function RecordIcon() {
  return <><circle cx="12" cy="12" r="9" fill="none" stroke="currentColor" strokeWidth="2" /><circle cx="12" cy="12" r="4" fill="currentColor" /></>;
}
function StopIcon() {
  return <><circle cx="12" cy="12" r="9" fill="none" stroke="currentColor" strokeWidth="2" /><rect x="8.5" y="8.5" width="7" height="7" rx="1" fill="currentColor" /></>;
}
// Record dot + a small YouTube play-triangle badge — the "also stream live" icon.
function RecordLiveIcon() {
  return (
    <>
      <circle cx="11" cy="12" r="8.5" fill="none" stroke="currentColor" strokeWidth="2" />
      <circle cx="11" cy="12" r="3.5" fill="currentColor" />
      <circle cx="19" cy="18" r="5.5" fill="#dc2626" stroke="#0c0c0c" strokeWidth="1.5" />
      <path d="M17.3 15.7v4.6l4-2.3z" fill="#fff" />
    </>
  );
}
function BufferIcon() {
  return <><path d="M3 12a9 9 0 1 1 3 6.7" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" /><path d="M3 22v-4h4" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" /><path d="M12 7v5l3 3" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" /></>;
}
function WindowIcon() {
  return <><rect x="2" y="4" width="20" height="16" rx="2" fill="none" stroke="currentColor" strokeWidth="2" /><line x1="2" y1="9" x2="22" y2="9" stroke="currentColor" strokeWidth="2" /></>;
}
function AreaIcon() {
  return <path d="M6 2v14a2 2 0 0 0 2 2h14M18 22V8a2 2 0 0 0-2-2H2" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" />;
}
function GalleryIcon() {
  return <><rect x="3" y="3" width="18" height="18" rx="2" fill="none" stroke="currentColor" strokeWidth="2" /><circle cx="8.5" cy="8.5" r="1.5" fill="currentColor" /><path d="m21 15-5-5L5 21" fill="none" stroke="currentColor" strokeWidth="2" /></>;
}
function SettingsIcon() {
  return <><circle cx="12" cy="12" r="3" fill="none" stroke="currentColor" strokeWidth="2" /><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09a1.65 1.65 0 0 0-1.08-1.51 1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09a1.65 1.65 0 0 0 1.51-1.08 1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" fill="none" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" strokeLinejoin="round" /></>;
}
// Chevron pointing back — shown on the "back to root" branch wedge and, in
// a submenu, over the hub as a reminder that clicking it also backs out.
function BackIcon() {
  return <path d="M14.5 5 8 12l6.5 7" fill="none" stroke="currentColor" strokeWidth="2.3" strokeLinecap="round" strokeLinejoin="round" />;
}
function CursorIcon() {
  return <path d="m4 3 8 17 2.5-6.5L21 11 4 3z" fill="none" stroke="currentColor" strokeWidth="2" strokeLinejoin="round" />;
}
function SpeakerIcon() {
  return <><path d="M11 5 6 9H3v6h3l5 4V5z" fill="none" stroke="currentColor" strokeWidth="2" strokeLinejoin="round" /><path d="M15.5 8.5a5 5 0 0 1 0 7M18.5 6a9 9 0 0 1 0 12" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" /></>;
}
function MicIcon() {
  return <><rect x="9" y="2" width="6" height="12" rx="3" fill="none" stroke="currentColor" strokeWidth="2" /><path d="M5 11a7 7 0 0 0 14 0M12 18v4" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" /></>;
}
function YoutubeIcon() {
  return <><rect x="2" y="5" width="20" height="14" rx="4" fill="none" stroke="currentColor" strokeWidth="2" /><path d="M10.5 9v6l5-3z" fill="currentColor" /></>;
}
function PlayIcon() {
  return <path d="M8 5v14l11-7z" fill="currentColor" />;
}
function PauseIcon() {
  return <><rect x="6" y="5" width="4" height="14" fill="currentColor" /><rect x="14" y="5" width="4" height="14" fill="currentColor" /></>;
}
function CloseIcon() {
  return <path d="M6 6l12 12M18 6 6 18" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" />;
}
// Icons for the settings branches — mode/quality/format wedges.
function TargetIcon() {
  return <><circle cx="12" cy="12" r="9" fill="none" stroke="currentColor" strokeWidth="2" /><circle cx="12" cy="12" r="5" fill="none" stroke="currentColor" strokeWidth="2" /><circle cx="12" cy="12" r="1.4" fill="currentColor" /></>;
}
function GaugeIcon() {
  return <><path d="M4 15a8 8 0 1 1 16 0" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" /><path d="M12 15 16 9" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" /><circle cx="12" cy="15" r="1.4" fill="currentColor" /></>;
}
function SignalIcon() {
  return <><rect x="4" y="14" width="3.5" height="6" fill="currentColor" /><rect x="10.25" y="10" width="3.5" height="10" fill="currentColor" /><rect x="16.5" y="5" width="3.5" height="15" fill="currentColor" /></>;
}
function ChipIcon() {
  return <><rect x="6" y="6" width="12" height="12" rx="2" fill="none" stroke="currentColor" strokeWidth="2" /><path d="M9 2v4M15 2v4M9 18v4M15 18v4M2 9h4M2 15h4M18 9h4M18 15h4" stroke="currentColor" strokeWidth="2" strokeLinecap="round" /></>;
}
function FileIcon() {
  return <><path d="M7 2h7l4 4v16H7z" fill="none" stroke="currentColor" strokeWidth="2" strokeLinejoin="round" /><path d="M14 2v4h4" fill="none" stroke="currentColor" strokeWidth="2" strokeLinejoin="round" /></>;
}
function FolderIcon() {
  return <path d="M3 6a1 1 0 0 1 1-1h5l2 2h9a1 1 0 0 1 1 1v10a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1z" fill="none" stroke="currentColor" strokeWidth="2" strokeLinejoin="round" />;
}
// Global-vs-per-game settings target wedge: globe when editing the shared
// config, a small controller when editing the current game's overrides.
function GlobeIcon() {
  return <><circle cx="12" cy="12" r="9" fill="none" stroke="currentColor" strokeWidth="2" /><path d="M3 12h18" stroke="currentColor" strokeWidth="2" /><path d="M12 3a14 14 0 0 1 0 18 14 14 0 0 1 0-18z" fill="none" stroke="currentColor" strokeWidth="2" /></>;
}
function ControllerIcon() {
  return <><rect x="2" y="7" width="20" height="11" rx="5.5" fill="none" stroke="currentColor" strokeWidth="2" /><path d="M7 10.5v4M5 12.5h4" stroke="currentColor" strokeWidth="2" strokeLinecap="round" /><circle cx="16" cy="11" r="1.2" fill="currentColor" /><circle cx="18.5" cy="13.5" r="1.2" fill="currentColor" /></>;
}

// Shared by the health strip and the bitrate number (see `StatusPanel`'s
// `bitrateColor`) so both agree: green = keeping up with target, yellow =
// mild shortfall, red = serious shortfall or stalled/zero.
function healthColor(ratio) {
  return ratio >= 0.85 ? "#22c55e" : ratio >= 0.5 ? "#eab308" : "#ef4444";
}

// Mini "stream health" sparkline — one bar per recent `live-stats` sample
// (~1/sec, oldest left). A bitrate-vs-target proxy, not real dropped-frame
// telemetry (ffmpeg's RTMP output doesn't expose that).
function StreamHealthStrip({ samples }) {
  if (!samples || samples.length === 0) return null;
  return (
    <div style={{ display: "flex", alignItems: "flex-end", height: 14, marginTop: 1 }}>
      {samples.map((ratio, i) => (
        <span key={i} style={{
          width: 1.5, marginRight: 0.5, borderRadius: 1, flexShrink: 0,
          height: Math.max(2, Math.min(14, ratio * 14)),
          background: healthColor(ratio),
        }} />
      ))}
    </div>
  );
}

// Corner card: live capture stats. Recording and the replay buffer can both
// run at once, so each gets its own card. `bitrateColor`, when given, wins
// over `bitrateLive` so the number agrees with the health strip.
function StatusPanel({ side, dotColor, icon, title, big, sub, stats, bitrate, bitrateLive, bitrateColor, health, inline }) {
  const row = { fontSize: 10.5, color: "#a8a29e", whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" };
  return (
    <div style={{
      ...(inline ? {} : { position: "fixed", top: 28, [side]: 28 }),
      display: "flex", flexDirection: "column", gap: 3, minWidth: 148,
      background: "rgba(20,18,17,0.92)", border: "1px solid rgba(255,255,255,0.1)",
      borderRadius: 8, padding: "10px 14px",
      fontFamily: "'Segoe UI', sans-serif", pointerEvents: "none",
    }}>
      <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
        {icon
          ? <img src={icon} alt="" style={{ width: 15, height: 15, borderRadius: 4, objectFit: "cover", flexShrink: 0 }} />
          : <span style={{ width: 7, height: 7, borderRadius: "50%", background: dotColor, flexShrink: 0 }} />}
        <span style={{ fontSize: 10.5, fontWeight: 700, color: "#d6d3d1", letterSpacing: 0.5, textTransform: "uppercase" }}>{title}</span>
      </div>
      <div style={{ display: "flex", alignItems: "baseline", gap: 8 }}>
        <span style={{ fontSize: 21, fontWeight: 700, color: "#fff", fontVariantNumeric: "tabular-nums" }}>{big}</span>
        {bitrate && (
          <span style={{ fontSize: 11.5, fontWeight: 700, color: bitrateColor ?? (bitrateLive ? "#86efac" : "#78716c"), fontVariantNumeric: "tabular-nums" }}>
            {bitrate}
          </span>
        )}
      </div>
      {sub && <div style={row}>{sub}</div>}
      {stats && <div style={row}>{stats}</div>}
      {health?.length > 0 && <StreamHealthStrip samples={health} />}
    </div>
  );
}

// Small circular quick-setting toggle under the wheel — doesn't close it,
// so a few can be flipped before dismissing. `style` lets callers layer in
// an entrance animation without this component knowing about it.
function QuickToggle({ icon: Glyph, label, on, onToggle, style }) {
  return (
    <button
      onClick={(e) => { e.stopPropagation(); onToggle(); }}
      style={{
        display: "flex", flexDirection: "column", alignItems: "center", gap: 6,
        background: "none", border: "none", cursor: "pointer", padding: 0,
        fontFamily: "'Segoe UI', sans-serif",
        ...style,
      }}
    >
      <span style={{
        position: "relative",
        display: "flex", alignItems: "center", justifyContent: "center",
        width: 54, height: 54, borderRadius: "50%",
        background: on ? "rgba(6,182,212,0.22)" : "rgba(12,12,12,0.82)",
        border: `2px solid ${on ? "rgba(34,211,238,0.85)" : "rgba(255,255,255,0.14)"}`,
        color: on ? "#67e8f9" : "#78716c",
        boxShadow: "0 4px 10px rgba(0,0,0,0.5)",
      }}>
        <svg width="24" height="24" viewBox="0 0 24 24"><Glyph /></svg>
        {!on && (
          <svg width="54" height="54" viewBox="0 0 54 54" style={{ position: "absolute", inset: 0 }}>
            <line x1="14" y1="40" x2="40" y2="14" stroke="#78716c" strokeWidth="2.5" strokeLinecap="round" />
          </svg>
        )}
      </span>
      <span style={{ fontSize: 12, fontWeight: 600, color: on ? "#e7e5e4" : "#78716c" }}>{label}</span>
    </button>
  );
}

// Same shape as `QuickToggle`, but for a value that cycles through options
// (recording resolution) rather than flipping on/off — shows the current
// value's short label inside the circle instead of an icon.
function ValueToggle({ display, label, active, onCycle, style }) {
  return (
    <button
      onClick={(e) => { e.stopPropagation(); onCycle(); }}
      style={{
        display: "flex", flexDirection: "column", alignItems: "center", gap: 6,
        background: "none", border: "none", cursor: "pointer", padding: 0,
        fontFamily: "'Segoe UI', sans-serif",
        ...style,
      }}
    >
      <span style={{
        display: "flex", alignItems: "center", justifyContent: "center",
        width: 54, height: 54, borderRadius: "50%",
        background: active ? "rgba(6,182,212,0.22)" : "rgba(12,12,12,0.82)",
        border: `2px solid ${active ? "rgba(34,211,238,0.85)" : "rgba(255,255,255,0.14)"}`,
        color: active ? "#67e8f9" : "#a8a29e",
        boxShadow: "0 4px 10px rgba(0,0,0,0.5)",
        fontSize: 11.5, fontWeight: 700, fontVariantNumeric: "tabular-nums",
      }}>
        {display}
      </span>
      <span style={{ fontSize: 12, fontWeight: 600, color: active ? "#e7e5e4" : "#78716c" }}>{label}</span>
    </button>
  );
}

// Thumbnail cache — warms once per wheel open, survives across opens as
// long as the wheel process/window is reused.
const clipThumbCache = new Map();

function ClipThumb({ video, onOpen, style }) {
  const cacheKey = `${video.name}:${video.modified ?? 0}`;
  const [thumb, setThumb] = useState(() => clipThumbCache.get(cacheKey) ?? null);
  const [hover, setHover] = useState(false);
  useEffect(() => {
    if (thumb) return;
    let cancelled = false;
    invoke("read_video_thumbnail", { name: video.name })
      .then((b64) => {
        const url = `data:image/jpeg;base64,${b64}`;
        clipThumbCache.set(cacheKey, url);
        if (!cancelled) setThumb(url);
      })
      .catch(() => {});
    return () => { cancelled = true; };
  }, [cacheKey]);
  return (
    <button
      onClick={(e) => { e.stopPropagation(); onOpen(video); }}
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
      title={video.title || video.name}
      style={{
        position: "relative", width: 76, height: 43, borderRadius: 6, overflow: "hidden",
        background: "#0c0c0c", border: `1px solid ${hover ? "rgba(255,255,255,0.35)" : "rgba(255,255,255,0.12)"}`,
        cursor: "pointer", padding: 0, flexShrink: 0,
        ...style,
      }}
    >
      {thumb && <img src={thumb} alt="" style={{ width: "100%", height: "100%", objectFit: "cover", display: "block" }} />}
      {hover && (
        <span style={{
          position: "absolute", inset: 0, display: "flex", alignItems: "center", justifyContent: "center",
          background: "rgba(0,0,0,0.4)", color: "#fff",
        }}>
          <svg width="18" height="18" viewBox="0 0 24 24"><PlayIcon /></svg>
        </span>
      )}
    </button>
  );
}

// Position/size per embedded window, keyed by `geomKey` — survives
// closing/reopening a window within the same wheel session (`DraggableWindow`
// unmounts on close), reset only when the whole wheel window is rebuilt.
const windowGeometry = {};

// Draggable/resizable panel chrome shared by the Gallery/Settings/Player
// views — renders inside the wheel's own window rather than a separate OS
// window, so the dim backdrop never disappears.
function DraggableWindow({ geomKey, title, onClose, width, height, offsetX = 0, offsetY = 0, minWidth = 340, minHeight = 220, noPadding, children }) {
  const saved = windowGeometry[geomKey];
  const [pos, setPos] = useState(() => saved
    ? { x: saved.x, y: saved.y }
    : {
        x: Math.max(12, (window.innerWidth - width) / 2 + offsetX),
        y: Math.max(12, (window.innerHeight - height) / 2 + offsetY),
      });
  const [size, setSize] = useState(() => saved ? { width: saved.width, height: saved.height } : { width, height });
  const dragRef = useRef(null);
  const resizeRef = useRef(null);

  const onHeaderPointerDown = (e) => {
    dragRef.current = { startX: e.clientX, startY: e.clientY, origX: pos.x, origY: pos.y };
    e.currentTarget.setPointerCapture(e.pointerId);
  };
  const onHeaderPointerMove = (e) => {
    if (!dragRef.current) return;
    const next = {
      x: dragRef.current.origX + (e.clientX - dragRef.current.startX),
      y: dragRef.current.origY + (e.clientY - dragRef.current.startY),
    };
    setPos(next);
    windowGeometry[geomKey] = { ...windowGeometry[geomKey], ...next, width: size.width, height: size.height };
  };
  const onHeaderPointerUp = () => { dragRef.current = null; };

  const onResizePointerDown = (e) => {
    e.stopPropagation();
    resizeRef.current = { startX: e.clientX, startY: e.clientY, origW: size.width, origH: size.height };
    e.currentTarget.setPointerCapture(e.pointerId);
  };
  const onResizePointerMove = (e) => {
    if (!resizeRef.current) return;
    e.stopPropagation();
    const next = {
      width: Math.max(minWidth, resizeRef.current.origW + (e.clientX - resizeRef.current.startX)),
      height: Math.max(minHeight, resizeRef.current.origH + (e.clientY - resizeRef.current.startY)),
    };
    setSize(next);
    windowGeometry[geomKey] = { ...windowGeometry[geomKey], ...next, x: pos.x, y: pos.y };
  };
  const onResizePointerUp = (e) => { e.stopPropagation(); resizeRef.current = null; };

  return (
    <div onClick={(e) => e.stopPropagation()} style={{
      position: "absolute", left: pos.x, top: pos.y, width: size.width, height: size.height, zIndex: 40,
      display: "flex", flexDirection: "column",
      background: "rgba(20,18,17,0.97)", border: "1px solid rgba(255,255,255,0.14)",
      borderRadius: 12, overflow: "hidden",
      fontFamily: "'Segoe UI', sans-serif",
      boxShadow: "0 20px 60px rgba(0,0,0,0.6)",
    }}>
      <div
        onPointerDown={onHeaderPointerDown}
        onPointerMove={onHeaderPointerMove}
        onPointerUp={onHeaderPointerUp}
        style={{
          display: "flex", alignItems: "center", justifyContent: "space-between",
          padding: "12px 16px", borderBottom: "1px solid rgba(255,255,255,0.08)", flexShrink: 0,
          cursor: "grab", userSelect: "none", touchAction: "none",
        }}
      >
        <span style={{ fontSize: 13.5, fontWeight: 700, color: "#e7e5e4", letterSpacing: 0.3 }}>{title}</span>
        <button
          onClick={onClose}
          onPointerDown={(e) => e.stopPropagation()}
          style={{
            display: "flex", alignItems: "center", justifyContent: "center",
            width: 24, height: 24, borderRadius: 6,
            background: "rgba(255,255,255,0.06)", border: "1px solid rgba(255,255,255,0.12)",
            color: "#d6d3d1", cursor: "pointer",
          }}
        >
          <svg width="12" height="12" viewBox="0 0 24 24"><CloseIcon /></svg>
        </button>
      </div>
      <div className={noPadding ? undefined : "wheel-scroll"} style={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column", overflow: noPadding ? "hidden" : "auto", padding: noPadding ? 0 : 18 }}>
        {children}
      </div>

      {/* Resize grip — bottom-right corner, drag to resize. */}
      <div
        onPointerDown={onResizePointerDown}
        onPointerMove={onResizePointerMove}
        onPointerUp={onResizePointerUp}
        style={{
          position: "absolute", right: 0, bottom: 0, width: 18, height: 18,
          cursor: "nwse-resize", touchAction: "none",
          display: "flex", alignItems: "flex-end", justifyContent: "flex-end", padding: 3,
        }}
      >
        <svg width="10" height="10" viewBox="0 0 10 10" style={{ pointerEvents: "none" }}>
          <line x1="9" y1="2" x2="2" y2="9" stroke="rgba(255,255,255,0.35)" strokeWidth="1.4" strokeLinecap="round" />
          <line x1="9" y1="6" x2="6" y2="9" stroke="rgba(255,255,255,0.35)" strokeWidth="1.4" strokeLinecap="round" />
        </svg>
      </div>
    </div>
  );
}

function GalleryTile({ video, onOpen }) {
  const isExternal = Boolean(video.local_path?.startsWith("http"));
  const cacheKey = `${video.name}:${video.modified ?? 0}`;
  const [thumb, setThumb] = useState(() => clipThumbCache.get(cacheKey) ?? null);
  const [hover, setHover] = useState(false);
  useEffect(() => {
    if (thumb || isExternal) return;
    let cancelled = false;
    invoke("read_video_thumbnail", { name: video.name })
      .then((b64) => {
        const url = `data:image/jpeg;base64,${b64}`;
        clipThumbCache.set(cacheKey, url);
        if (!cancelled) setThumb(url);
      })
      .catch(() => {});
    return () => { cancelled = true; };
  }, [cacheKey, isExternal]);
  return (
    <button
      onClick={() => onOpen(video)}
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
      title={video.title || video.name}
      style={{ width: 172, cursor: "pointer", padding: 0, background: "none", border: "none", textAlign: "left" }}
    >
      <div style={{
        position: "relative", width: "100%", aspectRatio: "16/9", borderRadius: 8, overflow: "hidden",
        background: "#0c0c0c", border: `1px solid ${hover ? "rgba(255,255,255,0.35)" : "rgba(255,255,255,0.1)"}`,
      }}>
        {thumb && <img src={thumb} alt="" style={{ width: "100%", height: "100%", objectFit: "cover", display: "block" }} />}
        {hover && (
          <span style={{ position: "absolute", inset: 0, display: "flex", alignItems: "center", justifyContent: "center", background: "rgba(0,0,0,0.4)", color: "#fff" }}>
            <svg width="24" height="24" viewBox="0 0 24 24"><PlayIcon /></svg>
          </span>
        )}
        {video.kind === "youtube_live" && (
          <span style={{ position: "absolute", top: 6, left: 6, background: "#dc2626", color: "#fff", fontSize: 9.5, fontWeight: 700, padding: "2px 6px", borderRadius: 4 }}>
            LIVE
          </span>
        )}
      </div>
      <div style={{ fontSize: 11.5, fontWeight: 600, color: "#d6d3d1", marginTop: 6, whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>
        {video.title || video.name}
      </div>
      <div style={{ fontSize: 10.5, color: "#78716c" }}>
        {video.app || (video.drive_only ? "Drive" : isExternal ? "YouTube" : "")}
      </div>
    </button>
  );
}

function GalleryPanel({ t, onOpenClip }) {
  const [videos, setVideos] = useState(null); // null = loading
  useEffect(() => { invoke("list_videos").then(setVideos).catch(() => setVideos([])); }, []);
  if (videos === null) {
    return <div style={{ color: "#78716c", fontSize: 13, textAlign: "center", padding: 40 }}>…</div>;
  }
  if (videos.length === 0) {
    return <div style={{ color: "#78716c", fontSize: 13, textAlign: "center", padding: 40 }}>{t("wheel.no_clips")}</div>;
  }
  return (
    <div style={{ display: "flex", flexWrap: "wrap", gap: 14 }}>
      {videos.map((v) => <GalleryTile key={v.name} video={v} onOpen={onOpenClip} />)}
    </div>
  );
}

// Draggable-window content: a plain HTML5 `<video>` fed the local file via
// Tauri's asset protocol (`convertFileSrc`) so it plays inside this same
// overlay window — no OS media player, no second window to alt-tab into.
function WheelVideoPlayer({ video }) {
  const videoRef = useRef(null);
  const barRef = useRef(null);
  const [playing, setPlaying] = useState(false);
  const [current, setCurrent] = useState(0);
  const [duration, setDuration] = useState(0);
  const [volume, setVolume] = useState(1);
  const [muted, setMuted] = useState(false);
  const src = useMemo(() => convertFileSrc(video.local_path), [video.local_path]);

  const togglePlay = () => { const v = videoRef.current; if (!v) return; if (v.paused) v.play(); else v.pause(); };
  const seekBy = (d) => { const v = videoRef.current; if (!v) return; v.currentTime = Math.min(Math.max(0, v.currentTime + d), v.duration || 0); };
  const seekToClientX = (clientX) => {
    const v = videoRef.current, bar = barRef.current;
    if (!v || !bar || !duration) return;
    const rect = bar.getBoundingClientRect();
    const pct = Math.min(1, Math.max(0, (clientX - rect.left) / rect.width));
    v.currentTime = pct * duration;
    setCurrent(pct * duration);
  };
  const btnStyle = {
    display: "flex", alignItems: "center", justifyContent: "center",
    width: 30, height: 30, borderRadius: 6, background: "rgba(255,255,255,0.06)",
    border: "1px solid rgba(255,255,255,0.1)", color: "#d6d3d1", cursor: "pointer", flexShrink: 0,
  };

  return (
    <div style={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column", background: "#000" }}>
      <div style={{ position: "relative", flex: 1, minHeight: 0 }} onClick={togglePlay}>
        <video
          ref={videoRef}
          src={src}
          autoPlay
          style={{ width: "100%", height: "100%", objectFit: "contain", display: "block", background: "#000" }}
          onPlay={() => setPlaying(true)}
          onPause={() => setPlaying(false)}
          onTimeUpdate={(e) => setCurrent(e.currentTarget.currentTime)}
          onLoadedMetadata={(e) => setDuration(e.currentTarget.duration)}
        />
        {!playing && (
          <span style={{
            position: "absolute", inset: 0, display: "flex", alignItems: "center", justifyContent: "center",
            color: "#fff", pointerEvents: "none",
          }}>
            <span style={{ width: 56, height: 56, borderRadius: "50%", background: "rgba(0,0,0,0.5)", display: "flex", alignItems: "center", justifyContent: "center" }}>
              <svg width="24" height="24" viewBox="0 0 24 24"><PlayIcon /></svg>
            </span>
          </span>
        )}
      </div>
      <div style={{ padding: "10px 14px", background: "rgba(12,12,12,0.96)", flexShrink: 0 }} onClick={(e) => e.stopPropagation()}>
        <div
          ref={barRef}
          onPointerDown={(e) => { seekToClientX(e.clientX); e.currentTarget.setPointerCapture(e.pointerId); }}
          onPointerMove={(e) => { if (e.buttons === 1) seekToClientX(e.clientX); }}
          style={{ position: "relative", height: 5, borderRadius: 3, background: "rgba(255,255,255,0.2)", cursor: "pointer", marginBottom: 10 }}
        >
          <div style={{ position: "absolute", top: 0, bottom: 0, left: 0, width: `${duration ? (current / duration) * 100 : 0}%`, background: "#22d3ee", borderRadius: 3 }} />
        </div>
        <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
          <button onClick={togglePlay} style={btnStyle}>
            <svg width="14" height="14" viewBox="0 0 24 24">{playing ? <PauseIcon /> : <PlayIcon />}</svg>
          </button>
          <button onClick={() => seekBy(-10)} style={{ ...btnStyle, width: "auto", padding: "0 8px", fontSize: 10.5, fontWeight: 700 }}>-10s</button>
          <button onClick={() => seekBy(10)} style={{ ...btnStyle, width: "auto", padding: "0 8px", fontSize: 10.5, fontWeight: 700 }}>+10s</button>
          <span style={{ fontSize: 11, fontVariantNumeric: "tabular-nums", color: "#a8a29e" }}>
            {fmtClock(Math.floor(current))} / {fmtClock(Math.floor(duration))}
          </span>
          <div style={{ flex: 1 }} />
          <svg width="14" height="14" viewBox="0 0 24 24" color="#a8a29e" onClick={() => { const v = videoRef.current; if (!v) return; v.muted = !v.muted; setMuted(v.muted); }} style={{ cursor: "pointer" }}>
            <SpeakerIcon />
          </svg>
          <input
            type="range" min={0} max={1} step={0.01} value={muted ? 0 : volume}
            onChange={(e) => {
              const next = Number(e.target.value);
              const v = videoRef.current;
              if (v) { v.volume = next; v.muted = next === 0; }
              setVolume(next);
              setMuted(next === 0);
            }}
            style={{ width: 70 }}
          />
        </div>
      </div>
    </div>
  );
}

export default function App() {
  const [lang, setLang] = useState("en");
  const [hover, setHover] = useState(null);
  const [recording, setRecording] = useState(null); // RecordingSession | null
  const [buffer, setBuffer] = useState(null); // { running, buffered_seconds, app } | null
  const [settings, setSettings] = useState(null);
  const now = useClockSeconds();
  // null = root ring, or a branch id below — Settings lives as branches of
  // the wheel itself (see `settingsBranchActions`/`BRANCH_PARENT`) rather
  // than a separate window.
  const [branch, setBranch] = useState(null);
  // Embedded, draggable overlay windows (Gallery/Player), rendered inside this same
  // fullscreen window so the dim backdrop persists; can be open independently of each other.
  const [showGallery, setShowGallery] = useState(false);
  const [player, setPlayer] = useState(null); // VideoItem | null
  // Measured (not configured-target) bitrate, from ffmpeg's own progress
  // feed — null until the encoder's first ~1s report arrives.
  const [liveBitrateKbps, setLiveBitrateKbps] = useState(null);
  // Same, but for the live stream's own ffmpeg process (separate from the
  // local file's — see `WriterControl`), plus a rolling measured/target
  // ratio window for the stream-health strip below.
  const [streamBitrateKbps, setStreamBitrateKbps] = useState(null);
  const [streamHealth, setStreamHealth] = useState([]);
  // Most-recent local clips for the quick-access strip — fetched once on
  // open, not polled (the wheel is short-lived).
  const [recentClips, setRecentClips] = useState([]);
  // Display name of whatever game the detection loop currently sees in the
  // foreground — `null` when nothing's detected. Powers the top-left context
  // card below, alongside the live clock.
  const [currentApp, setCurrentApp] = useState(null);
  const appIcon = useAppIcon(currentApp);
  // Whether the Settings branch's value pickers write to the global config
  // or to the currently-detected game's overrides.
  const [settingsTarget, setSettingsTarget] = useState("global"); // "global" | "game"
  // The current game's overrides, refetched whenever the detected game
  // changes. Optimistically patched locally on every write too.
  const [gameOverrides, setGameOverrides] = useState({});
  useEffect(() => {
    if (!currentApp) { setGameOverrides({}); return; }
    invoke("get_game_overrides", { name: currentApp }).then(setGameOverrides).catch(() => {});
  }, [currentApp]);
  // Bumped on every "wheel-shown" and used as the backdrop's `key` so the
  // open animation remounts and replays on every summon (the window itself
  // is reused, not rebuilt — see `wheel.rs::open`).
  const [openToken, setOpenToken] = useState(0);
  // Gates every entrance `animation` below between frozen-at-frame-0 and
  // actually playing — flipped true only once the re-shown window is
  // confirmed visible and painted (see `closeSelf`/`wheel-shown` below).
  const [playing, setPlaying] = useState(false);
  const t = useMemo(() => createT(lang), [lang]);

  // Set once by the dev-only `set-wheel-demo` command below — once frozen,
  // `loadAll`'s poll must stop overwriting `currentApp`/`buffer` with the
  // real (empty, on a screenshot rig) values a couple seconds later.
  const demoFrozenRef = useRef(false);

  // Shared by the initial mount and the "wheel-shown" listener — the window
  // is kept alive and hidden/re-shown rather than rebuilt, so this needs to
  // be callable again on re-show.
  const loadAll = () => {
    invoke("get_settings").then((s) => { setSettings(s); setLang(s.language ?? "en"); }).catch(() => {});
    invoke("get_recording_status").then(setRecording).catch(() => {});
    if (!demoFrozenRef.current) {
      invoke("get_replay_buffer_status").then(setBuffer).catch(() => {});
      invoke("get_current_game").then(setCurrentApp).catch(() => {});
    }
    invoke("list_videos")
      .then((vids) => setRecentClips(vids.filter((v) => v.local_path && !v.local_path.startsWith("http")).slice(0, 5)))
      .catch(() => {});
  };

  useEffect(() => {
    loadAll();
    const poll = setInterval(loadAll, 2000);
    return () => clearInterval(poll);
  }, []);

  // Dev-only hook for store_screenshots.rs; stripped from prod builds. Fakes
  // a running replay buffer, a detected game, and a hovered wedge for the
  // "replay" screenshot scene — never a real capture session.
  useEffect(() => {
    if (!import.meta.env.DEV) return;
    let unlisten;
    (async () => {
      unlisten = await listen("store-screenshot-cmd", ({ payload }) => {
        if (payload?.action === "set-wheel-demo") {
          demoFrozenRef.current = true;
          setCurrentApp(payload.currentApp ?? null);
          setBuffer(payload.buffer ?? null);
          setHover(payload.hover ?? null);
          requestAnimationFrame(() => setTimeout(() => emit("store-screenshot-ready", {}), 50));
        }
      });
    })();
    return () => unlisten?.();
  }, []);

  useEffect(() => {
    let unlisten;
    listen("recording-stats", (e) => setLiveBitrateKbps(e.payload?.bitrate_kbps ?? null)).then((u) => { unlisten = u; });
    return () => unlisten?.();
  }, []);
  // Stale once nothing is recording — don't show last session's number.
  useEffect(() => { if (!recording) setLiveBitrateKbps(null); }, [recording]);

  // Mirrored into a ref instead of a direct dependency below, since `recording` gets a
  // new reference on every 2s poll and re-subscribing `listen` that often risks dropped events.
  const liveTargetRef = useRef(null);
  useEffect(() => { liveTargetRef.current = recording?.live_bitrate_kbps ?? null; }, [recording]);
  // Up to ~1.5 minutes of health samples (ffmpeg reports ~1/sec) — each a
  // measured/target ratio, capped a bit above 1 so a brief overshoot doesn't
  // read identically to a comfortable margin.
  const STREAM_HEALTH_WINDOW = 90;
  useEffect(() => {
    let unlisten;
    listen("live-stats", (e) => {
      const kbps = e.payload?.bitrate_kbps ?? null;
      setStreamBitrateKbps(kbps);
      const target = liveTargetRef.current;
      if (kbps == null || !target) return;
      const ratio = Math.min(1.3, kbps / target);
      setStreamHealth((prev) => [...prev, ratio].slice(-STREAM_HEALTH_WINDOW));
    }).then((u) => { unlisten = u; });
    return () => unlisten?.();
  }, []);
  // Stale once streaming stops (or the whole session does) — don't show the
  // last stream's numbers/history for a plain local-only recording.
  useEffect(() => {
    if (!recording?.live) { setStreamBitrateKbps(null); setStreamHealth([]); }
  }, [recording?.live]);

  // Backend re-shows the same mounted window instead of rebuilding it (reset to root/frame-0
  // already happened in `closeSelf`), so just pull fresh data and double-rAF the entrance.
  useEffect(() => {
    let unlisten;
    listen("wheel-shown", () => {
      closingRef.current = false;
      loadAll();
      let raf1 = requestAnimationFrame(() => {
        raf1 = requestAnimationFrame(() => setPlaying(true));
      });
    }).then((u) => { unlisten = u; });
    return () => unlisten?.();
  }, []);

  // Backend-initiated closes can't just hide the window themselves — they'd
  // race ahead of the frontend's own reset (see `closeSelf`). So they only
  // ask; this window still owns the actual `hide()` call.
  useEffect(() => {
    let unlisten;
    listen("wheel-close-requested", closeSelf).then((u) => { unlisten = u; });
    return () => unlisten?.();
  }, []);

  // Covers the very first-ever open, which has no `wheel-shown` event: paint
  // the paused frame-0 pose once, then start the animations.
  useEffect(() => {
    let raf1 = requestAnimationFrame(() => {
      raf1 = requestAnimationFrame(() => setPlaying(true));
    });
    return () => cancelAnimationFrame(raf1);
  }, []);

  // Quick-setting toggles: patch + save, optimistically updating local state
  // so the button flips immediately.
  const patchSettings = async (make) => {
    const fresh = await invoke("get_settings").catch(() => null);
    if (!fresh) return;
    const next = make(fresh);
    setSettings(next);
    await invoke("save_settings", { settings: next }).catch(() => {});
  };
  const video = settings?.video ?? {};
  const audio = video.audio ?? {};
  const toggleCursor = () => patchSettings((s) => ({ ...s, video: { ...s.video, capture_cursor: !(s.video?.capture_cursor ?? true) } }));
  const toggleSystemAudio = () => patchSettings((s) => ({ ...s, video: { ...s.video, audio: { ...s.video?.audio, system_muted: !(s.video?.audio?.system_muted ?? false) } } }));
  const toggleMic = () => patchSettings((s) => ({ ...s, video: { ...s.video, audio: { ...s.video?.audio, mic_muted: !(s.video?.audio?.mic_muted ?? false) } } }));
  const RESOLUTION_CYCLE = ["native", "p1080", "p720", "p480"];
  const cycleResolution = () => patchSettings((s) => {
    const cur = s.video?.resolution ?? "native";
    const next = RESOLUTION_CYCLE[(RESOLUTION_CYCLE.indexOf(cur) + 1) % RESOLUTION_CYCLE.length];
    return { ...s, video: { ...s.video, resolution: next } };
  });
  // Mode for the *next* manual recording from this wheel: cycles
  // record -> both -> stream via the "Stream" quick-toggle below. Local
  // transient state, separate from the persisted Full-Session auto-stream setting.
  const STREAM_MODES = ["record", "both", "stream"];
  const [streamMode, setStreamMode] = useState("record");
  const cycleStreamMode = () => setStreamMode((m) => STREAM_MODES[(STREAM_MODES.indexOf(m) + 1) % STREAM_MODES.length]);
  // Short in-circle glyphs for the quick-toggle below — same abbreviation
  // style as the resolution toggle's "1080p"/"Auto" (not translated; the
  // caption underneath, which is, carries the actual meaning).
  const STREAM_MODE_DISPLAY = { record: "REC", both: "REC+", stream: "LIVE" };
  // Manual per-recording folder override (see `config::RecordingFolder`) —
  // `null` means "use the game's own default, or the recordings root".
  // Transient like `streamMode`: reset on every close, not persisted.
  const [folderOverride, setFolderOverride] = useState(null);
  // Only folders that could apply to the recording this wheel is about to
  // start: global ones plus whichever are scoped to the currently-detected
  // game — never another game's own folder.
  const recordingFolders = (settings?.recording_folders ?? []).filter((f) => !f.game || f.game === currentApp);
  // Settings branches pick a value directly from a sub-ring of wedges. Option lists mirror
  // `RecordSettingsCard.jsx` but are duplicated as plain arrays since this window skips Tailwind.
  const FPS_OPTIONS = [24, 30, 60, 120, 144];
  const BITRATE_OPTIONS = [4000, 6000, 8000, 12000, 15000, 20000, 30000, 50000];
  const CONTAINER_OPTIONS = ["mp4", "mkv", "mov", "mp4_fragmented", "mov_fragmented"];
  // Keep in sync with `RecordSettingsCard.jsx`'s CONTAINER_LABELS (not
  // imported — see this block's own doc comment above for why).
  const CONTAINER_LABELS = {
    mp4: "MP4", mkv: "MKV", mov: "MOV",
    mp4_fragmented: "MP4 (Fragmented)", mov_fragmented: "MOV (Fragmented)",
  };
  const MODE_OPTIONS = ["clips", "full_session", "off"];
  const BUFFER_MIN_OPTIONS = [1, 2, 3, 5, 10, 15];
  // Grouped by vendor so each ring stays short — a flat 14-way list in one
  // ring would be cramped; drilling vendor → codec is two short picks instead.
  const ENCODER_GROUPS = {
    nvenc: [["nvenc_h264", "H.264"], ["nvenc_hevc", "HEVC"], ["nvenc_av1", "AV1"]],
    amf: [["amf_h264", "H.264"], ["amf_hevc", "HEVC"], ["amf_av1", "AV1"]],
    qsv: [["qsv_h264", "H.264"], ["qsv_hevc", "HEVC"], ["qsv_av1", "AV1"]],
    sw: [["x264_software", "x264"], ["x265_software", "x265"], ["svt_av1", "SVT-AV1"], ["aom_av1", "AOM AV1"]],
  };
  const selectFps = (v) => patchSettings((s) => ({ ...s, video: { ...s.video, fps: v } }));
  const selectBitrate = (v) => patchSettings((s) => ({ ...s, video: { ...s.video, bitrate_kbps: v } }));
  const selectEncoder = (v) => patchSettings((s) => ({ ...s, video: { ...s.video, encoder: v } }));
  const selectContainer = (v) => patchSettings((s) => ({ ...s, video: { ...s.video, container: v } }));
  const selectMode = (v) => patchSettings((s) => ({ ...s, video: { ...s.video, replay_buffer: { ...s.video?.replay_buffer, game_detect_mode: v } } }));
  const selectBufferMinutes = (v) => patchSettings((s) => ({ ...s, video: { ...s.video, replay_buffer: { ...s.video?.replay_buffer, buffer_minutes: v } } }));
  // Local clips play inline (embedded `<video>`, see `WheelVideoPlayer`);
  // YouTube-hosted entries have no local file to feed it, so those still
  // open externally.
  const openClip = (v) => {
    if (v.local_path?.startsWith("http")) invoke("open_url", { url: v.local_path }).catch(() => {});
    else setPlayer(v);
  };

  // Resets to the root ring and freezes entrance animations at frame 0.
  // Must run *before* `hide()`: a hidden window stops compositing, so
  // whatever frame was last painted is what redisplays on the next `show()`.
  const resetWheelAnim = () => {
    setBranch(null);
    setShowGallery(false);
    setPlayer(null);
    setSettingsTarget("global");
    setFolderOverride(null);
    setPlaying(false);
    setOpenToken((n) => n + 1);
  };

  // Guards against `closeSelf` re-entering itself: `hide()` below causes the
  // OS to fire our own `blur` listener a moment later, which would otherwise
  // call `closeSelf` a second time for the same close (double-scheduling the
  // idle-destroy timer and spamming the log with superseded epochs). Cleared
  // whenever the window becomes visible again.
  const closingRef = useRef(false);
  // Hides rather than closes — the backend keeps this window alive and reuses
  // it on the next hotkey press (see `wheel.rs::open`). Resets first, then
  // waits two animation frames to confirm the reset painted before `hide()`.
  const closeSelf = () => {
    if (closingRef.current) return;
    closingRef.current = true;
    resetWheelAnim();
    let raf1 = requestAnimationFrame(() => {
      raf1 = requestAnimationFrame(() => {
        window.__TAURI__?.window?.getCurrentWindow?.()?.hide();
        // Esc/blur/backdrop-click close entirely from here, with no Rust
        // side involved at all (unlike the Alt+F2 toggle and wedge-action
        // paths) — without this, the backend never learns the window
        // actually went hidden, so its idle-destroy timer never starts.
        invoke("wheel_closed").catch(() => {});
      });
    });
  };
  // Step back one level at a time: close any open windows first, then out of
  // a submenu (see `BRANCH_PARENT`), then close the whole overlay. Reached
  // only via the hub click, Esc, and right-click.
  const backOrClose = () => {
    if (player || showGallery) {
      setPlayer(null);
      setShowGallery(false);
      return;
    }
    if (branch) { setBranch(BRANCH_PARENT[branch] ?? null); return; }
    closeSelf();
  };

  useEffect(() => {
    // Re-bound whenever this state changes so Esc always sees the current
    // level (a `[]`-only effect would close over stale state).
    const onKey = (e) => { if (e.key === "Escape") backOrClose(); };
    const onBlur = () => closeSelf();
    window.addEventListener("keydown", onKey);
    window.addEventListener("blur", onBlur);
    return () => { window.removeEventListener("keydown", onKey); window.removeEventListener("blur", onBlur); };
  }, [branch, player, showGallery]);

  const isRecording = Boolean(recording);
  const bufferRunning = Boolean(buffer?.running);

  // Corner panels: recording and the replay buffer are independent sessions, each with its
  // own card. Stats read off the session/buffer objects, never global `settings.video`.
  const targetLabel = (target) => {
    if (target?.window?.app) return target.window.app;
    if (target?.area) return t("wheel.stats_area");
    return t("wheel.stats_monitor");
  };
  const RES_LABEL = { native: null, p1080: "1080p", p720: "720p", p480: "480p" };
  const ENCODER_LABEL = {
    auto: "Auto",
    nvenc_h264: "NVENC H.264", nvenc_hevc: "NVENC HEVC", nvenc_av1: "NVENC AV1",
    amf_h264: "AMF H.264", amf_hevc: "AMF HEVC", amf_av1: "AMF AV1",
    qsv_h264: "QSV H.264", qsv_hevc: "QSV HEVC", qsv_av1: "QSV AV1",
    x264_software: "x264", x265_software: "x265",
    svt_av1: "SVT-AV1", aom_av1: "AOM AV1",
  };
  const mbps = (kbps) => (kbps ? `${(kbps / 1000).toFixed(kbps < 10000 ? 1 : 0)} Mbps` : null);

  // Short display label for the quick-settings row's resolution cycle
  // button, and the abbreviation used for a picked mode value's wedge.
  const MODE_SHORT = { clips: "CLIPS", full_session: "FULL", off: "OFF" };
  const resLabel = RES_LABEL[video.resolution ?? "native"] ?? "Auto";

  // Root ring: the frequent, single-click actions. The record wedge behaves
  // differently depending on state — mid-recording it stops directly;
  // otherwise it opens the "how do I start" submenu.
  const rootActions = [
    { id: "save_replay", icon: SaveIcon, label: t("wheel.save_replay"), disabled: !bufferRunning },
    isRecording
      ? { id: "toggle_recording", icon: StopIcon, label: t("wheel.stop_recording"), danger: true }
      : { id: "branch:record", icon: RecordIcon, label: t("wheel.toggle_recording") },
    // Independent on/off for each of a running session's two outputs — only
    // meaningful once something is actually recording; turning off whichever
    // output is the only one left running is really just Stop.
    ...(isRecording ? [
      recording.local
        ? { id: "toggle_local_recording", icon: StopIcon, label: t("wheel.stop_local_recording"), danger: true }
        : { id: "toggle_local_recording", icon: RecordIcon, label: t("wheel.start_local_recording") },
      recording.live
        ? { id: "toggle_live_streaming", icon: StopIcon, label: t("wheel.stop_live_streaming"), danger: true }
        : { id: "toggle_live_streaming", icon: YoutubeIcon, label: t("wheel.start_live_streaming") },
    ] : []),
    bufferRunning
      ? { id: "toggle_buffer", icon: BufferIcon, label: t("wheel.buffer_stop"), danger: true }
      : { id: "toggle_buffer", icon: BufferIcon, label: t("wheel.buffer_start") },
    { id: "open_gallery", icon: GalleryIcon, label: t("wheel.open_gallery") },
    { id: "branch:settings", icon: SettingsIcon, label: t("wheel.open_settings") },
  ];

  // "record" submenu: every way to start a capture. Streaming follows the `streamMode`
  // cycle below the wheel. Window/area capture go through the picker overlay (no live variant).
  const RECORD_ACTION_BY_MODE = { record: "toggle_recording", both: "record_monitor_live", stream: "record_monitor_stream_only" };
  const SESSION_ACTION_BY_MODE = { record: "record_session", both: "record_session_live", stream: "record_session_stream_only" };
  const RECORD_LABEL_BY_MODE = { record: "record_screen", both: "record_screen_live", stream: "record_screen_stream_only" };
  const SESSION_LABEL_BY_MODE = { record: "record_session", both: "record_session_live", stream: "record_session_stream_only" };
  const recordBranchActions = [
    {
      id: RECORD_ACTION_BY_MODE[streamMode],
      icon: streamMode === "record" ? RecordIcon : streamMode === "both" ? RecordLiveIcon : YoutubeIcon,
      label: t(`wheel.${RECORD_LABEL_BY_MODE[streamMode]}`),
    },
    {
      id: SESSION_ACTION_BY_MODE[streamMode],
      icon: streamMode === "record" ? TargetIcon : streamMode === "both" ? RecordLiveIcon : YoutubeIcon,
      label: t(`wheel.${SESSION_LABEL_BY_MODE[streamMode]}`),
      // Records the currently-detected game's window directly (no picker) —
      // same target the context card names and the buffer wedge already
      // uses. Disabled with nothing detected: there's no window to target.
      disabled: !currentApp,
    },
    { id: "record_window", icon: WindowIcon, label: t("wheel.record_window") },
    { id: "record_area", icon: AreaIcon, label: t("wheel.record_area") },
    // Only the session-record actions read `folderOverride` — window/area
    // capture uses the interactive picker overlay, which has no folder
    // selection yet. Shown only when at least one folder is configured.
    ...(recordingFolders.length > 0 ? [{
      id: "branch:folder_pick", icon: FolderIcon,
      label: t("wheel.folder")(folderOverride ? recordingFolders.find((f) => f.id === folderOverride)?.name ?? t("wheel.folderDefault") : t("wheel.folderDefault")),
      active: folderOverride != null,
    }] : []),
  ];

  // Settings as wheel branches, covering only what the quick-settings row doesn't. Can target
  // the detected game instead of global config; buffer duration is hidden when doing so
  // since `GameOverrides` has no field for it.
  const editingGame = settingsTarget === "game" && Boolean(currentApp);
  const settingsBranchActions = [
    ...(currentApp ? [{
      id: "cycle_target", icon: editingGame ? ControllerIcon : GlobeIcon,
      label: editingGame ? currentApp : t("wheel.target_global"), active: editingGame,
    }] : []),
    { id: "branch:mode_pick", icon: TargetIcon, label: t("wheel.mode") },
    ...(editingGame ? [] : [{ id: "branch:buffer_pick", icon: BufferIcon, label: t("wheel.buffer_minutes") }]),
    { id: "branch:fps_pick", icon: GaugeIcon, label: "FPS" },
    { id: "branch:bitrate_pick", icon: SignalIcon, label: t("wheel.bitrate") },
    { id: "branch:encoder_pick", icon: ChipIcon, label: t("wheel.encoder") },
    { id: "branch:container_pick", icon: FileIcon, label: t("wheel.container") },
  ];

  // Effective value = game override if targeting a game and set, else global. Used to tint
  // the active wedge and to compare against each ring's "Default" wedge.
  const globalMode = video.replay_buffer?.game_detect_mode ?? "off";
  const globalFps = video.fps ?? 30;
  const globalBitrateKbps = video.bitrate_kbps ?? 12000;
  const globalContainer = video.container ?? "mp4";
  const globalEncoder = video.encoder ?? "auto";
  const effMode = editingGame ? gameOverrides.game_detect_mode ?? globalMode : globalMode;
  const effFps = editingGame ? gameOverrides.fps ?? globalFps : globalFps;
  const effBitrateKbps = editingGame ? gameOverrides.bitrate_kbps ?? globalBitrateKbps : globalBitrateKbps;
  const effContainer = editingGame ? gameOverrides.container ?? globalContainer : globalContainer;
  const effEncoder = editingGame ? gameOverrides.encoder ?? globalEncoder : globalEncoder;

  // A "Default" wedge up front when targeting a game, so a per-game override
  // can be cleared (back to whatever the global setting is) as easily as it
  // was set — it's tinted active exactly when there's no override to clear.
  const defaultWedge = (key, icon, isSet) =>
    editingGame ? [{ id: `clear:${key}`, icon, label: t("wheel.default"), active: !isSet }] : [];

  const curBufferMinutes = video.replay_buffer?.buffer_minutes ?? 3;
  const modePickActions = [
    ...defaultWedge("mode", TargetIcon, gameOverrides.game_detect_mode != null),
    ...MODE_OPTIONS.map((m) => ({ id: `select:mode:${m}`, icon: TargetIcon, label: MODE_SHORT[m] ?? m, active: m === effMode })),
  ];
  const bufferPickActions = BUFFER_MIN_OPTIONS.map((m) => ({
    id: `select:buffer:${m}`, icon: BufferIcon, label: `${m}m`, active: m === curBufferMinutes,
  }));
  const fpsPickActions = [
    ...defaultWedge("fps", GaugeIcon, gameOverrides.fps != null),
    ...FPS_OPTIONS.map((f) => ({ id: `select:fps:${f}`, icon: GaugeIcon, label: `${f}`, active: f === effFps })),
  ];
  const bitratePickActions = [
    ...defaultWedge("bitrate", SignalIcon, gameOverrides.bitrate_kbps != null),
    ...BITRATE_OPTIONS.map((b) => ({ id: `select:bitrate:${b}`, icon: SignalIcon, label: `${Math.round(b / 1000)}M`, active: b === effBitrateKbps })),
  ];
  const containerPickActions = [
    ...defaultWedge("container", FileIcon, gameOverrides.container != null),
    ...CONTAINER_OPTIONS.map((c) => ({ id: `select:container:${c}`, icon: FileIcon, label: CONTAINER_LABELS[c], active: c === effContainer })),
  ];
  // Vendor first, codec second — a flat 14-way ring would be cramped; this
  // is two short picks instead. Auto has no codec variant, so it selects
  // directly rather than branching.
  const encoderPickActions = [
    ...defaultWedge("encoder", ChipIcon, gameOverrides.encoder != null),
    { id: "select:encoder:auto", icon: ChipIcon, label: "Auto", active: effEncoder === "auto" },
    { id: "branch:encoder_nvenc", icon: ChipIcon, label: "NVIDIA" },
    { id: "branch:encoder_amf", icon: ChipIcon, label: "AMD" },
    { id: "branch:encoder_qsv", icon: ChipIcon, label: "Intel QSV" },
    { id: "branch:encoder_sw", icon: ChipIcon, label: t("wheel.software") },
  ];
  const encoderVendorActions = (vendor) => ENCODER_GROUPS[vendor].map(([value, label]) => ({
    id: `select:encoder:${value}`, icon: ChipIcon, label, active: value === effEncoder,
  }));

  // Manual per-recording folder pick, applying to local `folderOverride` state
  // (via the `select_folder:` prefix in `runAction`) instead of a persisted setting.
  const folderPickActions = [
    { id: "select_folder:", icon: FolderIcon, label: t("wheel.folderDefault"), active: folderOverride == null },
    ...recordingFolders.map((f) => ({
      id: `select_folder:${f.id}`, icon: FolderIcon, label: f.name, active: folderOverride === f.id,
    })),
  ];

  // Parent of each branch, for multi-level "back" (Esc, right-click, hub
  // click) — `null`/missing means back goes straight to the root ring.
  const BRANCH_PARENT = {
    mode_pick: "settings", buffer_pick: "settings", fps_pick: "settings", bitrate_pick: "settings",
    encoder_pick: "settings", container_pick: "settings",
    encoder_nvenc: "encoder_pick", encoder_amf: "encoder_pick", encoder_qsv: "encoder_pick", encoder_sw: "encoder_pick",
    folder_pick: "record",
  };
  const branches = {
    record: recordBranchActions,
    settings: settingsBranchActions,
    mode_pick: modePickActions,
    buffer_pick: bufferPickActions,
    fps_pick: fpsPickActions,
    bitrate_pick: bitratePickActions,
    container_pick: containerPickActions,
    encoder_pick: encoderPickActions,
    encoder_nvenc: encoderVendorActions("nvenc"),
    encoder_amf: encoderVendorActions("amf"),
    encoder_qsv: encoderVendorActions("qsv"),
    encoder_sw: encoderVendorActions("sw"),
    folder_pick: folderPickActions,
  };
  const actions = branch ? branches[branch] : rootActions;

  // Global setters (unchanged) vs. the game-override field each key maps to.
  const GLOBAL_SETTERS = {
    mode: selectMode,
    buffer: (v) => selectBufferMinutes(Number(v)),
    fps: (v) => selectFps(Number(v)),
    bitrate: (v) => selectBitrate(Number(v)),
    encoder: selectEncoder,
    container: selectContainer,
  };
  const GAME_FIELD = { mode: "game_detect_mode", fps: "fps", bitrate: "bitrate_kbps", encoder: "encoder", container: "container" };
  const NUMERIC_FIELDS = new Set(["fps", "bitrate_kbps"]);
  // Applies a picked (or cleared) setting to whichever scope is selected. Per-game writes
  // patch+persist immediately since there's no single settings blob to round-trip.
  const applySetting = (key, rawValue) => {
    if (editingGame) {
      const field = GAME_FIELD[key];
      const value = rawValue == null ? null : NUMERIC_FIELDS.has(field) ? Number(rawValue) : rawValue;
      const next = { ...gameOverrides, [field]: value };
      setGameOverrides(next);
      invoke("set_game_overrides", { name: currentApp, overrides: next }).catch(() => {});
    } else {
      GLOBAL_SETTERS[key]?.(rawValue);
    }
  };

  const runAction = (id) => {
    if (id === "cycle_target") { setSettingsTarget((cur) => (cur === "game" ? "global" : "game")); return; }
    if (id.startsWith("branch:")) { setBranch(id.slice(7)); return; }
    // A picked value applies immediately and drops back to the settings
    // list (not just one level, e.g. out of an encoder vendor submenu)
    // so the next thing the user sees is always "what else can I adjust".
    if (id.startsWith("select:")) {
      const rest = id.slice(7);
      const sep = rest.indexOf(":");
      applySetting(rest.slice(0, sep), rest.slice(sep + 1));
      setBranch("settings");
      return;
    }
    if (id.startsWith("clear:")) { applySetting(id.slice(6), null); setBranch("settings"); return; }
    // Manual folder pick — local state only (see `folderOverride`'s
    // declaration), not a persisted setting; drops back to the record
    // branch so the user picks an actual start action next.
    if (id.startsWith("select_folder:")) {
      const folderId = id.slice("select_folder:".length);
      setFolderOverride(folderId || null);
      setBranch("record");
      return;
    }
    // Gallery opens as an embedded draggable window instead of a separate
    // OS window — see `DraggableWindow`.
    if (id === "open_gallery") { setShowGallery(true); return; }
    // Backend only asks this window to close (`wheel-close-requested`); the
    // actual `hide()` waits here for the reset animation (see `closeSelf`).
    // `folder` only matters for actions that start a recording directly here.
    invoke("wheel_action", { action: id, folder: folderOverride }).catch(() => closeSelf());
  };

  // Hub is purely navigational; live stats have their own corner panels.
  const hubTop = branch ? t(`wheel.branch_${branch}`) : "Capcove";
  const hubTopColor = "#e7e5e4";
  // While targeting a game anywhere in the settings tree, the hub's bottom line shows that
  // game's name instead of "Back" — a reminder of scope so picks don't land in the wrong place.
  const inSettingsTree = branch === "settings" || branch in BRANCH_PARENT;
  const hubBottom = editingGame && inSettingsTree ? currentApp : branch ? t("wheel.back") : t("wheel.dismiss");

  // Always-shown top-left context card: detected foreground app plus a live clock,
  // for deciding what to record regardless of whether anything's recording yet.
  const clockLocale = lang === "tr" ? "tr-TR" : "en-US";
  const nowDate = new Date(now);
  const contextPanel = {
    dotColor: "#78716c",
    icon: appIcon,
    title: currentApp ?? t("wheel.desktop"),
    big: nowDate.toLocaleTimeString(clockLocale, { hour: "2-digit", minute: "2-digit", second: "2-digit", hour12: false }),
    sub: nowDate.toLocaleDateString(clockLocale, { day: "numeric", month: "long", year: "numeric" }),
  };

  // Local recording and live streaming are independent processes (see
  // `WriterControl`), each with its own encoder/bitrate settings — so each
  // gets its own panel, shown only while that output is active.
  let localPanel = null;
  if (isRecording && recording.local) {
    const elapsed = Math.max(0, Math.floor(now / 1000) - recording.started_at);
    const stats = [ENCODER_LABEL[recording.encoder] ?? recording.encoder, `${recording.fps} FPS`, RES_LABEL[recording.resolution]]
      .filter(Boolean).join(" · ");
    const localMbps = mbps(liveBitrateKbps);
    const targetMbps = mbps(recording.bitrate_kbps);
    localPanel = {
      side: "left",
      dotColor: "#ef4444",
      title: t("wheel.status_recording"),
      big: fmtClock(elapsed),
      bitrate: localMbps ?? targetMbps,
      bitrateLive: !!localMbps,
      sub: targetLabel(recording.target),
      stats,
    };
  }
  let livePanel = null;
  if (isRecording && recording.live) {
    const elapsed = Math.max(0, Math.floor(now / 1000) - recording.started_at);
    const stats = [
      ENCODER_LABEL[recording.encoder] ?? recording.encoder,
      recording.live_fps ? `${recording.live_fps} FPS` : null,
      RES_LABEL[recording.live_resolution],
    ].filter(Boolean).join(" · ");
    const streamMbps = mbps(streamBitrateKbps);
    const targetMbps = mbps(recording.live_bitrate_kbps);
    // Same ratio the health strip's bars use for their most recent sample —
    // see `StatusPanel`'s `bitrateColor` doc comment for why the number
    // needs to agree with them instead of just meaning "do we have data".
    const latestRatio = streamHealth.length > 0 ? streamHealth[streamHealth.length - 1] : null;
    livePanel = {
      side: "left",
      dotColor: "#dc2626",
      title: t("wheel.live_tag"),
      big: fmtClock(elapsed),
      bitrate: streamMbps ?? targetMbps,
      bitrateLive: !!streamMbps,
      bitrateColor: latestRatio != null ? healthColor(latestRatio) : undefined,
      health: streamHealth,
      sub: targetLabel(recording.target),
      stats,
    };
  }
  let bufferPanel = null;
  if (bufferRunning) {
    const stats = [ENCODER_LABEL[buffer.encoder] ?? buffer.encoder, buffer.fps ? `${buffer.fps} FPS` : null, RES_LABEL[buffer.resolution]]
      .filter(Boolean).join(" · ");
    // Derived from `started_at` and ticked locally by `now` (capped at the
    // buffer length) so it counts up each second, instead of stepping only
    // when the 2s status poll lands.
    const bufferSecs = buffer.started_at != null
      ? Math.min(buffer.max_seconds ?? Infinity, Math.max(0, Math.floor(now / 1000) - buffer.started_at))
      : (buffer.buffered_seconds ?? 0);
    bufferPanel = {
      side: "right",
      dotColor: "#f59e0b",
      title: t("wheel.status_clipping"),
      big: fmtClock(bufferSecs),
      bitrate: mbps(buffer.bitrate_kbps),
      bitrateLive: false,
      sub: buffer.app ?? t("wheel.stats_monitor"),
      stats,
    };
  }

  const step = 360 / actions.length;

  return (
    // Fullscreen dimmer behind the wheel — stays mounted even with a Gallery/
    // Settings/Player window open on top, so the game underneath never loses
    // focus. Click or right-click anywhere steps back one level.
    <div
      key={openToken}
      onClick={backOrClose}
      onContextMenu={(e) => { e.preventDefault(); backOrClose(); }}
      style={{
        width: "100vw", height: "100vh", position: "relative",
        display: "flex", flexDirection: "column", alignItems: "center", justifyContent: "center",
        gap: 26,
        background: "rgba(0,0,0,0.72)",
        animation: "wheelBackdropIn 110ms ease-out",
        animationPlayState: playing ? "running" : "paused",
      }}
    >
      {/* Custom scrollbar for embedded-window content, matching the flat dark
          theme. Also the wheel's open animation: window creation is instant,
          so this is just the wedge ring/backdrop popping in over ~120-150ms,
          replayed each summon via `key={openToken}` and held at frame 0 by
          `animationPlayState` until `playing` confirms the window is visible. */}
      <style>{`
        .wheel-scroll::-webkit-scrollbar { width: 9px; height: 9px; }
        .wheel-scroll::-webkit-scrollbar-track { background: transparent; }
        .wheel-scroll::-webkit-scrollbar-thumb {
          background-color: rgba(255,255,255,0.16);
          border-radius: 5px;
          border: 2px solid rgba(20,18,17,0.97);
          background-clip: padding-box;
        }
        .wheel-scroll::-webkit-scrollbar-thumb:hover { background-color: rgba(255,255,255,0.28); }
        .wheel-scroll::-webkit-scrollbar-corner { background: transparent; }
        @keyframes wheelBackdropIn { from { opacity: 0; } to { opacity: 1; } }
        @keyframes wheelFadeIn { from { opacity: 0; } to { opacity: 1; } }
        @keyframes wheelPopIn {
          from { opacity: 0; transform: scale(0.35); }
          to { opacity: 1; transform: scale(1); }
        }
        @keyframes wheelFadeUp {
          from { opacity: 0; transform: translateY(10px); }
          to { opacity: 1; transform: translateY(0); }
        }
      `}</style>

      {/* Top-left stack: context card always, recording/streaming cards below
          it when active — one wrapper instead of separately `fixed` cards. */}
      <div style={{
        position: "fixed", top: 28, left: 28, display: "flex", flexDirection: "column", gap: 10,
        animation: "wheelFadeIn 200ms ease-out both",
        animationPlayState: playing ? "running" : "paused",
      }}>
        <StatusPanel inline {...contextPanel} />
        {localPanel && <StatusPanel inline {...localPanel} />}
        {livePanel && <StatusPanel inline {...livePanel} />}
      </div>
      {bufferPanel && (
        <div style={{
          position: "fixed", top: 28, right: 28,
          animation: "wheelFadeIn 200ms ease-out both",
          animationPlayState: playing ? "running" : "paused",
        }}>
          <StatusPanel {...bufferPanel} />
        </div>
      )}

      {showGallery && (
        <DraggableWindow geomKey="gallery" title={t("wheel.open_gallery")} onClose={() => setShowGallery(false)} width={860} height={540} offsetX={-140} offsetY={-30}>
          <GalleryPanel t={t} onOpenClip={openClip} />
        </DraggableWindow>
      )}
      {player && (
        <DraggableWindow geomKey="player" title={player.title || player.name} onClose={() => setPlayer(null)} width={760} height={480} offsetX={0} offsetY={60} minWidth={420} minHeight={280} noPadding>
          <WheelVideoPlayer video={player} />
        </DraggableWindow>
      )}

      <svg width={SIZE} height={SIZE} viewBox={`0 0 ${SIZE} ${SIZE}`}
        style={{ userSelect: "none" }} onClick={(e) => e.stopPropagation()}>
        {/* Flat design: solid colors only — no gradients, glows, or drop
            shadows. Separation between wedges comes from thin divider
            spokes, not gaps or shadow depth. */}
        <circle cx={CX} cy={CY} r={R_OUTER + 3} fill="none" stroke="rgba(255,255,255,0.14)" strokeWidth="1" pointerEvents="none" />

        {/* Each wedge pops in from its own center, swept around the ring in
            sequence (~18ms/wedge) with a slight overshoot — noticeably more
            "alive" than a single uniform fade, still flat/no-glow. */}
        {actions.map(({ id, disabled, isBack, active }, i) => {
          const start = i * step;
          const end = start + step;
          const isHover = hover === id && !disabled;
          const hoverFill = isBack ? "rgba(30,58,95,0.45)" : "rgba(22,78,99,0.45)";
          const [wx, wy] = polar(start + step / 2, R_MID);
          return (
            <path key={id} d={wedgePath(start, end)}
              fill={isHover ? hoverFill : active ? "rgba(6,182,212,0.16)" : "rgba(23,20,18,0.9)"}
              style={{
                cursor: disabled ? "default" : "pointer", transition: "fill 100ms ease",
                transformOrigin: `${wx}px ${wy}px`,
                animation: "wheelPopIn 220ms cubic-bezier(0.34, 1.56, 0.64, 1) both",
                animationDelay: `${i * 18}ms`,
                animationPlayState: playing ? "running" : "paused",
              }}
              onMouseEnter={() => setHover(id)}
              onMouseLeave={() => setHover(null)}
              onClick={() => { if (!disabled) runAction(id); }}
            />
          );
        })}

        {/* Divider spokes on top of the wedge fills — the sole separator. */}
        {actions.map((_, i) => {
          const [ix, iy, ox, oy] = divider(i * step);
          return (
            <line key={`div-${i}`} x1={ix} y1={iy} x2={ox} y2={oy} stroke="rgba(255,255,255,0.1)" strokeWidth="1" pointerEvents="none"
              style={{ animation: "wheelFadeIn 220ms ease-out both", animationDelay: `${i * 18}ms`, animationPlayState: playing ? "running" : "paused" }} />
          );
        })}

        {actions.map(({ id, icon: Glyph, label, disabled, danger, isBack, active }, i) => {
          const mid = i * step + step / 2;
          const [ax, ay] = polar(mid, R_MID);
          const iconX = ax;
          const iconY = ay - 22;
          const isHover = hover === id && !disabled;
          const glyphColor = disabled ? "#57534e" : isHover ? (isBack ? "#93c5fd" : "#67e8f9") : danger ? "#f87171" : active ? "#67e8f9" : isBack ? "#a8a29e" : "#d6d3d1";
          const textColor = disabled ? "#57534e" : isHover ? (isBack ? "#bfdbfe" : "#a5f3fc") : danger ? "#f87171" : active ? "#67e8f9" : isBack ? "#78716c" : "#a8a29e";
          return (
            <g key={`label-${id}`} pointerEvents="none"
              style={{ animation: "wheelFadeIn 220ms ease-out both", animationDelay: `${i * 18 + 40}ms`, animationPlayState: playing ? "running" : "paused" }}>
              <g transform={`translate(${iconX - 12}, ${iconY - 12})`} color={glyphColor}>
                <svg width="24" height="24" viewBox="0 0 24 24" style={{ overflow: "visible" }}>
                  <Glyph />
                </svg>
              </g>
              <text x={ax} y={ay + 16} textAnchor="middle"
                fill={textColor} letterSpacing="0.3"
                fontFamily="'Segoe UI', sans-serif" fontSize="12.5" fontWeight="600">
                {label}
              </text>
            </g>
          );
        })}

        {/* Center hub: brand mark + live status; click backs out of a
            submenu, or dismisses at the root. Pops in first, punchier than
            the wedges since it's the anchor the whole ring builds around. */}
        <g style={{ transformOrigin: `${CX}px ${CY}px`, animation: "wheelPopIn 240ms cubic-bezier(0.34, 1.56, 0.64, 1) both", animationPlayState: playing ? "running" : "paused" }}>
          <circle cx={CX} cy={CY} r={R_INNER - 8}
            fill="#141210" stroke="rgba(255,255,255,0.14)" strokeWidth="1"
            style={{ cursor: "pointer" }} onClick={backOrClose} />
          {branch ? (
            <g pointerEvents="none" color="#a8a29e">
              <g transform={`translate(${CX - 9}, ${CY - 34})`}>
                <svg width="18" height="18" viewBox="0 0 24 24"><BackIcon /></svg>
              </g>
            </g>
          ) : (
            <image href={logo} x={CX - 15} y={CY - 38} width="30" height="30" pointerEvents="none" />
          )}
          <text x={CX} y={CY + 3} textAnchor="middle" fill={hubTopColor} letterSpacing="0.2"
            fontFamily="'Segoe UI', sans-serif" fontSize="13.5" fontWeight="700" pointerEvents="none">
            {hubTop}
          </text>
          <text x={CX} y={CY + 20} textAnchor="middle" fill="#78716c"
            fontFamily="'Segoe UI', sans-serif" fontSize="10.5" pointerEvents="none">
            {hubBottom}
          </text>
        </g>
      </svg>

      {/* Quick settings under the wheel — flip without closing. Fades/slides
          up in a stagger, cascading in right after the wheel's own wedge
          sweep finishes above. */}
      {settings && (
        <div style={{ display: "flex", gap: 26 }} onClick={(e) => e.stopPropagation()}>
          <QuickToggle icon={CursorIcon} label={t("wheel.cursor")}
            on={video.capture_cursor ?? true} onToggle={toggleCursor}
            style={{ animation: "wheelFadeUp 220ms ease-out both", animationDelay: "260ms", animationPlayState: playing ? "running" : "paused" }} />
          <QuickToggle icon={SpeakerIcon} label={t("wheel.sound")}
            on={!(audio.system_muted ?? false)} onToggle={toggleSystemAudio}
            style={{ animation: "wheelFadeUp 220ms ease-out both", animationDelay: "285ms", animationPlayState: playing ? "running" : "paused" }} />
          <QuickToggle icon={MicIcon} label={t("wheel.mic")}
            on={!(audio.mic_muted ?? false)} onToggle={toggleMic}
            style={{ animation: "wheelFadeUp 220ms ease-out both", animationDelay: "310ms", animationPlayState: playing ? "running" : "paused" }} />
          <ValueToggle display={resLabel} label={t("wheel.resolution")}
            active={(video.resolution ?? "native") !== "native"} onCycle={cycleResolution}
            style={{ animation: "wheelFadeUp 220ms ease-out both", animationDelay: "335ms", animationPlayState: playing ? "running" : "paused" }} />
          <ValueToggle display={STREAM_MODE_DISPLAY[streamMode]} label={t(`wheel.stream_mode_${streamMode}`)}
            active={streamMode !== "record"} onCycle={cycleStreamMode}
            style={{ animation: "wheelFadeUp 220ms ease-out both", animationDelay: "360ms", animationPlayState: playing ? "running" : "paused" }} />
        </div>
      )}

      {/* Recent-clips quick-access strip — glance at (or open) the last few
          local recordings without leaving the game for the full gallery.
          Same cascade, starting once the quick-settings row above is done. */}
      {recentClips.length > 0 && (
        <div style={{ display: "flex", alignItems: "center", gap: 8 }} onClick={(e) => e.stopPropagation()}>
          {recentClips.map((v, i) => (
            <ClipThumb key={v.name} video={v} onOpen={openClip}
              style={{ animation: "wheelFadeUp 200ms ease-out both", animationDelay: `${420 + i * 25}ms`, animationPlayState: playing ? "running" : "paused" }} />
          ))}
          <button
            onClick={() => runAction("open_gallery")}
            title={t("wheel.open_full_gallery")}
            style={{
              display: "flex", alignItems: "center", justifyContent: "center",
              width: 43, height: 43, borderRadius: 6, flexShrink: 0,
              background: "rgba(12,12,12,0.82)", border: "1px solid rgba(255,255,255,0.14)",
              color: "#a8a29e", cursor: "pointer", padding: 0,
              animation: "wheelFadeUp 200ms ease-out both",
              animationDelay: `${420 + recentClips.length * 25}ms`,
              animationPlayState: playing ? "running" : "paused",
            }}
          >
            <svg width="18" height="18" viewBox="0 0 24 24"><GalleryIcon /></svg>
          </button>
        </div>
      )}
    </div>
  );
}
