/**
 * The opening animation's timeline, in ms (the "C · Mark as the prompt" splash design): the
 * mark `>▌` waits at an empty prompt and its cursor blinks once, "rift" is typed with the cursor
 * moving along, then the splash fades to the app. Under a second in all.
 */
export const SPLASH = {
  word: "rift",
  /** The cursor is off for this span of the wait, then on again as typing starts. */
  blinkOff: { from: 200, to: 400 },
  /** When each letter of `word` appears. */
  typeAt: [400, 470, 540, 610],
  /** How long the finished prompt stays before the fade starts. */
  hold: 120,
  fade: 180,
  /** With reduced motion: the finished prompt, shown this long, then the fade. */
  still: 300,
} as const;

/** What the splash shows `t` ms in: how many letters are typed and whether the cursor is lit. */
export function splashFrame(t: number, reduceMotion = false): { typed: number; cursor: boolean } {
  if (reduceMotion) return { typed: SPLASH.word.length, cursor: true };
  const typed = SPLASH.typeAt.filter((at) => at <= t).length;
  const cursor = !(t >= SPLASH.blinkOff.from && t < SPLASH.blinkOff.to);
  return { typed, cursor };
}

/** When the frame can change (each blink edge and letter), in order. */
export function splashSteps(reduceMotion = false): number[] {
  return reduceMotion ? [] : [SPLASH.blinkOff.from, SPLASH.blinkOff.to, ...SPLASH.typeAt];
}

/** When the fade starts. */
export function splashFadeAt(reduceMotion = false): number {
  return reduceMotion ? SPLASH.still : Math.max(...SPLASH.typeAt) + SPLASH.hold;
}

/** When the splash has gone. */
export function splashDuration(reduceMotion = false): number {
  return splashFadeAt(reduceMotion) + SPLASH.fade;
}
