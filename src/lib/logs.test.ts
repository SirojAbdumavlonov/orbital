import { describe, expect, it } from "vitest";
import { filterLines, keepTail, level } from "./logs";

describe("log helpers", () => {
  it("rates lines", () => {
    expect(level("2026-11-20 ERROR connect ECONNREFUSED")).toBe("error");
    expect(level('1.2.3.4 - - "GET / HTTP/1.1" 502 12')).toBe("error");
    expect(level("WARN slow query")).toBe("warn");
    expect(level("INFO started")).toBeNull();
  });
  it("keeps the newest lines", () => {
    expect(keepTail("a\nb\nc\nd\n", 2)).toBe("c\nd\n");
    expect(keepTail("a\nb\n", 5)).toBe("a\nb\n");
  });
  it("filters by text and problems", () => {
    const t = "INFO up\nWARN slow\nERROR down\nINFO Down again\n";
    expect(filterLines(t, "down", false).map((l) => l.text)).toEqual(["ERROR down", "INFO Down again"]);
    expect(filterLines(t, "", true).map((l) => l.level)).toEqual(["warn", "error"]);
    expect(filterLines(t, "", false)).toHaveLength(4);
  });
});
