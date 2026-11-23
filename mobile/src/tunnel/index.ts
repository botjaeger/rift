/**
 * Open a page served on the computer's `localhost:PORT` in the phone's browser sheet (feature
 * `tunnel`): a listener on the phone's loopback (the rift-tunnel module), each of its
 * connections a stream (./streams) over a tunnel connection to the computer (./channel).
 * Spec: docs/remote-protocol.md, "Local web servers".
 */

import * as WebBrowser from "expo-web-browser";
import { AppState } from "react-native";
import { tunnel as native } from "../../modules/rift-tunnel";
import type { TunnelTarget } from "../protocol/client";
import { TunnelChannel } from "./channel";
import type { Frame } from "./frames";
import { Streams } from "./streams";
import { base64ToBytes, bytesToBase64 } from "./url";

export { localUrl } from "./url";

/** Whether this build can open the computer's local pages (the native listener is in it). */
export const tunnelAvailable = native !== null;

/** Frames held while the tunnel connection is still opening. */
const MAX_QUEUED = 256;

/** One computer port served on the phone's loopback, while its page is open. */
class Session {
  private channel: TunnelChannel | null = null;
  private queued: Frame[] = [];
  readonly streams: Streams;
  /** What went wrong was said: once per page, not once per request the browser makes. */
  private said = new Set<string>();
  private readonly onTrouble: (why: string) => void;

  constructor(
    readonly port: number,
    private readonly target: () => TunnelTarget | null,
    onTrouble: (why: string) => void,
  ) {
    this.onTrouble = (why) => {
      if (this.said.has(why)) return;
      this.said.add(why);
      onTrouble(why);
    };
    this.streams = new Streams(port, (f) => this.send(f), {
      write: async (stream, data) => {
        if (!(await native!.write(stream, bytesToBase64(data)))) throw new Error("closed");
      },
      close: (stream) => void native!.close(stream),
    });
  }

  private send(f: Frame): boolean {
    const ch = this.connect();
    if (!ch) return false;
    if (ch.open) return ch.send(f);
    if (this.queued.length >= MAX_QUEUED) return false;
    this.queued.push(f);
    return true;
  }

  private connect(): TunnelChannel | null {
    if (this.channel) return this.channel;
    const target = this.target();
    if (!target) {
      this.onTrouble("Not connected to the computer");
      return null;
    }
    const ch = new TunnelChannel(target, {
      onOpen: () => {
        const q = this.queued;
        this.queued = [];
        for (const f of q) ch.send(f);
      },
      onFrame: (f) => this.streams.onFrame(f),
      onClose: (why) => {
        if (this.channel !== ch) return;
        this.channel = null;
        this.queued = [];
        this.streams.closeAll();
        if (why === "forbidden") this.onTrouble("This phone is paired as view only");
      },
    });
    this.channel = ch;
    return ch;
  }

  stop(): void {
    this.channel?.close("stopped");
    this.channel = null;
    this.streams.closeAll();
  }
}

let current: Session | null = null;
let subscribed = false;

function listen(): void {
  if (subscribed || !native) return;
  subscribed = true;
  native.addListener("onOpen", ({ stream }) => current?.streams.localOpen(stream));
  native.addListener("onData", ({ stream, data }) => current?.streams.localData(stream, base64ToBytes(data)));
  native.addListener("onEnd", ({ stream }) => current?.streams.localEnd(stream));
  native.addListener("onClose", ({ stream }) => current?.streams.localClose(stream));
}

/** Resolves once the app has left the front and come back (the custom tab over it closed). */
function backInFront(): Promise<void> {
  return new Promise((resolve) => {
    let left = AppState.currentState !== "active";
    const sub = AppState.addEventListener("change", (state) => {
      if (state !== "active") left = true;
      else if (left) {
        sub.remove();
        resolve();
      }
    });
  });
}

/**
 * Open `localhost:port` + `path` of the computer in the browser sheet; resolves when the sheet
 * closes (the listener and the tunnel stop then). `target` reads the session's path at each
 * new tunnel connection. Throws when this build has no listener.
 */
export async function openLocal(
  port: number,
  path: string,
  target: () => TunnelTarget | null,
  onTrouble: (why: string) => void,
): Promise<void> {
  if (!native) throw new Error("This version of the app cannot open the computer's pages; update it");
  listen();
  current?.stop();
  await native.stop();
  const session = new Session(port, target, onTrouble);
  current = session;
  try {
    // the same port number where it is free: dev servers that check the Host header see theirs
    const bound = await native.listen(port);
    const url = `http://127.0.0.1:${bound}${path}`;
    // Android: the module's custom tab binds the app's keep-alive service, without which Android
    // 15 cuts the app's network (and the tunnel) about 5 s after the tab covers it. Its tab, like
    // expo-web-browser's there, is open until the app is in front again.
    if (native.openTab && (await native.openTab(url))) await backInFront();
    else if ((await WebBrowser.openBrowserAsync(url)).type === "opened") await backInFront();
  } finally {
    if (current === session) {
      session.stop();
      current = null;
      await native.stop();
    }
  }
}
