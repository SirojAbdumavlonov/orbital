import { expect, it, vi } from "vitest";
vi.mock("@tauri-apps/plugin-dialog", () => ({ save: async () => null }));
vi.mock("../store", () => ({ useStore: () => null }));
vi.mock("../components/ui", () => ({ confirm: async () => true, Modal: () => null }));
const { dbChoices, newId, scheduleText, sourceText } = await import("./BackupsView");

const job = (over: any = {}) => ({ id: "x", name: "X", keep: 7, source: { kind: "folder", path: "/var/www/shop" }, schedule: { every: "day", weekday: 0, hour: 3, minute: 5 }, ...over });

it("describes jobs in words", () => {
  expect(scheduleText(job() as any)).toBe("Every day at 03:05");
  expect(scheduleText(job({ schedule: { every: "week", weekday: 0, hour: 22, minute: 30 } }) as any)).toBe("Sundays at 22:30");
  expect(sourceText(job({ source: { kind: "database", engine: "postgres", name: "shop" } }) as any)).toBe("PostgreSQL database shop");
  expect(sourceText(job() as any)).toBe("/var/www/shop");
});

it("makes ids that are slugs and not taken", () => {
  expect(newId("Shop uploads!", [])).toBe("shop-uploads");
  expect(newId("Shop uploads", ["shop-uploads"])).toBe("shop-uploads-2");
  expect(newId("  ", [])).toBe("backup");
});

it("offers database servers on the server and in containers", () => {
  expect(dbChoices(null)).toEqual([]);
  expect(dbChoices([{ id: "nginx", onHost: true, containers: [] }])).toEqual([]);
  const c = dbChoices([{ id: "postgresql", onHost: true, containers: ["pg-old"] }, { id: "mongodb", onHost: false, containers: ["mongo"] }]);
  expect(c.map((p) => [p.engine, p.container, p.label])).toEqual([
    ["postgres", null, "PostgreSQL on this server"],
    ["postgres", "pg-old", "PostgreSQL in the container pg-old"],
    ["mongo", "mongo", "MongoDB in the container mongo"],
  ]);
});
