import { expect, it, vi } from "vitest";
vi.mock("../lib/api", () => ({ api: {}, asApiError: (e: any) => e }));
vi.mock("../store", () => ({ useStore: () => () => {} }));
const { paths, nearest, eventsNear, describeEvent } = await import("./HistoryChart");

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

it("finds the moment under the pointer and what changed then", () => {
  const pts = [{ t: 0 }, { t: 60 }, { t: 600 }];
  expect(nearest(pts, 70, 180)?.t).toBe(60);
  expect(nearest(pts, 400, 100)).toBeNull();
  const ev = [
    { t: 30, kind: "service", action: "service.restart", target: "nginx", ok: true },
    { t: 500, kind: "restart", action: "server.restart", target: null, ok: true },
  ];
  expect(eventsNear(ev, 0, 120).length).toBe(1);
  expect(describeEvent(ev[0])).toBe("Restart nginx");
  expect(describeEvent(ev[1])).toBe("Server restarted");
  expect(describeEvent({ t: 0, kind: "config", action: "config.safe_save", target: "/etc/x.conf", ok: false })).toBe("Safe save /etc/x.conf (failed)");
});
