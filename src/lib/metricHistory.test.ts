import { describe, expect, it, vi } from "vitest";
vi.mock("./api", () => ({ api: {} }));
const { diskForecast, fromOld, toCsv } = await import("./metricHistory");

const pt = (t: number, disk: number | null) => ({ t, cpu: 1, mem: 2, disk, load: 0.5, rx: 10, tx: null, uptime: 100 });

describe("monitoring history", () => {
  it("forecasts when a filling disk is full, only from a rising day's worth", () => {
    // 1 % per hour from 50 % over 24 hours: 74 % now, full in 26 h.
    const rising = Array.from({ length: 25 }, (_, i) => pt(i * 3600, 50 + i));
    expect(diskForecast(rising)).toBeCloseTo(26 / 24, 1);
    expect(diskForecast(rising.map((p) => ({ ...p, disk: 60 })))).toBeNull();
    expect(diskForecast(rising.slice(0, 5))).toBeNull();
    expect(diskForecast(Array.from({ length: 10 }, (_, i) => pt(i * 60, 50 + i)))).toBeNull();
    // Very slow growth: more than a year away, not worth saying.
    expect(diskForecast(Array.from({ length: 25 }, (_, i) => pt(i * 3600, 50 + i * 0.0001)))).toBeNull();
  });
  it("exports CSV with empty cells for missing values", () => {
    const csv = toCsv([pt(0, null)]);
    expect(csv.split("\n")[0]).toContain("fullest_disk_percent");
    expect(csv.split("\n")[1]).toBe("1970-01-01T00:00:00.000Z,1,2,,0.5,10,");
  });
  it("reads points kept by older versions", () => {
    expect(fromOld({ t: 60, cpu: 5, mem: 40, disk: null })).toEqual({ t: 60, cpu: 5, mem: 40, disk: null, load: 0, rx: null, tx: null, uptime: null });
    expect(fromOld({ t: "x" })).toBeNull();
  });
});
