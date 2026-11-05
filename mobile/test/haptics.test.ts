import { beforeEach, describe, expect, it, vi } from "vitest";

const calls: string[] = [];
let os = "ios";
let fails = false;
const answer = (name: string) => {
  calls.push(name);
  return fails ? Promise.reject(new Error("no vibrator")) : Promise.resolve();
};

vi.mock("react-native", () => ({
  Platform: {
    get OS() {
      return os;
    },
  },
}));
vi.mock("expo-haptics", () => ({
  ImpactFeedbackStyle: { Light: "light", Medium: "medium" },
  NotificationFeedbackType: { Success: "success" },
  impactAsync: (style: string) => answer(`impact:${style}`),
  selectionAsync: () => answer("selection"),
  notificationAsync: (type: string) => answer(`notification:${type}`),
}));

const haptics = await import("../src/haptics");

describe("haptics", () => {
  beforeEach(() => {
    calls.length = 0;
    os = "ios";
    fails = false;
  });
  it("gives each moment its own vibration", () => {
    haptics.selectStart();
    haptics.selectStep();
    haptics.copied();
    haptics.dragLift();
    expect(calls).toEqual(["impact:light", "selection", "notification:success", "impact:medium"]);
  });
  it("vibrates on Android too", () => {
    os = "android";
    haptics.copied();
    expect(calls).toEqual(["notification:success"]);
  });
  it("does nothing on the web", () => {
    os = "web";
    haptics.selectStart();
    haptics.selectStep();
    haptics.copied();
    haptics.dragLift();
    expect(calls).toEqual([]);
  });
  it("a phone that cannot vibrate fails nothing", async () => {
    fails = true;
    const unhandled = vi.fn();
    process.on("unhandledRejection", unhandled);
    haptics.selectStart();
    haptics.copied();
    await new Promise((r) => setTimeout(r, 10));
    process.off("unhandledRejection", unhandled);
    expect(calls).toHaveLength(2);
    expect(unhandled).not.toHaveBeenCalled();
  });
});
