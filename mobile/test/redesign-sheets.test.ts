import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const src = (path: string) => readFileSync(new URL(`../${path}`, import.meta.url), "utf8");

// The wiring of the New tab, New pane and Attach sheets and the 44 pt key row, read from the
// screens' own source (rendering and navigation are device checks, rows 5.1 to 5.8).
describe("the redesign's sheets are wired", () => {
  const layout = src("src/app/_layout.tsx");
  it("registers each as a form sheet, as the other sheets are", () => {
    for (const name of ["new-tab", "new-pane", "attach"]) {
      const at = layout.indexOf(`name="${name}"`);
      expect(at, name).toBeGreaterThan(0);
      expect(layout.slice(at, at + 400), name).toContain('presentation: "formSheet"');
    }
  });
  it("the Tabs sheet, the Panes sheet and the Portals card open the new sheets", () => {
    expect(src("src/app/tabs.tsx")).toContain('pathname: "/new-tab"');
    expect(src("src/app/tabs.tsx")).not.toContain("openNewTab");
    expect(src("src/app/panes.tsx")).toContain('pathname: "/new-pane"');
    expect(src("src/app/index.tsx")).toContain('pathname: "/new-tab", params: { key: host.record.key, from: "home" }');
    expect(src("src/app/index.tsx")).not.toContain("openNewTab");
  });
  it("view-only devices get no New tab or New pane control", () => {
    const tabs = src("src/app/tabs.tsx");
    expect(tabs).toContain("const operate = !!host && canOperate(host);");
    expect(tabs).toContain("right={operate ? <IconButton");
    expect(src("src/app/panes.tsx")).toContain("host && loc && canOperate(host) ? (");
  });
  it("there is no split direction to choose", () => {
    for (const f of ["src/app/new-pane.tsx", "src/app/panes.tsx", "src/ui/NewSheet.tsx", "src/ui/useLaunch.ts"]) expect(src(f), f).not.toMatch(/\bsplit|"(horizontal|vertical)"/i);
  });
  it("a program is typed through run, which Ask before running and Face ID govern", () => {
    const hook = src("src/ui/useLaunch.ts");
    expect(hook).toContain("launchAsks(p, s.settings).auth");
    expect(hook).toContain("fleet.run(hostKey, target, command");
    expect(src("src/launch.ts")).toContain("confirm: settings.askBeforeRun");
  });
  it("Rift closed goes through start (Face ID) with a shell only", () => {
    expect(src("src/app/new-tab.tsx")).toContain("<StartTab host={host} onOpened={launch.finish} />");
    expect(src("src/ui/newTab.ts")).toContain("fleet.startShell(key)");
  });
});

describe("the message box and the Attach sheet", () => {
  const composer = src("src/ui/Composer.tsx");
  it("+ opens Attach; the Paste and Photo icons are gone from the field", () => {
    expect(composer).toContain('accessibilityLabel="Attach"');
    expect(composer).toContain('pathname: "/attach"');
    expect(composer).not.toContain("Attach a photo");
    expect(composer).not.toContain("SystemPasteButton");
    expect(composer).not.toContain("ImagePicker");
  });
  it("iOS's Paste from clipboard row is the system paste control, not a clipboard read", () => {
    const attach = src("src/app/attach.tsx");
    const at = attach.indexOf("hasSystemPasteButton ? (");
    expect(at).toBeGreaterThan(0);
    const native = attach.slice(at, attach.indexOf(") : (", at));
    expect(native).toContain("<SystemPasteButton");
    expect(native).toContain("labelled");
    expect(native).not.toContain("Clipboard.");
    expect(native).not.toContain("readClipboard");
    expect(src("modules/rift-keys/ios/RiftPasteButton.swift")).toContain("UIPasteControl(configuration: config)");
  });
  it("Attach has the four routes and returns through the pane's own store, never a draft write", () => {
    const attach = src("src/app/attach.tsx");
    for (const t of ["Paste from clipboard", "Photo library", "Take a photo", "File"]) expect(attach).toContain(`"${t}"`);
    expect(attach).toContain("pending.add(box, a.item)");
    expect(attach).toContain("pending.giveText(box, a.text)");
    expect(attach).not.toContain("setDraft");
    expect(attach).not.toContain("clearDraft");
  });
  it("the camera's usage text is in the app config and says what it is for", () => {
    const config = JSON.parse(src("app.json")) as { expo: { plugins: unknown[] } };
    const plugin = (name: string) => (config.expo.plugins.find((p) => Array.isArray(p) && p[0] === name) as [string, Record<string, unknown>])[1];
    expect(plugin("expo-camera").cameraPermission).toMatch(/take a photo/);
    // the image picker must not block the camera permission the scanner needs
    expect(plugin("expo-image-picker").cameraPermission).toBe(plugin("expo-camera").cameraPermission);
  });
});

describe("the key row", () => {
  const row = src("src/ui/ControlRow.tsx");
  const keys = src("src/controlKeys.ts");
  const paste = src("modules/rift-keys/ios/RiftPasteButton.swift");
  it("draws the Keys redesign board's 41 pt keys with 8 pt corners beside the 132 x 88 arrow pad", () => {
    expect(row).toContain("export const KEY_H = 41;");
    expect(row).toContain("export const PAD_W = 132;");
    expect(row).toContain("export const BLOCK_H = 88;");
    expect(row).toMatch(/key: \{\s*borderRadius: 8,/);
  });
  it("draws every symbol from the one icon set, keys in mono, words in Inter, colour only for a state", () => {
    // ⌃ ⌥ ⇧ are icons now, not font glyphs; ⇧Tab leads with the shift icon
    expect(keys).toMatch(/id: "ctrl",[^\n]*icon: "control"/);
    expect(keys).toMatch(/id: "alt",[^\n]*icon: "option"/);
    expect(keys).toMatch(/id: "shift",[^\n]*icon: "shift"/);
    expect(keys).toMatch(/id: "shift\+tab",[^\n]*lead: "shift"/);
    expect(row).toContain('<Icon name={def.icon} size={18} weight="medium" color={fg} />');
    expect(row).toContain("label: { fontFamily: monoMedium, fontSize: 13,");
    expect(row).toContain("word: { fontFamily: ui(500), fontSize: 12 }");
    expect(row).toContain("keyActive: { backgroundColor: colors.accent, borderColor: colors.accent }");
    // the system Paste takes the key's ground instead of drawing a black circle: the theme's,
    // handed in by the key row (Dark Default's only when none is)
    expect(paste).toContain("config.cornerStyle = .fixed");
    expect(paste).toContain("config.baseBackgroundColor = ground ?? keyGround");
    expect(row).toContain("ground={colors.surface} ink={colors.text}");
    // the Attach sheet's Paste row: the list's ground and the accent of its sibling rows' icons
    expect(paste).toContain("config.baseBackgroundColor = ground ?? rowGround");
    expect(paste).toContain("config.baseForegroundColor = ink ?? accent");
    expect(src("src/app/attach.tsx")).toMatch(/<SystemPasteButton\s+labelled\s+ground=\{colors\.bg\}\s+ink=\{colors\.accent\}/);
    expect(src("modules/rift-keys/ios/RiftPasteModule.swift")).toContain('Prop("ground")');
  });
  it("never scrolls: two rows of keys, so the screen above keeps its size", () => {
    expect(row).not.toContain("ScrollView");
    expect(row).toContain("block: { flexDirection: \"row\", gap: 10, height: BLOCK_H }");
  });
});
