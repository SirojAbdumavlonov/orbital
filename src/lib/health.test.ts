import { describe, expect, it, vi } from "vitest";
vi.mock("../store", () => ({ isSsh: () => true, useStore: { getState: () => ({ sessions: {} }), subscribe: () => () => {} } }));
vi.mock("./settings", () => ({ useSettings: { getState: () => ({ healthAlerts: true }) } }));
const { assess, alertStep, NO_ALERTS } = await import("./health");

const GB = 1024 ** 3;
const metrics = (over: any = {}) => ({
  cores: 4, load: [0.5, 0.4, 0.3], memTotal: 8 * GB, memAvailable: 4 * GB,
  disks: [{ filesystem: "/dev/sda1", fsType: "ext4", mount: "/", totalBytes: 100 * GB, usedBytes: 40 * GB, availableBytes: 60 * GB, usedPercent: 40 }],
  ...over,
}) as any;

describe("assess", () => {
  it("is unknown without metrics and ok when all is well", () => {
    expect(assess(undefined, []).level).toBe("unknown");
    expect(assess(metrics(), [])).toEqual({ level: "ok", reasons: [] });
  });
  it("warns and alarms on disks, memory and load, worst first", () => {
    const h = assess(metrics({
      disks: [
        { filesystem: "/dev/sda1", fsType: "ext4", mount: "/", totalBytes: 100 * GB, usedBytes: 0, availableBytes: 0, usedPercent: 88 },
        { filesystem: "/dev/sdb1", fsType: "xfs", mount: "/data", totalBytes: 100 * GB, usedBytes: 0, availableBytes: 0, usedPercent: 97 },
        { filesystem: "tmpfs", fsType: "tmpfs", mount: "/run", totalBytes: GB, usedBytes: 0, availableBytes: 0, usedPercent: 100 },
      ],
      memAvailable: 0.6 * GB,
      load: [9, 1, 1],
    }), []);
    expect(h.level).toBe("bad");
    expect(h.reasons).toEqual(["Disk /data 97%", "Load 9.0 on 4 cores", "Disk / 88%", "Memory 93% used"]);
  });
  it("names failed services", () => {
    expect(assess(metrics(), ["nginx.service"])).toEqual({ level: "warn", reasons: ["nginx failed"] });
    expect(assess(metrics(), ["a.service", "b.service", "c.service", "d.service"]).reasons[0]).toBe("4 services failed (a, b, c, …)");
  });
});

describe("alertStep", () => {
  const red = { level: "bad" as const, reasons: ["Disk / 97%"] };
  const green = { level: "ok" as const, reasons: [] };
  it("alerts after two red checks in a row, once, and when it's back", () => {
    let st = NO_ALERTS;
    let r = alertStep(st, "web", red, []);
    expect(r.messages).toEqual([]);
    r = alertStep(r.next, "web", red, []);
    expect(r.messages).toEqual([{ kind: "bad", title: "web needs attention", body: "Disk / 97%" }]);
    r = alertStep(r.next, "web", red, []);
    expect(r.messages).toEqual([]);
    r = alertStep(r.next, "web", green, []);
    expect(r.messages[0]).toMatchObject({ kind: "ok", title: "web is back to normal" });
    st = r.next;
    expect(alertStep(st, "web", green, []).messages).toEqual([]);
  });
  it("a single red check (a load spike) doesn't alert", () => {
    const a = alertStep(NO_ALERTS, "web", red, []);
    expect(alertStep(a.next, "web", green, []).messages).toEqual([]);
  });
  it("tells about a failed service at once, and only once", () => {
    const warn = { level: "warn" as const, reasons: ["nginx failed"] };
    const a = alertStep(NO_ALERTS, "web", warn, ["nginx.service"]);
    expect(a.messages).toEqual([{ kind: "bad", title: "web: nginx failed", body: "nginx" }]);
    expect(alertStep(a.next, "web", warn, ["nginx.service"]).messages).toEqual([]);
    expect(alertStep(a.next, "web", green, []).messages[0].kind).toBe("ok");
  });
  it("ignores checks without numbers", () => {
    expect(alertStep(NO_ALERTS, "web", { level: "unknown", reasons: [] }, [])).toEqual({ next: NO_ALERTS, messages: [] });
  });
});
