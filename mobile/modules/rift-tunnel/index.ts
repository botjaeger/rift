// A TCP listener on this phone's loopback (iOS: ios/RiftTunnelModule.swift, Android:
// android/…/RiftTunnelModule.kt). Each connection to it is a stream whose bytes come and go as
// base64, for JS to forward to the computer. Null on web and in a build without the module.
import { requireOptionalNativeModule } from "expo";

export interface TunnelEvents {
  onOpen(e: { stream: number }): void;
  /** `data` is base64. */
  onData(e: { stream: number; data: string }): void;
  /** The peer finished sending (half-close); the stream stays writable until `close`. */
  onEnd(e: { stream: number }): void;
  /** The stream is gone: closed by `close`, `stop`, a reset or a failed write. */
  onClose(e: { stream: number }): void;
}

export interface RiftTunnelNative {
  /** Binds 127.0.0.1:`port`, or an ephemeral port when that one is taken; resolves with the bound port. */
  listen(port: number): Promise<number>;
  /** Closes the listener and every open stream. */
  stop(): Promise<void>;
  /** Queues bytes (base64) on a stream; false when the stream is gone. */
  write(stream: number, base64: string): Promise<boolean>;
  /** Ends a stream after its queued writes. */
  close(stream: number): Promise<void>;
  /**
   * Android only (absent on iOS): opens `url` in a custom tab that keeps the app's network open
   * while it covers the app (the browser binds the keep-alive service). Resolves at once, true
   * when it opened; the tab is closed when the app is in front again.
   */
  openTab?(url: string): Promise<boolean>;
  addListener<K extends keyof TunnelEvents>(event: K, listener: TunnelEvents[K]): { remove(): void };
}

export const tunnel = requireOptionalNativeModule<RiftTunnelNative>("RiftTunnel");
