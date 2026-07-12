// Replaces the __SITE_URL__ placeholder (canonical/hreflang tags) in every HTML
// entry point at build time, so the same source files work for any deploy target.
export function siteUrlPlugin(siteUrl) {
  return {
    name: "site-url",
    transformIndexHtml(html) {
      return html.replaceAll("__SITE_URL__", siteUrl);
    },
  };
}
