import { useEffect, useState } from "react";
import { invoke, listen } from "../lib/tauri.js";

const BORDER_PX = 6;

const RESIZE_HANDLES = [
  { dir: "NorthWest", cursor: "nwse-resize", style: { top: 0, left: 0 } },
  { dir: "North", cursor: "ns-resize", style: { top: 0, left: "50%", transform: "translateX(-50%)" } },
  { dir: "NorthEast", cursor: "nesw-resize", style: { top: 0, right: 0 } },
  { dir: "East", cursor: "ew-resize", style: { top: "50%", right: 0, transform: "translateY(-50%)" } },
  { dir: "SouthEast", cursor: "nwse-resize", style: { bottom: 0, right: 0 } },
  { dir: "South", cursor: "ns-resize", style: { bottom: 0, left: "50%", transform: "translateX(-50%)" } },
  { dir: "SouthWest", cursor: "nesw-resize", style: { bottom: 0, left: 0 } },
  { dir: "West", cursor: "ew-resize", style: { top: "50%", left: 0, transform: "translateY(-50%)" } },
];

export default function App() {
  const [recording, setRecording] = useState(false);

  // The backend controls this window's click-through state (a cursor poll in
  // Area mode, always-on in Window mode) and already shows it explicitly when
  // it's created — no `window_ready`/focus needed here. Focusing it would
  // fight with the control bar for the topmost z-order band.
  useEffect(() => {
    const unsubs = [];
    listen("recording-started", () => setRecording(true)).then((u) => unsubs.push(u));
    listen("recording-stopped", () => setRecording(false)).then((u) => unsubs.push(u));
    invoke("get_recording_status").then((s) => setRecording(!!s)).catch(() => {});
    return () => unsubs.forEach((fn) => fn?.());
  }, []);

  // The whole gesture runs in the backend off the OS cursor position — the
  // frontend just says "a drag started" (which handle), and the fast path
  // "released in-window". `handle` is "move" or a compass direction.
  const startGesture = (handle) => (e) => {
    e.preventDefault();
    e.stopPropagation();
    invoke("recorder_area_drag_begin", { handle }).catch(() => {});
    const onUp = () => {
      window.removeEventListener("mouseup", onUp);
      invoke("recorder_area_drag_end").catch(() => {});
    };
    window.addEventListener("mouseup", onUp);
  };

  const onBodyDown = recording ? undefined : startGesture("move");
  const onHandleDown = (dir) => startGesture(dir);

  // No styles.css here (same reason as the wheel window — it forces an opaque
  // canvas behind "transparent" windows), so layout is inline only.
  return (
    <div style={{ position: "fixed", inset: 0 }}>
      <div
        onMouseDown={onBodyDown}
        style={{
          position: "absolute",
          inset: 0,
          border: `${BORDER_PX}px dashed ${recording ? "#ef4444" : "#22d3ee"}`,
          boxSizing: "border-box",
          cursor: recording ? "default" : "move",
        }}
      />
      {!recording && RESIZE_HANDLES.map(({ dir, cursor, style }) => (
        <div
          key={dir}
          onMouseDown={onHandleDown(dir)}
          style={{ position: "absolute", height: 18, width: 18, ...style, cursor }}
        />
      ))}
    </div>
  );
}
