// Rift's dark tokens (DESIGN.md: Warp's Dark colours since 2026-10-16), free of React Native imports so the contrast test runs in
// node. Mapping: bg = bg, chrome = bg_chrome, surface = surface, hairline = border,
// textBright = text_bright, text = text_ui, textMuted = text_dim (#a1a4ad), textDim =
// text_muted (#868993), accent, success, error, warning, fg = fg (terminal text, block output),
// borderStrong = the mockups' input and outlined-button border (white 14 %).

export const colors = {
  bg: "#050505",
  chrome: "#000000",
  surface: "#1e1e1e",
  hairline: "#ffffff1a",
  textBright: "#ffffff",
  text: "#e6e6e6",
  textMuted: "#9b9b9b",
  textDim: "#828282",
  accent: "#19aad8",
  success: "#1ca05a",
  error: "#ff8272",
  warning: "#e5a01a",
  /** ANSI magenta (Warp Dark): a merged pull request's chip. */
  magenta: "#ff8ffd",
  /** The PHP mark's indigo: the accent mixed half with magenta, as the desktop mixes it. */
  indigo: "#8c9cea",
  fg: "#ffffff",
  borderStrong: "#ffffff24",
  /** The boards' glass controls (round back badges, header pills): white 8 % over a 12 % rim. */
  glass: "#ffffff14",
  glassRim: "#ffffff1f",
  /** The floating search bar's ground, as the floating tab bar's. */
  floating: "#1e1e1ef0",
  /** The Pair board's ground behind the camera, and the viewfinder before it starts. */
  camera: "#000000",
  viewfinder: "#121212",
} as const;

export type ColorName = keyof typeof colors;

/**
 * Every text color the app draws, on each background it is drawn on. test/contrast.test.ts
 * holds each pair to WCAG AA (4.5:1). One token falls short and is kept off a ground:
 * `textDim` on surface (4.33), where `textMuted` is used instead.
 */
export const TEXT_ON: Readonly<Record<"bg" | "chrome" | "surface" | "accent" | "warning" | "camera", readonly ColorName[]>> = {
  bg: ["textBright", "text", "fg", "textMuted", "textDim", "accent", "success", "warning", "error", "magenta", "indigo"],
  chrome: ["textBright", "text", "textMuted", "textDim", "accent", "success", "warning", "error", "magenta", "indigo"],
  camera: ["textBright", "text", "textMuted", "warning"],
  surface: ["textBright", "text", "textMuted", "accent", "success", "warning", "error"],
  accent: ["bg"],
  // the count on a back badge (the panes that need you)
  warning: ["bg"],
};

/**
 * The mockups' tinted grounds (a token at low alpha over a card or page): the accent pills
 * ("On screen", a running pane's time, step numbers) and the badges of the Host board. The
 * mockups' `#19aad822` puts accent text at 4.34:1 on a surface, so the accent tint is `1a`
 * (4.58:1). TINTED_TEXT lists each text drawn on one; test/contrast.test.ts checks them.
 */
export const tints = {
  accentBadge: "#19aad81a",
  successBadge: "#1ca05a14",
  warningBadge: "#e5a01a1f",
  errorBadge: "#ff827233",
  /** A prompt chip's fill (white 4 %, on chrome), as the desktop's `chip_fill`. */
  chipFill: "#ffffff0a",
  /** The running block's ground (on bg). */
  running: "#19aad814",
  /** The bell that is on (on the running block's ground). */
  bellOn: "#19aad833",
  /** A diff's added and removed lines (on the chrome reading pane), as the desktop review panel. */
  diffAdded: "#1ca05a1f",
  diffRemoved: "#ff827226",
  /** The answer page's context diff (the P2 Reply board), on the chrome box. */
  answerAdded: "#1ca05a1f",
  answerRemoved: "#ff82721f",
} as const;

export const TINTED_TEXT: readonly { fg: ColorName; tint: keyof typeof tints; ground: "bg" | "chrome" | "surface" }[] = [
  // the prompt chips' parts, on the chip fill over the command line's chrome ground
  ...(["textBright", "text", "textMuted", "textDim", "accent", "success", "warning", "error", "magenta", "indigo"] as const).map(
    (fg) => ({ fg, tint: "chipFill" as const, ground: "chrome" as const }),
  ),
  { fg: "accent", tint: "accentBadge", ground: "surface" },
  { fg: "accent", tint: "accentBadge", ground: "chrome" },
  // the Run sheet's first completion (surface) and its kind
  { fg: "textBright", tint: "accentBadge", ground: "surface" },
  { fg: "textMuted", tint: "accentBadge", ground: "surface" },
  { fg: "success", tint: "successBadge", ground: "surface" },
  { fg: "warning", tint: "warningBadge", ground: "surface" },
  { fg: "textBright", tint: "errorBadge", ground: "surface" },
  { fg: "textBright", tint: "running", ground: "bg" },
  { fg: "fg", tint: "running", ground: "bg" },
  { fg: "accent", tint: "running", ground: "bg" },
  { fg: "success", tint: "running", ground: "bg" },
  { fg: "fg", tint: "diffAdded", ground: "chrome" },
  { fg: "success", tint: "diffAdded", ground: "chrome" },
  { fg: "fg", tint: "diffRemoved", ground: "chrome" },
  { fg: "textBright", tint: "diffRemoved", ground: "chrome" },
  { fg: "textBright", tint: "answerAdded", ground: "chrome" },
  { fg: "textBright", tint: "answerRemoved", ground: "chrome" },
];

/** `#rrggbbaa` over the opaque `ground`, as one opaque `#rrggbb`. */
export function over(rgba: string, ground: string): string {
  const a = parseInt(rgba.slice(7, 9), 16) / 255;
  const ch = (hex: string, i: number) => parseInt(hex.slice(1 + i * 2, 3 + i * 2), 16);
  return `#${[0, 1, 2].map((i) => Math.round(ch(rgba, i) * a + ch(ground, i) * (1 - a)).toString(16).padStart(2, "0")).join("")}`;
}

/** WCAG 2 relative luminance of an `#rrggbb` color. */
export function luminance(hex: string): number {
  const ch = [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16) / 255);
  const [r, g, b] = ch.map((v) => (v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4)) as [number, number, number];
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

/** WCAG 2 contrast ratio of two opaque `#rrggbb` colors. */
export function contrast(a: string, b: string): number {
  const x = luminance(a);
  const y = luminance(b);
  return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05);
}
