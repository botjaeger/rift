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
    expect(tabs).toContain("{operate ? (");
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
  it("draws the Keys board's 52 x 44 pt keys, each its own 44 pt target", () => {
    expect(row).toContain("export const KEY_W = 52;");
    expect(row).toMatch(/key: \{\s*width: KEY_W,\s*height: TOUCH,\s*borderRadius: 12,/);
    expect(row).toContain("slot: { minWidth: TOUCH, minHeight: TOUCH,");
    expect(row).toContain("keyWord: { width: undefined, minWidth: KEY_W,");
  });
  it("draws every symbol from the one icon set, words in mono, colour only for a state", () => {
    // ⌃ ⌥ ⇧ are icons now, not font glyphs; ⇧Tab leads with the shift icon
    expect(keys).toMatch(/id: "ctrl",[^\n]*icon: "control"/);
    expect(keys).toMatch(/id: "alt",[^\n]*icon: "option"/);
    expect(keys).toMatch(/id: "shift",[^\n]*icon: "shift"/);
    expect(keys).toMatch(/id: "shift\+tab",[^\n]*lead: "shift"/);
    expect(row).toContain('<Icon name={def.icon} size={18} weight="medium" color={fg} />');
    expect(row).toContain("label: { fontFamily: monoMedium, fontSize: 13,");
    expect(row).toContain("keyActive: { backgroundColor: tints.accentBadge, borderColor: colors.accent }");
    // the system Paste takes the key's ground instead of drawing a black circle
    expect(paste).toContain("config.cornerStyle = .fixed");
    expect(paste).toContain("config.baseBackgroundColor = keyGround");
    // the Attach sheet's Paste row: the list's ground and the accent of its sibling rows' icons
    expect(paste).toContain("config.baseBackgroundColor = rowGround");
    expect(paste).toContain("config.baseForegroundColor = accent");
  });
  it("scrolls sideways only, without moving the screen", () => {
    expect(row).toContain("directionalLockEnabled");
    expect(row).toContain("bounces={false}");
    expect(row).toContain('overScrollMode="never"');
  });
});
