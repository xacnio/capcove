import { useEffect, useState } from "react";

/**
 * Wall-clock `now` (ms), re-rendering once per whole second via rAF instead
 * of `setInterval` — avoids timer coalescing/drift that bursts updates.
 */
export function useClockSeconds() {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    let raf = 0;
    let lastSec = Math.floor(Date.now() / 1000);
    const loop = () => {
      const t = Date.now();
      const sec = Math.floor(t / 1000);
      if (sec !== lastSec) {
        lastSec = sec;
        setNow(t);
      }
      raf = requestAnimationFrame(loop);
    };
    raf = requestAnimationFrame(loop);
    return () => cancelAnimationFrame(raf);
  }, []);
  return now;
}
