// Reads the latest Rift release from the public botjaeger/rift-releases repo at build time.
// Asset names carry the version (Rift-0.29.0-arm64.dmg), so /releases/latest/download/<name>
// URLs would break on the next release; the site is rebuilt instead (see the deploy workflow).
// The build fails when the API or an expected asset is missing, so a broken link never ships.

const REPO = 'botjaeger/rift-releases';
export const RELEASES_URL = `https://github.com/${REPO}/releases`;
export const REPO_URL = `https://github.com/${REPO}`;

interface ApiAsset {
  name: string;
  browser_download_url: string;
}
interface ApiRelease {
  tag_name: string;
  html_url: string;
  published_at: string;
  assets: ApiAsset[];
}

export interface Download {
  name: string;
  url: string;
}

export interface Release {
  version: string;
  date: string;
  isoDate: string;
  notesUrl: string;
  macArm: Download;
  macIntel: Download;
  deb: Download;
  appImage: Download;
  tarMacArm: Download;
  tarMacIntel: Download;
  tarLinux: Download;
  /** The Windows beta zip; releases before it have none (the site then names no Windows). */
  windows?: Download;
}

/** The platforms a release ships for, as the site's copy names them. */
export function platforms(r: Release): string {
  return r.windows ? 'macOS, Linux and Windows' : 'macOS and Linux';
}

const PATTERNS = {
  macArm: /^Rift-[\d.]+-arm64\.dmg$/,
  macIntel: /^Rift-[\d.]+-x86_64\.dmg$/,
  deb: /^rift-terminal_[\d.]+_amd64\.deb$/,
  appImage: /^Rift-[\d.]+-x86_64\.AppImage$/,
  tarMacArm: /^rift-terminal-[\d.]+-macos-arm64\.tar\.gz$/,
  tarMacIntel: /^rift-terminal-[\d.]+-macos-x86_64\.tar\.gz$/,
  tarLinux: /^rift-terminal-[\d.]+-linux-x86_64\.tar\.gz$/,
  windows: /^Rift-[\d.]+-windows-x86_64\.zip$/,
} as const;

let cached: Promise<Release> | undefined;

export function getRelease(): Promise<Release> {
  cached ??= load();
  return cached;
}

async function load(): Promise<Release> {
  const headers: Record<string, string> = {
    Accept: 'application/vnd.github+json',
    'User-Agent': 'rift-site-build',
  };
  const token = process.env.GITHUB_TOKEN;
  if (token) headers.Authorization = `Bearer ${token}`;

  const res = await fetch(`https://api.github.com/repos/${REPO}/releases/latest`, { headers });
  if (!res.ok) throw new Error(`GitHub API ${res.status} for ${REPO} latest release: ${await res.text()}`);
  const data = (await res.json()) as ApiRelease;

  const optional = (key: keyof typeof PATTERNS): Download | undefined => {
    const asset = data.assets.find((a) => PATTERNS[key].test(a.name));
    return asset && { name: asset.name, url: asset.browser_download_url };
  };
  const find = (key: keyof typeof PATTERNS): Download => {
    const asset = optional(key);
    if (!asset) throw new Error(`Release ${data.tag_name} has no asset matching ${PATTERNS[key]}`);
    return asset;
  };

  const published = new Date(data.published_at);
  return {
    version: data.tag_name.replace(/^v/, ''),
    date: published.toLocaleDateString('en-US', { year: 'numeric', month: 'long', day: 'numeric', timeZone: 'UTC' }),
    isoDate: data.published_at.slice(0, 10),
    notesUrl: data.html_url,
    macArm: find('macArm'),
    macIntel: find('macIntel'),
    deb: find('deb'),
    appImage: find('appImage'),
    tarMacArm: find('tarMacArm'),
    tarMacIntel: find('tarMacIntel'),
    tarLinux: find('tarLinux'),
    windows: optional('windows'),
  };
}
