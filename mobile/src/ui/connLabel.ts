// Connection status text for the status lines, free of React Native imports (tested in node).

import type { ConnState } from "../protocol/client";
import { pathLabel } from "../protocol/relay";
import type { ColorName } from "./palette";

/** Why a client stopped for good, in full (the Connection sheet, the pane's failure card). */
export function failureLabel(conn: ConnState): string {
  switch (conn.failure) {
    case "auth":
      return "This computer no longer accepts this device. Pair it again.";
    case "key_mismatch":
      return "A different computer answered at this address (its key does not match). Not connected.";
    case "version":
      return "This computer runs a Rift version this app cannot talk to.";
    case "unreachable":
      return "Could not reach the computer.";
    case "gave_up":
      return "Rift stopped trying to reach the computer. Check that it is on and Rift is running there.";
    case "host_quit":
      return "Rift quit on this computer. Open Rift there, then tap Try again.";
    default:
      return conn.error ?? "Stopped";
  }
}

/** The status text and dot color (a palette name); the dot is decoration, the text carries the meaning. */
export function describeConn(conn: ConnState): { label: string; color: ColorName } {
  const via = conn.path ? ` · ${pathLabel(conn.path)}` : "";
  if (conn.closing?.reason === "update" && (conn.status === "connecting" || (conn.status === "closed" && conn.retryInMs !== null)))
    return { label: "Rift is updating on the computer · reconnecting by itself", color: "accent" };
  switch (conn.status) {
    case "open":
      return { label: `Connected${via}`, color: "success" };
    case "connecting":
      return {
        label: `${conn.attempt > 0 ? `Reconnecting (attempt ${conn.attempt + 1})` : "Connecting"}${via}…`,
        color: "accent",
      };
    case "failed":
      return { label: failureLabel(conn), color: "error" };
    case "offline":
      return { label: "Offline: this device has no network", color: "textDim" };
    case "closed":
      return conn.retryInMs !== null
        ? {
            label: `Disconnected${conn.error ? ` (${conn.error})` : ""}, retrying in ${Math.ceil(conn.retryInMs / 1000)} s`,
            color: "error",
          }
        : { label: "Disconnected", color: "textDim" };
    case "idle":
      return { label: "Not connected", color: "textDim" };
  }
}

