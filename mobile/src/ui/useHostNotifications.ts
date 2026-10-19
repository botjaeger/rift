import { useState } from "react";
import { fleet } from "@/runtime";
import { useSingleFlight } from "./useSingleFlight";

/** Every computer-alert control reports a refused save and remains available to retry. */
export function useHostNotifications(
  key: string,
  onError: (message: string | null) => void,
  onSaved?: (on: boolean) => void,
) {
  const [saving, setSaving] = useState(false);
  const run = useSingleFlight();
  const change = (on: boolean) => {
    void run(async () => {
      setSaving(true);
      onError(null);
      try {
        await fleet.setHostNotifications(key, on);
        onSaved?.(on);
      } catch {
        onError("This phone could not save the notification setting. Try again.");
      } finally {
        setSaving(false);
      }
    });
  };
  return { saving, change };
}
