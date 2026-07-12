import { useEffect } from "react";
import releaseData from "./data/releases.json";
import Header from "./components/Header.jsx";
import Hero from "./components/Hero.jsx";
import Features from "./components/Features.jsx";
import Shortcuts from "./components/Shortcuts.jsx";
import Download from "./components/Download.jsx";
import Changelog from "./components/Changelog.jsx";
import Footer from "./components/Footer.jsx";

export default function App() {
  const releases = releaseData.releases ?? [];
  const latestRelease = releases.find((r) => !r.prerelease) ?? releases[0] ?? null;

  // Retry scroll-to-hash once content is rendered, since the browser's
  // native scroll-on-load can fire before React renders the target section.
  useEffect(() => {
    if (!window.location.hash) return;
    const id = window.location.hash.slice(1);
    document.getElementById(id)?.scrollIntoView();
  }, []);

  return (
    <div className="min-h-screen">
      <Header />
      <main>
        <Hero downloads={latestRelease?.downloads ?? []} />
        <Features />
        <Shortcuts />
        <Download latestRelease={latestRelease} />
        <Changelog releases={releases} />
      </main>
      <Footer />
    </div>
  );
}
