import { describe, expect, it } from "vitest";
import { bounds, fromLocalInput, label, ticks, toLocalInput } from "./range";

describe("shared date range", () => {
  it("presets end now; custom ranges are ordered", () => {
    expect(bounds({ preset: "1h" }, 10_000)).toEqual({ from: 6400, to: 10_000 });
    expect(bounds({ preset: "30d" }, 3_000_000).from).toBe(3_000_000 - 30 * 86400);
    expect(bounds({ from: 500, to: 100 })).toEqual({ from: 100, to: 500 });
    expect(label({ preset: "7d" })).toBe("Last 7 days");
  });
  it("ticks span the range", () => {
    const t = ticks(0, 4 * 3600, 5);
    expect(t.map((x) => x.t)).toEqual([0, 3600, 7200, 10800, 14400]);
  });
  it("datetime-local round trip", () => {
    const s = 1_800_000_000;
    expect(fromLocalInput(toLocalInput(s))).toBe(s - (s % 60));
    expect(fromLocalInput("nonsense")).toBeNull();
  });
});
