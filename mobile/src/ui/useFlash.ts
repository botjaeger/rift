import { useEffect, useRef, useState } from "react";

/** How long a "Copied" stays where the copy was made (a button, the selection's own notice). */
export const COPIED_MS = 1600;

/** A control's "Copied" moment: true for `COPIED_MS` after `flash()`, the timer cleared on unmount. */
export function useFlash(): [boolean, () => void] {
  const [on, setOn] = useState(false);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  useEffect(() => () => clearTimeout(timer.current), []);
  return [
    on,
    () => {
      clearTimeout(timer.current);
      setOn(true);
      timer.current = setTimeout(() => setOn(false), COPIED_MS);
    },
  ];
}
