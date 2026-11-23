/** Pure helpers of the tunnel (feature `tunnel`): which links it opens, base64 for the listener. */

/**
 * A URL on the computer itself (`localhost`, `127.0.0.1`, `0.0.0.0`, `[::1]`) with a port: what
 * the phone opens through the tunnel. Its port and path, else null.
 */
export function localUrl(url: string): { port: number; path: string } | null {
  const m = /^https?:\/\/(localhost|127\.0\.0\.1|0\.0\.0\.0|\[::1\]):(\d{1,5})(\/\S*)?$/i.exec(url.trim());
  if (!m) return null;
  const port = Number(m[2]);
  if (port < 1 || port > 65535) return null;
  return { port, path: m[3] ?? "/" };
}

/** A local page a pane printed (a dev server's "Local: http://localhost:5173/"). */
export interface LocalLink {
  url: string;
  port: number;
  path: string;
}

/**
 * The local pages in `texts` (a pane's outputs and screen, newest first), one per port, newest
 * first, at most `max`.
 */
export function localLinksIn(texts: readonly string[], max = 4): LocalLink[] {
  const seen = new Set<number>();
  const out: LocalLink[] = [];
  const re = /https?:\/\/(?:localhost|127\.0\.0\.1|0\.0\.0\.0|\[::1\]):\d{1,5}(?:\/[^\s"'<>)\]]*)?/gi;
  for (const text of texts) {
    const found = [...text.matchAll(re)].map((m) => m[0].replace(/[.,;:]+$/, "")).reverse();
    for (const url of found) {
      const l = localUrl(url);
      if (!l || seen.has(l.port)) continue;
      seen.add(l.port);
      out.push({ url, ...l });
      if (out.length >= max) return out;
    }
  }
  return out;
}

export function bytesToBase64(b: Uint8Array): string {
  let s = "";
  for (let i = 0; i < b.length; i += 0x8000) s += String.fromCharCode(...b.subarray(i, i + 0x8000));
  return btoa(s);
}

export function base64ToBytes(b64: string): Uint8Array {
  const s = atob(b64);
  const out = new Uint8Array(s.length);
  for (let i = 0; i < s.length; i++) out[i] = s.charCodeAt(i);
  return out;
}
