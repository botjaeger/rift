import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

// Defects found walking the redesign on the iOS simulator and the Android emulator (2026-11-06).
const src = (path: string) => readFileSync(new URL(`../${path}`, import.meta.url), "utf8");

describe("the redesign walk's fixes", () => {
  it("shows a search's or filter's results from their top on the Tabs page", () => {
    const tabs = src("src/app/tabs.tsx");
    expect(tabs).toMatch(/scrollToOffset\(\{ offset: 0, animated: false \}\);\n {2}\}, \[filter, query\]\);/);
    expect(tabs).toContain("ref={list}");
  });
  it("keeps Search keys above the keyboard on the Keys page", () => {
    const keys = src("src/app/control-keys.tsx");
    expect(keys).toContain("automaticallyAdjustKeyboardInsets");
    expect(keys).toContain('<KeyboardAvoidingView style={styles.fill} behavior="padding" enabled={Platform.OS === "android"}>');
  });
  it("leaves no caret in the output when Back ends an Android selection", () => {
    expect(src("src/ui/SelectableText.tsx")).toMatch(/if \(sel\.start === sel\.end\) \{[^}]*if \(Platform\.OS === "android"\) field\.current\?\.blur\(\);/);
  });
  it("lands on Portals after pairing, not the old host page", () => {
    const pair = src("src/app/pair.tsx");
    expect(pair).toContain('router.dismissTo("/");');
    expect(pair).not.toContain('pathname: "/host/[key]"');
  });
  it("says when the keychain could not be read, with Try again, instead of a blank Portals", () => {
    const portals = src("src/app/index.tsx");
    expect(portals).toContain("{s.loadError ? (");
    expect(portals).toContain('<Button label="Try again" variant="primary" onPress={() => void fleet.retryInit()} />');
  });
  it("folds a question with the standard sheet header's ✕", () => {
    const card = src("src/ui/AskCard.tsx");
    expect(card).toContain('<SheetHeader title={`${who} asks`}');
    expect(card).toContain('closeLabel="Fold the question" onClose={() => setMin(true)}');
  });
});
