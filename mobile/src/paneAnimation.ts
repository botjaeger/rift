// How the pane screen comes in when the tab row's ‹ or › replaces it: the previous tab from the
// left, the next from the right, so the motion matches the direction. Anything else (opened
// from a list, a notification) keeps the platform's push. Pure.

export type PaneAnimation = "default" | "slide_from_left" | "slide_from_right";

export function paneAnimation(dir: string | undefined): PaneAnimation {
  if (dir === "prev") return "slide_from_left";
  if (dir === "next") return "slide_from_right";
  return "default";
}
