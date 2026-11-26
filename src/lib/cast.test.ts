import { describe, expect, it } from "vitest";
import { parseCast } from "./cast";

const CAST = [
  JSON.stringify({ version: 2, width: 100, height: 30, timestamp: 1, title: "deploy@203.0.113.10: ~" }),
  JSON.stringify([0.5, "o", "$ ls\r\n"]),
  "not json",
  JSON.stringify([1.0, "r", "120x40"]),
  JSON.stringify([31.0, "o", "after a long pause"]),
  JSON.stringify([31.5, "o", "!"]),
].join("\n");

describe("asciinema recordings", () => {
  it("reads the header and events, skipping broken lines", () => {
    const c = parseCast(CAST);
    expect([c.width, c.height, c.title]).toEqual([100, 30, "deploy@203.0.113.10: ~"]);
    expect(c.events.map((e) => e.kind)).toEqual(["o", "r", "o", "o"]);
    expect(c.length).toBe(31.5);
  });

  it("shortens long pauses", () => {
    const c = parseCast(CAST, 2);
    expect(c.events[2].t).toBe(3);
    expect(c.events[3].t).toBe(3.5);
    expect(c.length).toBe(3.5);
  });

  it("copes with something that isn't a recording", () => {
    const c = parseCast("hello");
    expect(c.events).toEqual([]);
    expect(c.length).toBe(0);
  });
});
