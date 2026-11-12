import { readFileSync } from "node:fs";
import { describe, expect, it, vi } from "vitest";
import { releaseName } from "../src/changelog";
import { stubColors, themeStub } from "./helpers/themeStub";
import { findAll, flat, load, render, textOf, uiDeps, type Node } from "./helpers/render";

// The Settings and Sheets boards (batch 7 of the pixel plan): every sheet's height and pinned
// actions, Settings' rows and words, While the app is closed, Appearance's swatch grid, What's
// new. Run as the app's own modules where they render bare; the rest by their source. Pixels
// stay device checks (the iOS 27 sim and Android API 35).

const src = (p: string) => readFileSync(new URL(`../src/${p}`, import.meta.url), "utf8");
const style = (n: Node | undefined) => flat(n?.props.style);
// ui/sheet.ts imports react-native: run it with the stubs
const { SHEET_HEIGHTS, SHEET_TOP_ROOM, sheetHeight, sheetUnder } = load("ui/sheet.ts", uiDeps("ios")) as {
  SHEET_HEIGHTS: Record<string, number>;
  SHEET_TOP_ROOM: number;
  sheetHeight: (board: string, height: number, top: number, under?: number) => number;
  sheetUnder: (os: string, bottom: number) => number;
};

describe("sheet heights: the boards', to the screen's end", () => {
  it("each board sheet has its board's height on the 402 × 874 phone", () => {
    expect(SHEET_HEIGHTS).toEqual({ attach: 484, folder: 645, block: 690, newTab: 645, newPane: 507, confirm: 381, whatsNew: 484, connection: 541 });
    for (const board of Object.keys(SHEET_HEIGHTS)) expect(sheetHeight(board, 874, 62)).toBe(SHEET_HEIGHTS[board]);
  });
  it("a short screen keeps the sheet clear of the status bar", () => {
    // an iPhone SE: 667 tall, 20 pt status bar
    expect(sheetHeight("block", 667, 20)).toBe(667 - 20 - SHEET_TOP_ROOM);
    expect(sheetHeight("confirm", 667, 20)).toBe(381);
    expect(sheetHeight("block", 100, 80)).toBe(0);
  });
  it("on iOS the content leaves the room the system adds under it (the home indicator's inset)", () => {
    expect(sheetUnder("ios", 34)).toBe(34);
    expect(sheetUnder("android", 48)).toBe(0);
    expect(sheetHeight("connection", 874, 62, sheetUnder("ios", 34))).toBe(541 - 34);
  });
  it("every board sheet sizes to its content and draws no system grabber", () => {
    const layout = src("app/_layout.tsx");
    for (const name of ["new-tab", "new-pane", "attach", "whats-new", "block", "connection", "confirm"]) {
      const at = layout.indexOf(`name="${name}"`);
      const options = layout.slice(at, layout.indexOf("/>", at));
      expect(options, name).toContain('sheetAllowedDetents: "fitToContents"');
      expect(options, name).toContain("sheetGrabberVisible: false");
    }
    // Change folder too (a chip menu), not the long Changes list
    expect(layout).toMatch(/\["dirs", "pr", "python", "php", "docker", "elapsed"\]\.includes/);
  });
  it("every board sheet is a SheetFrame of its board", () => {
    const boards: Record<string, string> = {
      "app/attach.tsx": "attach",
      "app/new-tab.tsx": "newTab",
      "app/new-pane.tsx": "newPane",
      "app/block.tsx": "block",
      "app/confirm.tsx": "confirm",
      "app/whats-new.tsx": "whatsNew",
      "app/chips.tsx": "folder",
      "ui/ConnectionSheet.tsx": "connection",
    };
    for (const [file, board] of Object.entries(boards)) expect(src(file), file).toContain(`board="${board}"`);
  });
});

describe("SheetFrame: the header, the content scrolling, the actions pinned to the foot", () => {
  const frame = (platform: "ios" | "android", footer: unknown) => {
    const deps = uiDeps(platform, {
      "expo-router": { useRouter: () => ({ back: vi.fn(), replace: vi.fn(), canGoBack: () => true }) },
      "@/runtime": { useLock: () => ({ locked: false, covered: false }) },
    });
    const { SheetFrame } = load("ui/SheetFrame.tsx", deps) as { SheetFrame: (p: object) => unknown };
    return render({ type: SheetFrame, props: { board: "block", title: "npm test", onClose: vi.fn(), footer, children: [{ type: "Text", props: { children: ["body"] } }] } }) as Node;
  };
  const parts = (root: Node) => (root.props.children as unknown[]).flat().filter(Boolean) as Node[];

  it("is never flattened and has a height of its own, not a growing root (zero-height sheet on iOS 27)", () => {
    const root = frame("ios", { type: "Button", props: { label: "Run again", children: [] } });
    expect(root.props.collapsable).toBe(false);
    // the render helper's insets: 47 over, 34 under
    expect(style(root)).toMatchObject({ height: 690 - 34, backgroundColor: stubColors.surface });
    expect(style(root).flex).toBeUndefined();
    expect(style(root).flexGrow).toBeUndefined();
  });
  it("header, scrolling content, then the foot with the actions", () => {
    const root = frame("ios", { type: "Button", props: { label: "Run again", children: [] } });
    const [head, body, foot] = parts(root);
    expect(findAll(head, (n) => n.props.accessibilityRole === "header").map(textOf)).toEqual(["npm test"]);
    expect(body!.type).toBe("ScrollView");
    expect(style(body)).toMatchObject({ flex: 1 });
    expect(findAll(foot, (n) => n.type === "Button" && n.props.label === "Run again")).toHaveLength(1);
    // iOS: the system's 34 pt under the content already passes the board's 24
    expect(style(foot)).toMatchObject({ paddingTop: 12, paddingHorizontal: 16, paddingBottom: 0 });
  });
  it("Android: the foot is 24 pt over the navigation bar", () => {
    const [, , foot] = parts(frame("android", { type: "Button", props: { label: "Done", children: [] } }));
    expect(style(foot)).toMatchObject({ paddingBottom: 24 + 34 });
  });
  it("no actions: no foot", () => {
    expect(parts(frame("ios", null))).toHaveLength(2);
  });
  it("Run this?, Block, What's new and Connection pin their actions; New tab and New pane their confirmation", () => {
    for (const f of ["app/block.tsx", "app/confirm.tsx", "app/whats-new.tsx", "ui/ConnectionSheet.tsx"]) expect(src(f), f).toMatch(/footer=\{/);
    for (const f of ["app/new-tab.tsx", "app/new-pane.tsx"]) {
      expect(src(f), f).toContain("footer = <ConfirmProgramButtons");
      expect(src(f), f).toContain("footer={footer}");
    }
    // equal halves 8 apart
    expect(src("ui/SheetFrame.tsx")).toContain('buttons: { flexDirection: "row", gap: 8 }');
    expect(src("app/confirm.tsx").match(/style=\{styles\.flex\}/g)).toHaveLength(2);
    expect(src("app/block.tsx").match(/style=\{styles\.flex\}/g)).toHaveLength(3);
  });
});

describe("the sheets' contents", () => {
  it("New tab: Folder is a row with the path cut at its start and a ›, which opens Change folder for the tab beside", () => {
    const tab = src("app/new-tab.tsx");
    expect(tab).toMatch(/<NavRow\s+first\s+path\s+label="Folder"/);
    expect(tab).toContain('router.push({ pathname: "/chips", params: { key, pane: String(base), menu: "dirs" } })');
    expect(tab).not.toContain("ValueRow");
  });
  it("New tab and New pane: no explaining footnote (0.8.0 extra)", () => {
    expect(src("app/new-tab.tsx")).not.toContain("The new tab opens beside this one");
    expect(src("app/new-pane.tsx")).not.toContain("The new pane starts in");
  });
  it("Run this?: no branch in the where line; the ❯ stays", () => {
    const c = src("app/confirm.tsx");
    expect(c).toContain("const where = loc ? `In ${place} on ${hostName}.` : \"This pane has closed.\";");
    expect(c).toContain("❯ ");
    expect(c).toContain("padding: 12,");
  });
  it("Block: Inter title, grey subtitle with the duration (the folder on its own line), uppercase caption, 150 pt of output growing into the sheet when longer", () => {
    const b = src("app/block.tsx");
    expect(b).not.toContain("titleMono");
    expect(b).not.toContain("subtitleColor");
    expect(b).toContain("spacedDuration(block!.duration_ms)");
    // the subtitle no longer repeats the folder, cut at its end
    expect(b).not.toMatch(/spacedDuration\(block!\.duration_ms\),\s+place\[0\]/);
    expect(b).toContain('textTransform: "uppercase"');
    expect(b).toContain("outBox: { height: 172,");
    expect(b).toContain("outGrow: { height: undefined, flexGrow: 1, flexBasis: 0, minHeight: 172 }");
    expect(b).toContain("size={12} lineHeight={18}");
    expect(b).toContain("SelectableText text={`$ ${command");
  });
  it("Change folder: settings rows with a 28 pt grey tile and a ›; the field and its line 8 apart", () => {
    const c = src("app/chips.tsx");
    expect(c).toMatch(/<NavRow\s+key=\{d\}/);
    expect(c).toContain('tile: { width: 28, height: 28, borderRadius: 8, backgroundColor: colors.faint');
    expect(c).toContain("fieldGroup: { gap: 8 }");
  });
});

describe("What's new", () => {
  const notes = () => load("ui/ReleaseNotes.tsx", uiDeps("ios", { "@/changelog": { releaseName, releaseKey: (v: string, u: number) => `${v}-u${u}` } })) as {
    releaseLabel: (r: object) => string;
    releaseDay: (d: string) => string;
    ReleaseList: (p: object) => unknown;
  };
  const release = { version: "0.8.0", update: 2, date: "2026-11-07", summary: "Themes.", groups: [{ title: "New", items: [{ title: "Themes", text: "the app takes your computer's theme." }] }] };
  it("names a release as the board: 0.8.0 · update 2 · 7 Nov", () => {
    expect(notes().releaseLabel(release)).toBe("0.8.0 · update 2 · 7 Nov");
    expect(notes().releaseDay("2026-01-31")).toBe("31 Jan");
    expect(notes().releaseDay("soon")).toBe("soon");
  });
  it("items 15/22 in the body colour, the dash too, led by the group's dot; no hanging indent", () => {
    const tree = render({ type: notes().ReleaseList, props: { releases: [release], named: true, children: [] } });
    const item = findAll(tree, (n) => n.type === "Text" && n.props.accessibilityLabel === "Themes — the app takes your computer's theme.")[0]!;
    expect(style(item)).toMatchObject({ fontSize: 15, lineHeight: 22, color: stubColors.text });
    expect(textOf(item)).toBe("Themes — the app takes your computer's theme.");
    const dot = findAll(item, (n) => style(n).width === 6)[0]!;
    expect(style(dot)).toMatchObject({ height: 6, borderRadius: 3, backgroundColor: stubColors.accent });
    // the page's heading: the name, its day muted
    const name = findAll(tree, (n) => n.props.accessibilityRole === "header" && style(n).fontSize === 17)[0]!;
    expect(textOf(name)).toBe("0.8.0 · update 2 7 Nov");
  });
  it("the sheet: a straight apostrophe, one Done pinned, no All changes or lead line", () => {
    const w = src("app/whats-new.tsx");
    expect(w).toContain('title="What\'s new"');
    expect(w).toContain('footer={<Button label="Done" variant="primary"');
    expect(w).not.toContain("All changes");
    expect(w).not.toContain("Rift was updated");
    expect(src("app/changelog.tsx")).toContain('title="What\'s new"');
  });
});

describe("Settings (the Settings board)", () => {
  const s = src("app/settings.tsx");
  it("words: App lock's Face ID, no detail under Commands I ran from here, no Notifications footer", () => {
    expect(s).toContain("<SwitchRow label=\"App lock\" detail={bio}");
    expect(s).not.toContain("When a command you ran from this phone finishes");
    expect(s).not.toContain("Nothing notifies for the pane on screen");
    expect(s).toContain('detail="Shows the command first. ^C always sends at once."');
    expect(s).toContain('detail="Off: wide output scrolls sideways"');
    expect(s).toContain('detail="Off: only under a program"');
    expect(s).toContain('<NavRow label="What\'s new"');
  });
  it("Other commands is a row with its value; the choices open under it (no segmented control)", () => {
    expect(s).toContain("<OtherCommands value={st.notifyPolicy}");
    expect(s).not.toContain("<Segmented");
    expect(s).toMatch(/label="Other commands"\s+value=\{POLICIES\.find/);
  });
  it("Keys: the keys as the row's value (Inter 15), not a mono line", () => {
    expect(s).toMatch(/label="Keys"\s+value=\{st\.controlKeys/);
  });
  it("Name: the value Inter 15 hugging the name, and a ›", () => {
    expect(s).toContain("fontSize: 15,\n    textAlign: \"right\"");
    expect(s).toContain("width: Math.min(NAME_MAX, Math.ceil(width) + 4)");
    expect(s).toMatch(/<Icon name="forward" size=\{CHEVRON\}/);
  });
  it("While the app is closed: the caption over the card", () => {
    expect(src("app/settings-ntfy.tsx")).toContain('<Group title="Alerts through ntfy" footer={footer}>');
  });
});

describe("Appearance (the Themes board)", () => {
  const set = vi.fn();
  const deps = uiDeps("ios", {
    "@/appearance": { computerDetail: () => "studio-mac uses Dracula now" },
    "@/runtime": { fleet: { updateSettings: set }, useApp: () => ({ settings: { appearance: { mode: "computer" } }, hosts: [], lastView: null, lastTheme: null }) },
    "@/themes.data": { BUILTIN_THEMES: Array.from({ length: 13 }, (_, i) => ({ name: `Theme ${i}`, dark: true })) },
    "@/ui/Icon": { Icon: "Icon" },
    "@/ui/Rows": { CardBox: "CardBox", CHEVRON: 8, ChoiceRow: "ChoiceRow", Group: "Group", SettingsPage: "SettingsPage" },
    "@/ui/SaveError": { SaveError: "SaveError" },
    "@/ui/theme": themeStub,
  });
  const { default: Appearance } = load("app/appearance.tsx", deps) as { default: () => unknown };
  const tree = render({ type: Appearance, props: { children: [] } });

  it("the board's words; › on Choose a theme; no per-row Dark/Light lines, no 'stays on this phone' footer", () => {
    const rows = findAll(tree, (n) => n.type === "ChoiceRow");
    expect(rows.map((r) => [r.props.label, r.props.detail])).toEqual([
      ["Same as the computer", "studio-mac uses Dracula now"],
      ["Match this phone", "Light Default by day, Dark Default at night"],
      ["Choose a theme", "The same 13 themes as the terminal"],
    ]);
    expect(rows[2]!.props.trailing).toBeTruthy();
    for (const r of rows) expect(r.props.minHeight).toBe(76);
    expect(textOf(tree)).not.toContain("stays on this phone");
    expect(textOf(tree)).toContain("With several computers paired");
  });
  it("themes as a five-column grid of 40 pt swatches, each a choice", () => {
    const cells = findAll(tree, (n) => style(n).width === "20%");
    expect(cells).toHaveLength(13);
    const swatches = findAll(tree, (n) => n.props.accessibilityRole === "radio");
    expect(swatches).toHaveLength(13);
    expect(style(swatches[0])).toMatchObject({ height: 40, borderRadius: 10, borderWidth: 1, padding: 4, alignItems: "flex-end", justifyContent: "flex-end" });
    (swatches[4]!.props.onPress as () => void)();
    expect(set).toHaveBeenCalledWith({ appearance: { mode: "theme", theme: "Theme 4" } });
  });
  it("✓ weight 600", () => {
    expect(src("ui/Rows.tsx")).toContain("check: { fontFamily: ui(600), color: colors.accent, fontSize: 16");
  });
});
