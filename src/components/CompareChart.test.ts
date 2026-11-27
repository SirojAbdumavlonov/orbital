import { expect, it, vi } from "vitest";
vi.mock("../lib/api", () => ({ api: {} }));
vi.mock("./HistoryChart", () => ({ paths: () => "" }));
const { topFor } = await import("./CompareChart");

it("scales percentages to 100 and the rest to the highest value", () => {
  const p = (load: number, rx: number) => ({ t: 0, cpu: 1, mem: 1, disk: 1, load, rx, tx: null, uptime: null });
  expect(topFor("cpu", [[p(1, 1)]])).toBe(100);
  expect(topFor("load", [[p(0.2, 1)], [p(3.5, 1)]])).toBe(3.5);
  expect(topFor("load", [[p(0.2, 1)]])).toBe(1);
  expect(topFor("rx", [[p(0, 50_000)]])).toBe(50_000);
});
