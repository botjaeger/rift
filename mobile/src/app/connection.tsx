import { useLocalSearchParams, useRouter } from "expo-router";
import { useEffect } from "react";
import { useLock } from "@/runtime";
import { ConnectionSheet } from "@/ui/ConnectionSheet";

/** "Connection": how a computer is being reached (opened from Portals' status line and the pane's status strip). */
export default function ConnectionScreen() {
  const { key } = useLocalSearchParams<{ key: string }>();
  const router = useRouter();
  const lock = useLock();
  // The sheet is presented natively above the app's lock screen: when the app locks, close it.
  useEffect(() => {
    if (lock.locked) router.back();
  }, [lock.locked, router]);
  return <ConnectionSheet hostKey={key} />;
}
