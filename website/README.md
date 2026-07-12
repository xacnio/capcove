# Capcove website

Marketing/docs site for Capcove. Vite + React, same color palette and font as
the desktop app (`src/styles.css`). Deployed to GitHub Pages from
`.github/workflows/deploy-docs.yml`.

```bash
npm install
npm run dev     # fetches latest release data, then starts the dev server
npm run build   # fetches latest release data, then builds to dist/
```

Two scripts run before every dev/build (via the `fetch-data` npm script):

- `scripts/fetch-releases.mjs` pulls release + asset data from the GitHub API
  into `src/data/releases.json`, so download links and the changelog stay in
  sync with whatever is actually published — no client-side fetch, no manual
  edits needed when a new version ships.
- `scripts/build-legal.mjs` renders the repo's `PRIVACY.md` and `TERMS.md`
  (one level up) to HTML into `src/data/legal.json`, served as `privacy.html`
  and `terms.html`. The website never holds a second copy of that text —
  edit the `.md` files at the repo root and the site picks it up on next
  build. The `deploy-docs.yml` workflow also redeploys on changes to either
  file, not just to `website/**`.

## One-time setup

In the GitHub repo: **Settings → Pages → Source → GitHub Actions**. After
that, every push to `main` touching `website/**`, every published release,
and manual `workflow_dispatch` runs redeploy the site.

If the repo isn't `xacnio/capcove` or isn't served at the project-pages path
(`https://<user>.github.io/<repo>/`), update `base` in `vite.config.js` and
`REPO` in `scripts/fetch-releases.mjs` accordingly. For a custom domain, add a
`public/CNAME` file and set `base: "/"`.

## Screenshots

`public/screenshots/{gallery-recordings,editor-timeline,recording-settings,replay,youtube-live}.webp`
are real app captures, referenced from the Hero, Screenshots and Features
components (`src/components/Screenshot.jsx` renders them, with the `src`/
`placeholder`/`note` props set per usage site). If one of those files is ever
missing, the component falls back to a dashed placeholder with capture
instructions instead of a broken image — see each usage site's `placeholder`/
`note` props for what that shot should show if you need to retake one.
