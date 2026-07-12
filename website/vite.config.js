import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import { siteUrlPlugin } from "./scripts/site-url-plugin.mjs";

// Both deploy targets sit at the same /capcove/ subpath, so `base` never
// changes — only the canonical-domain placeholder and output dir do.
// Set via scripts/build-site.mjs, not meant to be passed manually.
const SITE_URL = (process.env.SITE_URL || "https://xacnio.github.io/capcove").replace(/\/$/, "");
const OUT_DIR = process.env.OUT_DIR || "dist";

export default defineConfig({
  base: "/capcove/",
  plugins: [react(), tailwindcss(), siteUrlPlugin(SITE_URL)],
  build: {
    outDir: OUT_DIR,
    rollupOptions: {
      input: {
        main: "index.html",
        tr: "tr/index.html",
        privacy: "privacy.html",
        terms: "terms.html",
        license: "license.html",
      },
    },
  },
});
