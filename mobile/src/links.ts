/**
 * Opening a link tapped in a pane's output. The system opens it, unless the app installed a
 * handler at start (src/tunnel/open: the computer's own `localhost` pages go through the tunnel).
 * Kept apart from that handler so views (and their tests) need no app runtime.
 */

import { Linking } from "react-native";

let handler: (url: string) => void = (url) => void Linking.openURL(url);

export function setLinkHandler(h: (url: string) => void): void {
  handler = h;
}

export function openLink(url: string): void {
  handler(url);
}
