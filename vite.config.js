import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

// Tauri multi-window: each window loads its own HTML entry point. Settings and the
// video editor are in-app views inside the gallery window, so they have no entry here.
export default defineConfig({
  plugins: [react(), tailwindcss()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
  },
  build: {
    target: "esnext",
    rollupOptions: {
      input: {
        gallery: "pages/gallery.html",
        overlay: "pages/overlay.html",
        recHud: "pages/rec-hud.html",
        wheel: "pages/wheel.html",
        toast: "pages/toast.html",
        recorder: "pages/recorder.html",
        recorderFrame: "pages/recorder-frame.html",
        recorderPicker: "pages/recorder-picker.html",
      },
    },
  },
});
