import { invoke } from "./tauri.js";

// Convenience-only translation; English text stays authoritative.
const cache = new Map();

async function translateOne(text, target) {
  const key = `${target}:${text}`;
  if (cache.has(key)) return cache.get(key);
  const result = await invoke("translate_text", { text, target }).catch(() => text);
  cache.set(key, result);
  return result;
}

// Translates only leaf text nodes, so tags (links, bold, code) stay intact.
export async function translateHtml(html, target) {
  const doc = new DOMParser().parseFromString(html, "text/html");
  const walker = doc.createTreeWalker(doc.body, NodeFilter.SHOW_TEXT);
  const nodes = [];
  let node;
  while ((node = walker.nextNode())) {
    if (node.parentElement?.closest("code")) continue;
    if (!node.nodeValue.trim()) continue;
    nodes.push(node);
  }
  await Promise.all(nodes.map(async (n) => {
    n.nodeValue = await translateOne(n.nodeValue, target);
  }));
  return doc.body.innerHTML;
}

export function translateText(text, target) {
  return translateOne(text, target);
}
