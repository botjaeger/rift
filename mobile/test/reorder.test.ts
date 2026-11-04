import { describe, expect, it } from "vitest";
import { moved, movedText, shiftOf, slotAt, stepTo } from "../src/reorder";

describe("reorder maths", () => {
  it("slotAt rounds to the nearest row and stays in the list", () => {
    expect(slotAt(2, 0, 70, 5)).toBe(2);
    expect(slotAt(2, 34, 70, 5)).toBe(2);
    expect(slotAt(2, 36, 70, 5)).toBe(3);
    expect(slotAt(2, -150, 70, 5)).toBe(0);
    expect(slotAt(2, 900, 70, 5)).toBe(4);
    expect(slotAt(2, 900, 0, 5)).toBe(2);
  });

  it("shiftOf slides the rows between the dragged row and its slot", () => {
    const shifts = (from: number, slot: number) => [0, 1, 2, 3, 4].map((i) => shiftOf(i, from, slot, 10));
    expect(shifts(1, 3)).toEqual([0, 0, -10, -10, 0]);
    expect(shifts(3, 1)).toEqual([0, 10, 10, 0, 0]);
    expect(shifts(2, 2)).toEqual([0, 0, 0, 0, 0]);
  });

  it("shifted rows are where moved puts them", () => {
    const list = ["a", "b", "c", "d", "e"];
    for (const [from, slot] of [[1, 3], [3, 1], [0, 4], [4, 0], [2, 2]] as const) {
      const after = moved(list, from, slot);
      list.forEach((item, i) => expect(after.indexOf(item)).toBe(i + shiftOf(i, from, slot, 1) + (i === from ? slot - from : 0)));
    }
  });

  it("stepTo refuses past either end", () => {
    expect(stepTo(0, -1, 3)).toBeNull();
    expect(stepTo(0, 1, 3)).toBe(1);
    expect(stepTo(2, 1, 3)).toBeNull();
    expect(stepTo(2, -1, 3)).toBe(1);
  });

  it("says where it went", () => {
    expect(movedText("build", 1, 5)).toBe("Moved build to position 2 of 5");
  });
});
