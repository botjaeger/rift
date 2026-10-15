// Which computer Now shows (the picker under its title): one computer's key, or null for all.
// Kept for the app run, not across launches.

import { useSyncExternalStore } from "react";

let only: string | null = null;
const listeners = new Set<() => void>();

export function setNowFilter(key: string | null): void {
  only = key;
  listeners.forEach((l) => l());
}

export function useNowFilter(): string | null {
  return useSyncExternalStore(
    (l) => {
      listeners.add(l);
      return () => listeners.delete(l);
    },
    () => only,
  );
}
