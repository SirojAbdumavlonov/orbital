import { expect, it, vi } from "vitest";
vi.mock("./api", () => ({ api: { terminalWrite: vi.fn(async () => {}) } }));
const { fanOut, useBroadcast, unregisterTerminal } = await import("./broadcast");

it("sends keys only from a target, only while on, never back to itself", () => {
  expect(fanOut(true, ["a", "b", "c"], "a")).toEqual(["b", "c"]);
  expect(fanOut(true, ["a", "b"], "z")).toEqual([]);
  expect(fanOut(false, ["a", "b"], "a")).toEqual([]);
});

it("turns itself off when fewer than two terminals are left", () => {
  useBroadcast.getState().setTargets(["a", "b"]);
  useBroadcast.getState().setOn(true);
  unregisterTerminal("b");
  expect(useBroadcast.getState()).toMatchObject({ on: false, targets: ["a"] });
});
