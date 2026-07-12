// Some WebKit builds (macOS WKWebView) render `color-mix(in oklab|oklch|lab|lch,
// <color>, transparent)` as a washed-out gray instead of translucent — a real WebKit
// bug. Stripping these rules lets Tailwind's legacy hex/rgb fallback win instead.
const stripColorMixFallback = () => ({
  postcssPlugin: "strip-color-mix-fallback",
  Declaration(decl) {
    if (/color-mix\(\s*in\s+(oklab|oklch|lab|lch)/i.test(decl.value)) {
      decl.remove();
    }
  },
  OnceExit(root) {
    root.walkRules((rule) => {
      if (rule.nodes.length === 0) rule.remove();
    });
  },
});
stripColorMixFallback.postcss = true;

export default {
  plugins: [stripColorMixFallback()],
};
