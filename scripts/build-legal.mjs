// Renders TERMS.md/PRIVACY.md/LICENSE to src/data/legal.json for in-app display.
import { readFileSync, writeFileSync, mkdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";
import { marked } from "marked";

const SCRIPT_DIR = path.dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = path.join(SCRIPT_DIR, "..");
const OUT_DIR = path.join(REPO_ROOT, "src", "data");
const OUT_FILE = path.join(OUT_DIR, "legal.json");

function buildDoc(filename) {
  const raw = readFileSync(path.join(REPO_ROOT, filename), "utf-8");
  const updatedMatch = raw.match(/\*Last updated:\s*(.+?)\*/);
  const body = raw
    .replace(/^#\s.+\n/, "") // drop the H1 — the modal renders its own title
    .replace(/\n---\s*\n\*Last updated:.*\*\s*$/, ""); // drop the trailing date line
  return { html: marked.parse(body.trim()), updated: updatedMatch?.[1] ?? null };
}

// LICENSE is plain text, not markdown — rendered verbatim in a <pre>.
function buildPlainText(filename) {
  return { text: readFileSync(path.join(REPO_ROOT, filename), "utf-8").trim(), updated: null };
}

const docs = {
  terms: buildDoc("TERMS.md"),
  privacy: buildDoc("PRIVACY.md"),
  license: buildPlainText("LICENSE"),
};

mkdirSync(OUT_DIR, { recursive: true });
writeFileSync(OUT_FILE, JSON.stringify(docs, null, 2));
console.log(`[build-legal] wrote ${path.relative(process.cwd(), OUT_FILE)}`);
