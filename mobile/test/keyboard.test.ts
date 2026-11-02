import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  InputBatcher,
  MIN_SEND_INTERVAL_MS,
  applySticky,
  composedText,
  keyEventToAction,
  mergeActions,
  splitText,
  stickyKey,
  terminalInput,
  textToActions,
  type InputAction,
  type KeyEventLike,
} from "../src/keyboard";
import { isClientMsg } from "../src/protocol/types";

const key = (k: string, mods: Partial<KeyEventLike> = {}): KeyEventLike => ({
  key: k,
  ctrl: false,
  alt: false,
  shift: false,
  meta: false,
  ...mods,
});
const act = (k: string, mods: Partial<KeyEventLike> = {}) => keyEventToAction(key(k, mods));

/** Every action the mapper makes must be a message the gateway takes. */
function sendable(a: InputAction): boolean {
  return "keys" in a
    ? isClientMsg({ t: "keys", id: "k1", pane: 1, keys: a.keys })
    : isClientMsg({ t: "text", id: "x1", pane: 1, text: a.text });
}

describe("keyEventToAction", () => {
  it.each([
    ["Escape", "esc"],
    ["Tab", "tab"],
    ["Enter", "enter"],
    ["Backspace", "backspace"],
    ["Delete", "delete"],
    ["Insert", "insert"],
    ["ArrowUp", "up"],
    ["ArrowDown", "down"],
    ["ArrowLeft", "left"],
    ["ArrowRight", "right"],
    ["Home", "home"],
    ["End", "end"],
    ["PageUp", "pageup"],
    ["PageDown", "pagedown"],
    ["F1", "f1"],
    ["F12", "f12"],
  ])("%s is the key %s", (k, name) => {
    expect(act(k)).toEqual({ keys: [name] });
  });

  it("named keys carry their modifiers in protocol order", () => {
    expect(act("Tab", { shift: true })).toEqual({ keys: ["shift+tab"] });
    expect(act("ArrowLeft", { ctrl: true })).toEqual({ keys: ["ctrl+left"] });
    expect(act("ArrowLeft", { alt: true, ctrl: true, shift: true })).toEqual({ keys: ["ctrl+alt+shift+left"] });
    expect(act("F5", { shift: true })).toEqual({ keys: ["shift+f5"] });
    expect(act("Delete", { ctrl: true, alt: true })).toEqual({ keys: ["ctrl+alt+delete"] });
  });

  it("ctrl / alt with a letter is a key; shift is dropped, case ignored", () => {
    expect(act("c", { ctrl: true })).toEqual({ keys: ["ctrl+c"] });
    expect(act("C", { ctrl: true, shift: true })).toEqual({ keys: ["ctrl+c"] });
    expect(act("b", { alt: true })).toEqual({ keys: ["alt+b"] });
    expect(act("x", { ctrl: true, alt: true })).toEqual({ keys: ["ctrl+alt+x"] });
    expect(act(" ", { ctrl: true })).toEqual({ keys: ["ctrl+space"] });
    expect(act("@", { ctrl: true, shift: true })).toEqual({ keys: ["ctrl+space"] });
    expect(act("2", { ctrl: true })).toEqual({ keys: ["ctrl+space"] });
    expect(act("[", { ctrl: true })).toEqual({ keys: ["esc"] });
  });

  it("plain characters are text, shifted or not, in any script", () => {
    expect(act("a")).toEqual({ text: "a" });
    expect(act("A", { shift: true })).toEqual({ text: "A" });
    expect(act(":")).toEqual({ text: ":" });
    expect(act(" ")).toEqual({ text: " " });
    expect(act("é")).toEqual({ text: "é" });
    expect(act("😀")).toEqual({ text: "😀" });
  });

  it("Cmd / Meta combinations stay with the system", () => {
    expect(act("c", { meta: true })).toBeNull();
    expect(act("ArrowUp", { meta: true })).toBeNull();
    expect(act("v", { meta: true, ctrl: true })).toBeNull();
  });

  it("lone modifiers, dead keys and unnamed combinations send nothing", () => {
    for (const k of ["Shift", "Control", "Alt", "Meta", "CapsLock", "Dead", "Unidentified", "Process", "F13"]) {
      expect(act(k)).toBeNull();
    }
    expect(act("1", { ctrl: true })).toBeNull();
    expect(act(".", { alt: true })).toBeNull();
    expect(act("[", { alt: true })).toBeNull();
  });

  it("AltGr / Option characters are text, not alt+key", () => {
    // Android AltGr (right Alt): the key's own character, and the composed one
    expect(act("q", { alt: true, char: "@", altGraph: true })).toEqual({ text: "@" });
    // web on Windows: AltGr is ctrl+alt with the AltGraph state; the key is already composed
    expect(act("@", { ctrl: true, alt: true, char: "@", altGraph: true })).toEqual({ text: "@" });
    expect(act("{", { ctrl: true, alt: true, char: "{", altGraph: true })).toEqual({ text: "{" });
    // ctrl+alt with a character the protocol has no key for: the composed character
    expect(act("{", { ctrl: true, alt: true, char: "{" })).toEqual({ text: "{" });
    // iPadOS / macOS Option+2 (charactersIgnoringModifiers "2", characters "™")
    expect(act("2", { alt: true, char: "™" })).toEqual({ text: "™" });
    expect(act("l", { alt: true, char: "|" })).toEqual({ text: "|" });
    expect(composedText(key("2", { alt: true, char: "™" }))).toBe("™");
  });

  it("alt+letter stays a key when the layout composes nothing else", () => {
    expect(act("b", { alt: true, char: "b" })).toEqual({ keys: ["alt+b"] }); // composed == base
    expect(act("b", { alt: true, char: "" })).toEqual({ keys: ["alt+b"] }); // none
    expect(act("b", { alt: true })).toEqual({ keys: ["alt+b"] });
    expect(act("b", { alt: true, char: "\u001b" })).toEqual({ keys: ["alt+b"] }); // not printable
    // with ctrl held (and no AltGr) a composed character does not make it text
    expect(act("c", { ctrl: true, char: "\u0003" })).toEqual({ keys: ["ctrl+c"] });
    expect(act("x", { ctrl: true, alt: true, char: "≈" })).toEqual({ keys: ["ctrl+alt+x"] });
    expect(act("@", { ctrl: true, shift: true, char: "@" })).toEqual({ keys: ["ctrl+space"] });
    expect(act("[", { ctrl: true, char: "\u001b" })).toEqual({ keys: ["esc"] });
    expect(act(".", { alt: true, char: "\u0085" })).toBeNull(); // a C1 control is not text
  });

  it("everything it makes is sendable", () => {
    const events: KeyEventLike[] = [];
    for (const k of ["a", "Z", " ", "Escape", "Tab", "F7", "ArrowUp", "[", "2", "é"])
      for (const ctrl of [false, true])
        for (const alt of [false, true])
          for (const shift of [false, true]) events.push(key(k, { ctrl, alt, shift }));
    for (const e of events) {
      const a = keyEventToAction(e);
      if (a) expect(sendable(a)).toBe(true);
    }
  });
});

describe("textToActions", () => {
  it("printable runs are text", () => {
    expect(textToActions("ls -la")).toEqual([{ text: "ls -la" }]);
    expect(textToActions("")).toEqual([]);
  });

  it("line breaks, tabs, backspace and escape are keys, in order", () => {
    expect(textToActions(":wq\n")).toEqual([{ text: ":wq" }, { keys: ["enter"] }]);
    expect(textToActions("a\tb")).toEqual([{ text: "a" }, { keys: ["tab"] }, { text: "b" }]);
    expect(textToActions("\b\u007f")).toEqual([{ keys: ["backspace"] }, { keys: ["backspace"] }]);
    expect(textToActions("\u001b:q")).toEqual([{ keys: ["esc"] }, { text: ":q" }]);
  });

  it("\\r\\n is one Enter; a lone \\r is Enter too", () => {
    expect(textToActions("a\r\nb")).toEqual([{ text: "a" }, { keys: ["enter"] }, { text: "b" }]);
    expect(textToActions("\r")).toEqual([{ keys: ["enter"] }]);
  });

  it("other C0 controls are ctrl+letter; C1 controls are dropped", () => {
    expect(textToActions("\u0003")).toEqual([{ keys: ["ctrl+c"] }]);
    expect(textToActions("\u001a")).toEqual([{ keys: ["ctrl+z"] }]);
    expect(textToActions("\u0000")).toEqual([{ keys: ["ctrl+space"] }]);
    expect(textToActions("a\u0085b")).toEqual([{ text: "a" }, { text: "b" }]);
    expect(textToActions("\u001c")).toEqual([]);
  });

  it("never puts a control character into text", () => {
    let all = "";
    for (let i = 0; i < 0xa0; i++) all += String.fromCharCode(i);
    for (const a of textToActions(all)) expect(sendable(a)).toBe(true);
  });
});

describe("sticky modifiers", () => {
  it("apply to the next key from the row", () => {
    expect(applySticky("up", { ctrl: true, alt: false })).toBe("ctrl+up");
    expect(applySticky("shift+tab", { ctrl: false, alt: true })).toBe("alt+shift+tab");
    expect(applySticky("ctrl+c", { ctrl: false, alt: true })).toBe("ctrl+alt+c");
    expect(applySticky("ctrl+c", { ctrl: true, alt: false })).toBe("ctrl+c");
    expect(applySticky("esc", { ctrl: false, alt: false })).toBe("esc");
    expect(applySticky("tab", { ctrl: false, alt: false, shift: true })).toBe("shift+tab");
    expect(applySticky("up", { ctrl: true, alt: false, shift: true })).toBe("ctrl+shift+up");
  });

  it("and to the next character typed", () => {
    expect(stickyKey("b", { ctrl: false, alt: true })).toBe("alt+b");
    expect(stickyKey("R", { ctrl: true, alt: false })).toBe("ctrl+r");
    expect(stickyKey("x", { ctrl: true, alt: true })).toBe("ctrl+alt+x");
    expect(stickyKey(" ", { ctrl: true, alt: false })).toBe("ctrl+space");
    expect(stickyKey("b", { ctrl: false, alt: false })).toBeNull();
  });

  it("sticky shift then Return is a new line, not a submit", () => {
    expect(stickyKey("\n", { ctrl: false, alt: false, shift: true })).toBe("ctrl+j");
    expect(stickyKey("\r", { ctrl: false, alt: false, shift: true })).toBe("ctrl+j");
    expect(stickyKey("\n", { ctrl: false, alt: false })).toBeNull();
    expect(stickyKey("a", { ctrl: false, alt: false, shift: true })).toBeNull();
  });

  it("whatever the next key is: line breaks, tab, escape, ctrl+[ and ctrl+@", () => {
    expect(stickyKey("\n", { ctrl: true, alt: false })).toBe("ctrl+enter"); // the soft keyboard's Return
    expect(stickyKey("\r", { ctrl: false, alt: true })).toBe("alt+enter");
    expect(stickyKey("\t", { ctrl: true, alt: false })).toBe("ctrl+tab");
    expect(stickyKey("\u007f", { ctrl: false, alt: true })).toBe("alt+backspace");
    expect(stickyKey("\u001b", { ctrl: false, alt: true })).toBe("alt+esc");
    expect(stickyKey("[", { ctrl: true, alt: false })).toBe("esc");
    expect(stickyKey("[", { ctrl: true, alt: true })).toBe("alt+esc");
    expect(stickyKey("@", { ctrl: true, alt: false })).toBe("ctrl+space");
    expect(stickyKey("@", { ctrl: true, alt: true })).toBe("ctrl+alt+space");
    expect(applySticky("left", { ctrl: false, alt: true })).toBe("alt+left");
    expect(applySticky("enter", { ctrl: true, alt: false })).toBe("ctrl+enter");
  });

  it("a key the protocol cannot carry with it is sent plain, or typed as text", () => {
    expect(stickyKey("\u0003", { ctrl: true, alt: false })).toBe("ctrl+c");
    expect(applySticky("ctrl+space", { ctrl: false, alt: false })).toBe("ctrl+space");
    expect(stickyKey("1", { ctrl: true, alt: false })).toBeNull(); // typed as "1"
    expect(stickyKey("[", { ctrl: false, alt: true })).toBeNull(); // typed as "["
    expect(stickyKey("é", { ctrl: true, alt: false })).toBeNull();
  });
});

describe("batching", () => {
  it("splits text at 1 KiB of UTF-8, never inside a character", () => {
    expect(splitText("x".repeat(1024))).toEqual(["x".repeat(1024)]);
    expect(splitText("x".repeat(1025))).toEqual(["x".repeat(1024), "x"]);
    const parts = splitText("é".repeat(600));
    expect(parts.map((p) => p.length)).toEqual([512, 88]);
    expect(splitText("😀".repeat(300)).every((p) => isClientMsg({ t: "text", id: "x1", pane: 1, text: p }))).toBe(true);
  });

  it("merges adjacent text and adjacent keys, keeping the order", () => {
    expect(mergeActions([{ text: "a" }, { text: "b" }, { keys: ["enter"] }, { keys: ["up"] }, { text: "c" }])).toEqual([
      { text: "ab" },
      { keys: ["enter", "up"] },
      { text: "c" },
    ]);
    const many = Array.from({ length: 45 }, () => ({ keys: ["backspace"] }));
    expect(mergeActions(many).map((a) => ("keys" in a ? a.keys.length : 0))).toEqual([20, 20, 5]);
  });

  it("does not change the queued actions", () => {
    const queued: InputAction[] = [{ text: "a" }, { text: "b" }];
    mergeActions(queued);
    expect(queued).toEqual([{ text: "a" }, { text: "b" }]);
  });

  function harness() {
    let now = 0;
    const timers: { at: number; fn: () => void }[] = [];
    const sent: InputAction[] = [];
    const b = new InputBatcher(
      (a) => sent.push(a),
      (fn, delay) => timers.push({ at: now + Math.max(delay, 16), fn }),
      () => now,
    );
    const advance = (ms: number) => {
      now += ms;
      for (;;) {
        const due = timers.filter((t) => t.at <= now).sort((x, y) => x.at - y.at)[0];
        if (!due) break;
        timers.splice(timers.indexOf(due), 1);
        due.fn();
      }
    };
    return { b, sent, advance, timers };
  }

  it("fast typing goes out as one message per frame, not one per character", () => {
    const h = harness();
    for (const ch of "hello world") h.b.push({ text: ch });
    expect(h.sent).toEqual([]);
    h.advance(16);
    expect(h.sent).toEqual([{ text: "hello world" }]);
  });

  it("sends at most every 50 ms (the gateway takes 20 a second), batching what came meanwhile", () => {
    const h = harness();
    let messages = 0;
    for (let ms = 0; ms < 1000; ms += 10) {
      h.b.push({ keys: ["backspace"] }); // a held key repeating every 10 ms
      h.advance(10);
      messages = h.sent.length;
    }
    h.advance(100);
    expect(messages).toBeLessThanOrEqual(1000 / MIN_SEND_INTERVAL_MS + 1);
    expect(h.sent.reduce((n, a) => n + ("keys" in a ? a.keys.length : 0), 0)).toBe(100);
  });

  it("flush sends at once", () => {
    const h = harness();
    h.b.push({ text: "a" });
    h.b.flush();
    expect(h.sent).toEqual([{ text: "a" }]);
  });
});

describe("input gate (rate and authentication)", () => {
  function gated(opts: { slots?: () => number; auth?: (a: InputAction) => Promise<boolean> | null }) {
    let now = 0;
    const timers: { at: number; fn: () => void }[] = [];
    const sent: InputAction[] = [];
    let wait = 0;
    const b = new InputBatcher(
      (a) => sent.push(a),
      (fn, delay) => timers.push({ at: now + Math.max(delay, 16), fn }),
      () => now,
      { delay: () => wait, authorize: opts.auth },
    );
    const advance = async (ms: number) => {
      now += ms;
      for (;;) {
        await Promise.resolve();
        await Promise.resolve();
        const due = timers.filter((t) => t.at <= now).sort((x, y) => x.at - y.at)[0];
        if (!due) break;
        timers.splice(timers.indexOf(due), 1);
        due.fn();
      }
    };
    return { b, sent, advance, setWait: (ms: number) => (wait = ms) };
  }

  it("while the rate holds input back, what is typed meanwhile merges", async () => {
    const h = gated({});
    h.setWait(300);
    for (const ch of "abc") {
      h.b.push({ text: ch });
      await h.advance(100);
    }
    expect(h.sent).toEqual([]);
    h.setWait(0);
    await h.advance(300);
    expect(h.sent).toEqual([{ text: "abc" }]);
  });

  it("a key that needs authentication holds everything after it; text typed meanwhile merges", async () => {
    let answer: (ok: boolean) => void = () => undefined;
    const h = gated({
      auth: (a) => ("keys" in a && a.keys.includes("enter") ? new Promise<boolean>((r) => (answer = r)) : null),
    });
    h.b.push({ text: "ls" }, { keys: ["enter"] }, { text: "x" });
    await h.advance(16);
    expect(h.sent).toEqual([{ text: "ls" }]);
    h.b.push({ text: "y" });
    h.b.push({ text: "z" });
    await h.advance(1000);
    expect(h.sent).toEqual([{ text: "ls" }]);
    answer(true);
    await h.advance(0);
    await h.advance(16);
    expect(h.sent).toEqual([{ text: "ls" }, { keys: ["enter"] }, { text: "xyz" }]);
  });

  it("a cancelled authentication drops that key only", async () => {
    const h = gated({ auth: (a) => ("keys" in a ? Promise.resolve(false) : null) });
    h.b.push({ text: "a" }, { keys: ["enter"] }, { text: "b" });
    await h.advance(16);
    await h.advance(16);
    expect(h.sent).toEqual([{ text: "a" }, { text: "b" }]);
  });
});

describe("terminalInput (F8: sends never wait for answers)", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("typing 10 characters a second over a 300 ms round trip leaves within a frame of each key", async () => {
    const RTT = 300;
    const wire: { at: number; a: InputAction }[] = [];
    const answered = (a: InputAction) => {
      wire.push({ at: Date.now(), a });
      return new Promise<{ ok: true }>((r) => setTimeout(() => r({ ok: true }), RTT));
    };
    const reports: unknown[] = [];
    const b = terminalInput({
      sendKeys: (keys) => answered({ keys }),
      sendText: (text) => answered({ text }),
      online: () => true,
      delay: () => 0,
      authorize: () => null,
      report: (r) => reports.push(r),
      schedule: (fn, d) => setTimeout(fn, d > 0 ? d : 16),
      now: () => Date.now(),
    });
    const typedAt: number[] = [];
    for (let i = 0; i < 30; i++) {
      typedAt.push(Date.now());
      b.push({ text: String.fromCharCode(97 + (i % 26)) });
      await vi.advanceTimersByTimeAsync(100);
    }
    await vi.advanceTimersByTimeAsync(2_000);
    expect(wire.map((w) => ("text" in w.a ? w.a.text : "")).join("")).toBe(
      Array.from({ length: 30 }, (_, i) => String.fromCharCode(97 + (i % 26))).join(""),
    );
    // each character leaves within one frame of being typed, not behind the previous answer
    expect(wire.at(-1)!.at - typedAt.at(-1)!).toBeLessThanOrEqual(16);
    expect(reports.length).toBe(wire.length);
  });

  it("a busy answer to text reports the prompt, other failures their message", async () => {
    const reports: unknown[] = [];
    const b = terminalInput({
      sendKeys: async () => ({ ok: false, code: "rate", message: "Too many" }),
      sendText: async () => ({ ok: false, code: "busy", message: "busy" }),
      online: () => true,
      delay: () => 0,
      authorize: () => null,
      report: (r) => reports.push(r),
      schedule: (fn, d) => setTimeout(fn, d > 0 ? d : 16),
      now: () => Date.now(),
    });
    b.push({ text: "a" }, { keys: ["up"] });
    await vi.advanceTimersByTimeAsync(50);
    expect(reports).toEqual(["busy", { error: "Too many" }]);
  });
});
