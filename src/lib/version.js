// release.yml's body has a Download Links table + What's New section + CI
// footer; only the latter is relevant in-app (the table just duplicates the
// website's own download buttons, and isn't even reachable from inside the app).
export function extractChangelog(body) {
  const match = body.match(/What's New\s*\n([\s\S]*?)(\n---|\n\*Automatically generated|$)/);
  return match ? match[1].trim() : body.trim();
}

// Numeric dotted-version compare (e.g. "1.0.2" vs "1.0.10"); no semver pre-release/build tags.
export function compareVersions(a, b) {
  const pa = a.replace(/^v/, "").split(".").map(Number);
  const pb = b.replace(/^v/, "").split(".").map(Number);
  for (let i = 0; i < Math.max(pa.length, pb.length); i++) {
    const diff = (pa[i] || 0) - (pb[i] || 0);
    if (diff !== 0) return diff;
  }
  return 0;
}
