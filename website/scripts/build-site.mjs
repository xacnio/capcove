// Builds the site for one deploy target. Both targets share the same
// /capcove/ base path (see vite.config.js) — only the canonical domain
// baked into HTML/sitemap/robots and the output dir differ.
import { build } from "vite";
import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";

const ROOT = path.dirname(path.dirname(fileURLToPath(import.meta.url)));

const TARGETS = {
  github: { siteUrl: "https://xacnio.github.io/capcove", outDir: "dist" },
  "xacnio-dev": { siteUrl: "https://xacnio.dev/capcove", outDir: "dist-xacnio-dev" },
};

const targetName = process.argv[2] || "github";
const target = TARGETS[targetName];
if (!target) {
  console.error(`[build-site] unknown target "${targetName}". Available: ${Object.keys(TARGETS).join(", ")}`);
  process.exit(1);
}

process.env.SITE_URL = target.siteUrl;
process.env.OUT_DIR = target.outDir;

await build({ root: ROOT });

// public/sitemap.xml and public/robots.txt are copied as-is (not run through
// Vite's HTML pipeline), so patch the placeholder in the built output directly.
const outDir = path.join(ROOT, target.outDir);
for (const file of ["sitemap.xml", "robots.txt"]) {
  const filePath = path.join(outDir, file);
  const content = readFileSync(filePath, "utf-8").replaceAll("__SITE_URL__", target.siteUrl);
  writeFileSync(filePath, content);
}

console.log(`[build-site] built target "${targetName}" -> ${target.outDir} (${target.siteUrl})`);
