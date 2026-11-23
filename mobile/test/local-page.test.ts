import { describe, expect, it, vi } from "vitest";
import { findAll, load, type Node, render, uiDeps } from "./helpers/render";

// iPhone, 2026-11-23: "Open a local page" closed its sheet, then opened the browser; iOS presented
// the browser over the sheet still dismissing, and it closed with it. The sheet now stays under
// the browser and closes once the browser has.
describe("the Open a local page sheet", () => {
  function sheet() {
    let closeBrowser = () => {};
    const openPort = vi.fn(() => new Promise<void>((resolve) => (closeBrowser = resolve)));
    const back = vi.fn();
    const host = { record: { key: "k", name: "Mac" }, conn: { status: "open" } };
    const deps = uiDeps("ios", {
      "expo-router": { useLocalSearchParams: () => ({ key: "k" }), useRouter: () => ({ back }) },
      "@/fleet": { canOperate: () => true, hasFeature: () => true },
      "@/runtime": { useApp: () => ({ hosts: [host], pane: { host: "k", state: { blocks: [] }, screen: { lines: [] } } }) },
      "@/tunnel": { tunnelAvailable: true },
      "@/tunnel/open": { openPort },
      "@/tunnel/url": { localLinksIn: () => [{ port: 3000, path: "/", url: "http://localhost:3000/" }] },
      "@/ui/Button": { Button: "Button" },
      "@/ui/Field": { Field: "Field" },
      "@/ui/NewSheet": { Footnote: "Footnote", SheetNote: "SheetNote" },
      "@/ui/Rows": { CardBox: "CardBox", Group: "Group", NavRow: "NavRow" },
      "@/ui/SheetFrame": { SheetFrame: "SheetFrame" },
    });
    const { default: LocalPage } = load("app/local-page.tsx", deps) as { default: () => unknown };
    const tree = render({ type: LocalPage, props: { children: [] } });
    const row = findAll(tree, (n: Node) => n.type === "NavRow")[0]!;
    return { row, openPort, back, closeBrowser: () => closeBrowser() };
  }

  it("stays open under the browser and closes once the browser has", async () => {
    const s = sheet();
    (s.row.props.onPress as () => void)();
    expect(s.openPort).toHaveBeenCalledWith("k", 3000, "/");
    await Promise.resolve();
    expect(s.back).not.toHaveBeenCalled();
    s.closeBrowser();
    await new Promise((r) => setTimeout(r, 0));
    expect(s.back).toHaveBeenCalledTimes(1);
  });
});
