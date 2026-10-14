import { useState } from "react";

export type SingleFlight = (task: () => Promise<void>) => Promise<void>;

/**
 * One task at a time: a call while the last one is still on its way is dropped (resolves at
 * once, without running), so a double tap on Run again sends the command once (it ran twice
 * before). A task that rejects frees the slot and passes the rejection on.
 */
export function createSingleFlight(): SingleFlight {
  let busy = false;
  return async (task) => {
    if (busy) return;
    busy = true;
    try {
      await task();
    } finally {
      busy = false;
    }
  };
}

/** `createSingleFlight` for one component, kept across its renders. */
export function useSingleFlight(): SingleFlight {
  const [run] = useState(createSingleFlight);
  return run;
}
