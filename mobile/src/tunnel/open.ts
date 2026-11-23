/**
 * Opening a link tapped in a pane: one on the computer itself (`http://localhost:5173`) goes
 * through the tunnel when the computer serves it to this phone (src/tunnel), anything else to
 * the system as before.
 */

import { Alert, Linking } from "react-native";
import { canTunnel } from "../fleet";
import { fleet } from "../runtime";
import { openLocal, tunnelAvailable } from "./index";
import { localUrl } from "./url";

let opening = false;

function trouble(why: string): void {
  Alert.alert("Couldn't open the page", why);
}

/** Whether `url` opens through the tunnel from the pane on show (else the system opens it). */
export function opensThroughTunnel(url: string): boolean {
  const host = fleet.paneHost;
  return tunnelAvailable && localUrl(url) !== null && host !== null && canTunnel(fleet.host(host));
}

/** Open `url` from the pane on show: the computer's own pages through the tunnel. */
export function openLink(url: string): void {
  const local = localUrl(url);
  const host = fleet.paneHost;
  if (!local || !host || !opensThroughTunnel(url)) {
    void Linking.openURL(url);
    return;
  }
  openPort(host, local.port, local.path);
}

/** Open `localhost:port` + `path` of computer `host` in the browser sheet. */
export function openPort(host: string, port: number, path = "/"): void {
  if (opening) return;
  opening = true;
  openLocal(port, path, () => fleet.tunnelTarget(host), trouble)
    .catch((e: unknown) => trouble(e instanceof Error ? e.message : String(e)))
    .finally(() => {
      opening = false;
    });
}
