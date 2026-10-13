// Rift's dark tokens (DESIGN.md), free of React Native imports so the contrast test runs in
// node. Mapping: bg = bg, chrome = bg_chrome, surface = surface, hairline = border,
// textBright = text_bright, text = text_ui, textMuted = text_dim (#a1a4ad), textDim =
// text_muted (#868993), accent, success, error, warning.

export const colors = {
  bg: "#1e1e1e",
  chrome: "#181818",
  surface: "#232326",
  hairline: "#ffffff12",
  textBright: "#f0f1f3",
  text: "#e6e7ea",
  textMuted: "#a1a4ad",
  textDim: "#868993",
  accent: "#569cd6",
  success: "#22c55e",
  error: "#ef4444",
  warning: "#d7ba7d",
} as const;

export type ColorName = keyof typeof colors;

/**
 * Every text color the app draws, on each background it is drawn on. test/contrast.test.ts
 * holds each pair to WCAG AA (4.5:1). Two tokens fall short and are kept off these grounds:
 * `error` on bg/surface (4.43 / 4.16), so errors there are `text` with a red bar, failed exit
 * badges there are `text` beside the red edge, and destructive actions use `warning`; and
 * `textDim` on surface (4.49), where `textMuted` is used instead.
 */
export const TEXT_ON: Readonly<Record<"bg" | "chrome" | "surface" | "accent", readonly ColorName[]>> = {
  bg: ["textBright", "text", "textMuted", "textDim", "accent", "success", "warning"],
  chrome: ["textBright", "text", "textMuted", "textDim", "accent", "success", "warning", "error"],
  surface: ["textBright", "text", "textMuted", "accent", "success", "warning"],
  accent: ["bg"],
};

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
