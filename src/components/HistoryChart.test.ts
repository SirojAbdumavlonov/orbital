import { expect, it, vi } from "vitest";
vi.mock("../lib/metricHistory", () => ({ DAY: 86400, historyOf: () => [], useHistory: () => undefined }));
const { paths } = await import("./HistoryChart");

it("draws a line per series and breaks it where Orbital wasn't connected", () => {
  const pts = [
    { t: 1000, cpu: 10, mem: 50, disk: null },
    { t: 1060, cpu: 20, mem: 50, disk: null },
    { t: 2000, cpu: 30, mem: 50, disk: null },
  ];
  const d = paths(pts, "cpu", 0, 3600);
  expect(d.match(/M/g)!.length).toBe(2);
  expect(d.match(/L/g)!.length).toBe(1);
  expect(paths(pts, "disk", 0, 3600)).toBe("");
});
