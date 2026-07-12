import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App.jsx";

// No styles.css import: its `color-scheme: dark` makes Chromium paint an
// opaque canvas behind the "transparent" window (same reason rec-hud avoids
// it) — the wheel styles itself inline/via SVG instead of Tailwind.
document.documentElement.style.background = "transparent";
document.body.style.background = "transparent";

createRoot(document.getElementById("root")).render(
  <StrictMode>
    <App />
  </StrictMode>
);
