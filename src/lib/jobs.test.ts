import { expect, it, vi } from "vitest";
vi.mock("./api", () => ({ api: {} }));
vi.mock("../store", () => ({ useStore: { getState: () => ({}) }, goTo: () => {} }));
const { reportsElsewhere } = await import("./jobs");

it("only long jobs that finished or failed go to the alert channels", () => {
  expect(reportsElsewhere({ status: "done", started: 0, ended: 180_000 })).toBe(true);
  expect(reportsElsewhere({ status: "failed", started: 0, ended: 120_000 })).toBe(true);
  expect(reportsElsewhere({ status: "done", started: 0, ended: 60_000 })).toBe(false);
  expect(reportsElsewhere({ status: "stopped", started: 0, ended: 600_000 })).toBe(false);
  expect(reportsElsewhere({ status: "running", started: 0 })).toBe(false);
});
