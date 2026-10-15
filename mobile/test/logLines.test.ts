import { describe, expect, it } from "vitest";
import { parseLogLines } from "../src/paneView";

describe("parseLogLines", () => {
  it("reads compose-style lines into source, level, message and time", () => {
    expect(parseLogLines(["api-1  | [info] GET /health 200 3ms", "api-1  | [warn] slow query 812ms"])).toEqual({
      source: "api-1",
      rows: [
        { level: "INFO", message: "GET /health 200", time: "3ms" },
        { level: "WARN", message: "slow query", time: "812ms" },
      ],
    });
    expect(parseLogLines(["INFO: server ready", "ERROR boom"])).toEqual({
      source: null,
      rows: [
        { level: "INFO", message: "server ready", time: "" },
        { level: "ERROR", message: "boom", time: "" },
      ],
    });
  });
  it("leaves lines without a level, mixed sources and nothing alone", () => {
    expect(parseLogLines(["Compiling wgpu v29.0.1", "INFO done"])).toBeNull();
    expect(parseLogLines(["api-1 | [info] a", "db-1 | [info] b"])).toBeNull();
    expect(parseLogLines(["information about it"])).toBeNull();
    expect(parseLogLines([])).toBeNull();
  });
});
