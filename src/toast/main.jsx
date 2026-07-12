import { createRoot } from "react-dom/client";
import App from "./App.jsx";

// No styles.css import: its `color-scheme: dark` makes Chromium paint an
// opaque canvas behind the "transparent" window (same reason wheel/rec-hud
// avoid it) — this page styles itself inline instead of Tailwind.
document.documentElement.style.background = "transparent";
document.body.style.background = "transparent";

// No <StrictMode>: its dev-mode double-invoke would mount the event listener
// effect twice, and the phantom first listener could outlive cleanup and
// stick around alongside the real one, doubling every toast.
createRoot(document.getElementById("root")).render(<App />);
