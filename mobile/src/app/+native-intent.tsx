// Incoming links. A pairing link (rift-remote://pair?…, e.g. from the system camera scanning the
// desktop's QR code) opens the pairing confirmation with the whole link, which parses it; it
// never pairs by itself.

export function redirectSystemPath({ path }: { path: string; initial: boolean }): string | null {
  try {
    if (/^rift-remote:\/\/pair\b/i.test(path)) return `/pair?link=${encodeURIComponent(path)}`;
    return path;
  } catch {
    return "/";
  }
}
