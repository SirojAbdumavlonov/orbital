import { describe, expect, it, vi } from "vitest";
vi.mock("../store", () => ({ useStore: { subscribe: () => () => {}, getState: () => ({ sessions: {} }) } }));
const { addPoint, point, DAY } = await import("./metricHistory");

const GB = 1024 ** 3;
const m = (over: any = {}) => ({
  sampledAtMs: 1_795_000_030_000, cpuPercent: 12.345, memTotal: 8 * GB, memAvailable: 2 * GB,
  disks: [
    { fsType: "ext4", mount: "/", totalBytes: 100 * GB, usedPercent: 41.26 },
    { fsType: "xfs", mount: "/data", totalBytes: 100 * GB, usedPercent: 77.71 },
    { fsType: "tmpfs", mount: "/run", totalBytes: GB, usedPercent: 99 },
  ],
  ...over,
}) as any;

describe("metric history", () => {
  it("takes one point a minute: CPU, memory used and the fullest real disk", () => {
    expect(point(m())).toEqual({ t: 1_795_000_020, cpu: 12.3, mem: 75, disk: 77.7 });
    expect(point(m({ cpuPercent: null, disks: [] }))).toMatchObject({ cpu: null, disk: null });
  });
  it("replaces a point for the same minute and keeps one day", () => {
    let pts = addPoint([], { t: 60, cpu: 1, mem: 1, disk: 1 });
    pts = addPoint(pts, { t: 60, cpu: 2, mem: 2, disk: 2 });
    expect(pts).toEqual([{ t: 60, cpu: 2, mem: 2, disk: 2 }]);
    pts = addPoint(pts, { t: 60 + DAY, cpu: 3, mem: 3, disk: 3 });
    expect(pts.map((p) => p.t)).toEqual([60 + DAY]);
  });
});
