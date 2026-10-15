// A pane's avatar (the Computers, Host and Now boards): a round mark in a colour of its own, so
// the same project reads the same everywhere. Pure.

/** The avatars' colours: the accent, then the editor theme's syntax colours (DESIGN.md). */
export const AVATAR_COLORS = ["#569cd6", "#22c55e", "#ce9178", "#9cdcfe", "#c586c0", "#4ec9b0", "#dcdcaa"] as const;

export interface Avatar {
  color: string;
  /** Claude Code's ✳ in place of the terminal icon; null for the icon. */
  glyph: string | null;
}

/** The folder a pane is in, as its avatar names it: the path's last part ("~" for home). */
export function folderOf(cwd: string | null | undefined): string | null {
  if (!cwd) return null;
  const parts = cwd.split("/").filter(Boolean);
  return parts.at(-1) ?? cwd;
}

/** A small stable hash (FNV-1a) of `s`. */
function hash(s: string): number {
  let h = 0x811c9dc5;
  for (let i = 0; i < s.length; i++) {
    h ^= s.charCodeAt(i);
    h = Math.imul(h, 0x01000193);
  }
  return h >>> 0;
}

/**
 * The avatar of a pane in `cwd` (else titled `title`) that runs `program`: its colour from the
 * folder's name, so a project's panes and tabs share one; Claude Code's mark while it runs.
 */
export function avatarFor(cwd: string | null | undefined, title: string, program: string | null): Avatar {
  const key = folderOf(cwd) ?? title;
  return {
    color: AVATAR_COLORS[hash(key) % AVATAR_COLORS.length]!,
    glyph: program === "claude" ? "✳" : null,
  };
}
