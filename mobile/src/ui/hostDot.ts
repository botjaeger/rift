import type { HostView } from "@/fleet";
import { updating } from "./connStatus";
import type { ColorName } from "./palette";

/** A computer's status dot (a palette name): green while connected, the accent while trying, red when it cannot be reached. */
export function hostDot(host: HostView): ColorName {
  // an update restart is not a fault, and a quit is the user's own doing
  if (updating(host.conn)) return "accent";
  if (host.conn.failure === "host_quit") return "textDim";
  switch (host.conn.status) {
    case "open":
      return "success";
    case "connecting":
      return "accent";
    case "closed":
      return host.conn.retryAt !== null || host.conn.retryInMs !== null ? "error" : "textDim";
    case "failed":
      return "error";
    default:
      return "textDim";
  }
}
