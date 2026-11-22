import { expect, it } from "vitest";
import { forStorage, MAX_RUNS } from "./netHistory";

const run = (n: number, over: any = {}) => ({ n, check: "trace" as const, target: "example.com", port: null, at: n, ...over });

it("stores finished results, trims long output, and marks a check cut short by quitting", () => {
  const long = { ok: true, summary: "12 hops", command: "traceroute example.com", output: "x".repeat(30_000) };
  const out = forStorage([run(3, { busy: true }), run(2, { result: long }), run(1, { error: "refused" })]);
  expect(out[0]).toMatchObject({ n: 3, error: "Orbital closed before this check finished" });
  expect(out[0].busy).toBeUndefined();
  expect(out[1].result!.output.length).toBeLessThan(20_100);
  expect(out[1].result!.output.endsWith("… (cut)")).toBe(true);
  expect(out[2]).toMatchObject({ error: "refused" });
});

it("keeps the newest 30", () => {
  expect(forStorage(Array.from({ length: 40 }, (_, i) => run(i))).length).toBe(MAX_RUNS);
});
