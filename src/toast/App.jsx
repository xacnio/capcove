import { useEffect, useRef, useState } from "react";
import { invoke, listen } from "../lib/tauri.js";

// In-app toast overlay — a fullscreen, transparent, click-through window (see
// `toast.rs`) rendering `toasts` in the user's chosen corner; queueing/timing/animation live here.

const VISIBLE_MS = 3200;
// The card's own exit slide (travels fully clear of its own width, off the
// screen edge) needs more time to read as a real departure than the older
// 24px nudge did; the wrapper's collapse (closing the gap the departed card
// leaves in the stack) runs a little longer still so it finishes just after
// the slide, instead of yanking the stack shut mid-motion.
const EXIT_MS = 320;
const COLLAPSE_MS = 380;
// The collapse (which finishes after the slide) starts this long into the
// leave, so the gap doesn't start closing before the card's mostly cleared it.
const COLLAPSE_DELAY_MS = 80;
const EXIT_TOTAL_MS = COLLAPSE_DELAY_MS + COLLAPSE_MS;
const MAX_STACK = 4;
// Comfortably clears the card's own width (380px) plus its shadow, so it's
// fully off-screen — not just faded — by the time it's removed.
const EXIT_DISTANCE = 460;

// Flush against the corner — no gap to the screen edge.
const CORNER_STYLE = {
  top_left: { top: 0, left: 0, alignItems: "flex-start" },
  top_right: { top: 0, right: 0, alignItems: "flex-end" },
  bottom_left: { bottom: 0, left: 0, alignItems: "flex-start" },
  bottom_right: { bottom: 0, right: 0, alignItems: "flex-end" },
};

// Slide direction matches the corner's horizontal side — toasts enter from
// the edge they're anchored to, and leave back off the same edge (further
// than they entered from, so the exit reads as a full departure rather than
// the entrance played in reverse). Leaving stays fully opaque — fading it out
// at the same time as the slide made it visually vanish in place instead of
// visibly travelling away; opacity now only ever changes on entrance.
function slideTransform(corner, phase) {
  const fromRight = corner === "top_right" || corner === "bottom_right";
  if (phase === "entered") return { opacity: 1, transform: "translateX(0)" };
  if (phase === "leaving") {
    return { opacity: 1, transform: `translateX(${fromRight ? EXIT_DISTANCE : -EXIT_DISTANCE}px)` };
  }
  return { opacity: 0, transform: fromRight ? "translateX(24px)" : "translateX(-24px)" };
}

function ToastCard({ toast, corner, leaving }) {
  const accent = toast.kind === "error" ? "#ef4444" : "#22d3ee";
  // Starts off-screen on mount; a CSS transition never animates the first
  // paint, so the double rAF guarantees a paint before flipping `entered`.
  const [entered, setEntered] = useState(false);
  useEffect(() => {
    let raf1 = requestAnimationFrame(() => {
      raf1 = requestAnimationFrame(() => setEntered(true));
    });
    return () => cancelAnimationFrame(raf1);
  }, []);
  const phase = leaving ? "leaving" : entered ? "entered" : "entering";

  return (
    // Outer wrapper collapses its own height (and the margin that makes the
    // stack gap) once the card starts leaving, so the toasts above/below
    // slide smoothly closed behind it instead of jumping the instant it's
    // removed from the DOM.
    <div
      style={{
        // Hidden on the Y axis only, to actually hide the collapsing gap —
        // hiding X too clipped the card the instant it slid past its own
        // width, cutting the exit off almost as soon as it started instead
        // of letting it travel all the way off-screen.
        overflowX: "visible",
        overflowY: "hidden",
        maxHeight: leaving ? 0 : 200,
        marginBottom: leaving ? 0 : 10,
        transition: `max-height ${COLLAPSE_MS}ms ease ${leaving ? COLLAPSE_DELAY_MS : 0}ms, margin-bottom ${COLLAPSE_MS}ms ease ${leaving ? COLLAPSE_DELAY_MS : 0}ms`,
      }}
    >
      <div
        style={{
          display: "flex", alignItems: "stretch", gap: 14,
          width: 380, maxWidth: "88vw",
          background: "rgba(15,14,13,0.97)", border: "1px solid rgba(255,255,255,0.06)",
          borderRadius: 0, overflow: "hidden",
          boxShadow: "0 16px 40px rgba(0,0,0,0.55)",
          backdropFilter: "blur(6px)",
          fontFamily: "'Segoe UI', sans-serif",
          transition: `opacity ${EXIT_MS}ms ease, transform ${EXIT_MS}ms ease`,
          ...slideTransform(corner, phase),
        }}
      >
        {toast.icon && (
          <div style={{
            display: "flex", alignItems: "center", padding: "14px 0 14px 16px", flexShrink: 0,
          }}>
            <img src={toast.icon} alt="" style={{ width: 44, height: 44, objectFit: "cover", display: "block" }} />
          </div>
        )}
        <div style={{ padding: `14px 18px 14px ${toast.icon ? 12 : 16}px`, minWidth: 0 }}>
          <div style={{ display: "flex", alignItems: "center", gap: 7, marginBottom: 4 }}>
            <span style={{ width: 6, height: 6, flexShrink: 0, background: accent }} />
            <span style={{ fontSize: 14, fontWeight: 700, color: "#f0efee" }}>{toast.title}</span>
          </div>
          {toast.body && (
            <div style={{
              fontSize: 12.5, color: "#a8a29e", lineHeight: 1.4,
              overflow: "hidden", textOverflow: "ellipsis", display: "-webkit-box",
              WebkitLineClamp: 3, WebkitBoxOrient: "vertical",
            }}>
              {toast.body}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

export default function App() {
  const [toasts, setToasts] = useState([]); // { id, kind, title, body, leaving }
  const [corner, setCorner] = useState("top_right");
  const timers = useRef(new Map());

  useEffect(() => {
    // `listen()` resolves asynchronously, so a plain cleanup could run before
    // it resolves and leak the listener; `cancelled` lets it unlisten itself.
    let cancelled = false;
    let unlisten;
    listen("show-toast", (e) => {
      const t = e.payload;
      // Fresh corner preference on every toast — infrequent enough that a
      // dedicated "settings changed" broadcast isn't worth adding just for this.
      invoke("get_settings").then((s) => setCorner(s.video?.toast_corner ?? "top_right")).catch(() => {});

      setToasts((cur) => [...cur, { ...t, leaving: false }].slice(-MAX_STACK));

      const exitAt = setTimeout(() => {
        setToasts((cur) => cur.map((x) => (x.id === t.id ? { ...x, leaving: true } : x)));
        const removeAt = setTimeout(() => {
          setToasts((cur) => cur.filter((x) => x.id !== t.id));
          timers.current.delete(t.id);
        }, EXIT_TOTAL_MS);
        timers.current.set(t.id, removeAt);
      }, VISIBLE_MS);
      timers.current.set(t.id, exitAt);
    }).then((u) => {
      if (cancelled) { u(); return; }
      unlisten = u;
      // Tells the backend our listener is actually live — any toast fired
      // before this point (including the very first one, while this page
      // was still loading) was queued there and gets flushed now.
      invoke("toast_ready").catch(() => {});
    });
    return () => {
      cancelled = true;
      unlisten?.();
      for (const id of timers.current.values()) clearTimeout(id);
    };
  }, []);

  if (toasts.length === 0) return null;

  const style = CORNER_STYLE[corner] ?? CORNER_STYLE.top_right;
  const isBottom = corner.startsWith("bottom");

  return (
    <div
      style={{
        position: "fixed", ...style,
        display: "flex", flexDirection: isBottom ? "column-reverse" : "column",
        // Spacing between cards lives on each `ToastCard`'s own wrapper
        // (`marginBottom`) instead of a container `gap`, so a leaving card
        // can collapse its own contribution to the gap as it exits.
        pointerEvents: "none",
      }}
    >
      {toasts.map((t) => <ToastCard key={t.id} toast={t} corner={corner} leaving={t.leaving} />)}
    </div>
  );
}
