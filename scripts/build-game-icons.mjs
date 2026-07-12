// Builds src-tauri/resources/game_icons.pack: downloads each catalog game's icon,
// converts to 64px WebP, and packs into one ~13MB file read lazily from disk (bundled
// as a resource, not include_bytes!-embedded — see games_db.rs's ArtPack).
// Format: [u32 LE index length][index JSON][blob bytes].

import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { execFile } from "node:child_process";
import { promisify } from "node:util";

const execFileP = promisify(execFile);

const ROOT = path.resolve(path.dirname(new URL(import.meta.url).pathname.replace(/^\/(\w:)/, "$1")), "..");
const CATALOG = path.join(ROOT, "src-tauri", "resources", "games_catalog.json");
const OUT = path.join(ROOT, "src-tauri", "resources", "game_icons.pack");
// Not committed to git (see .gitignore — ~137MB, over GitHub's push limit),
// so a fresh clone won't have this until you download it yourself; this
// script only runs locally, on demand, never in CI.
const FFMPEG = path.join(ROOT, "src-tauri", "binaries", "ffmpeg-x86_64-pc-windows-msvc.exe");
if (!fs.existsSync(FFMPEG)) {
  console.error(`ffmpeg not found at ${FFMPEG}`);
  console.error("This binary isn't committed to git (see .gitignore) — download the pinned build");
  console.error("release.yml/build-artifacts.yml fetch (BtbN/FFmpeg-Builds) and place it there yourself.");
  process.exit(1);
}
const TMP = fs.mkdtempSync(path.join(os.tmpdir(), "capcove-icons-"));

const CONCURRENCY = 16;

const catalog = JSON.parse(fs.readFileSync(CATALOG, "utf8"));
// One icon per display name (multiple exes share a game entry).
const targets = new Map();
for (const app of catalog) {
  if (app.id && app.icon_hash && !targets.has(app.name)) {
    targets.set(app.name, `https://cdn.discordapp.com/app-icons/${app.id}/${app.icon_hash}.png?size=64`);
  }
}
console.log(`games with icons: ${targets.size}`);

let done = 0;
let failed = 0;
const results = new Map(); // name -> Buffer (webp)

async function processOne(name, url, slot) {
  const pngPath = path.join(TMP, `i${slot}.png`);
  const webpPath = path.join(TMP, `i${slot}.webp`);
  try {
    const resp = await fetch(url);
    if (!resp.ok) throw new Error(`http ${resp.status}`);
    fs.writeFileSync(pngPath, Buffer.from(await resp.arrayBuffer()));
    await execFileP(FFMPEG, [
      "-y", "-hide_banner", "-loglevel", "error",
      "-i", pngPath,
      "-vf", "scale=64:64",
      "-c:v", "libwebp", "-quality", "70",
      webpPath,
    ]);
    results.set(name, fs.readFileSync(webpPath));
  } catch (e) {
    failed++;
  } finally {
    done++;
    if (done % 250 === 0) console.log(`${done}/${targets.size} (failed: ${failed})`);
    for (const p of [pngPath, webpPath]) { try { fs.unlinkSync(p); } catch {} }
  }
}

const entries = [...targets.entries()];
const workers = Array.from({ length: CONCURRENCY }, async (_, w) => {
  for (let i = w; i < entries.length; i += CONCURRENCY) {
    await processOne(entries[i][0], entries[i][1], w);
  }
});
await Promise.all(workers);

// Pack: index json + concatenated blobs.
const index = {};
const blobs = [];
let offset = 0;
for (const [name, buf] of results) {
  index[name] = [offset, buf.length];
  blobs.push(buf);
  offset += buf.length;
}
const indexBuf = Buffer.from(JSON.stringify(index), "utf8");
const header = Buffer.alloc(4);
header.writeUInt32LE(indexBuf.length, 0);
fs.writeFileSync(OUT, Buffer.concat([header, indexBuf, ...blobs]));

fs.rmSync(TMP, { recursive: true, force: true });
console.log(`packed ${results.size} icons (${failed} failed) -> ${OUT} (${(fs.statSync(OUT).size / 1048576).toFixed(1)} MB)`);
