import type { HostView } from "@/fleet";
import { updating } from "./connStatus";
import { colors } from "./theme";

/** A computer's status dot: green while connected, the accent while trying, red when it cannot be reached. */
export function hostDot(host: HostView): string {
  // an update restart is not a fault, and a quit is the user's own doing
  if (updating(host.conn)) return colors.accent;
  if (host.conn.failure === "host_quit") return colors.textDim;
  switch (host.conn.status) {
    case "open":
      return colors.success;
    case "connecting":
      return colors.accent;
    case "closed":
      return host.conn.retryAt !== null || host.conn.retryInMs !== null ? colors.error : colors.textDim;
    case "failed":
      return colors.error;
    default:
      return colors.textDim;
  }
}
