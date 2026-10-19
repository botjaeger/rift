import { describe, expect, it } from "vitest";
import { ACTION_W, OPEN_AT, settle } from "../src/ui/swipe";

describe("SwipeAction settle: where a sideways drag leaves a row", () => {
  it("opens past the threshold to the left and snaps back short of it", () => {
    expect(settle(false, -OPEN_AT)).toBe("open");
    expect(settle(false, -(OPEN_AT - 1))).toBe("closed");
    expect(settle(false, -400)).toBe("open");
    expect(settle(false, 80)).toBe("closed");
  });
  it("an open row closes when dragged back far enough, and stays open for a small drag", () => {
    expect(settle(true, 0)).toBe("open");
    expect(settle(true, ACTION_W - OPEN_AT + 1)).toBe("closed");
    expect(settle(true, ACTION_W - OPEN_AT)).toBe("open");
    expect(settle(true, -50)).toBe("open");
  });
});
