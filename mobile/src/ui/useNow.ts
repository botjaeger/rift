import { useEffect, useState } from "react";

/** The wall clock, re-read every `everyMs` while `on` (relative times, running clocks). */
export function useNow(everyMs: number, on = true): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!on) return;
    const t = setInterval(() => setNow(Date.now()), everyMs);
    return () => clearInterval(t);
  }, [everyMs, on]);
  return now;
}
