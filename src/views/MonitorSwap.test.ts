import { expect, it, vi } from "vitest";
vi.mock("../components/HistoryChart", () => ({ HistoryChart: () => null }));
vi.mock("../components/ui", () => ({ Bar: () => null, confirm: async () => true, LiveNumber: () => null, Modal: () => null, Sparkline: () => null }));
vi.mock("../store", () => ({ isSsh: () => true, useStore: () => null }));
const { suggestSwapMb } = await import("./MonitorView");

it("suggests swap like the engine does", () => {
  const gb = 1024 ** 3;
  expect(suggestSwapMb(gb)).toBe(2048);
  expect(suggestSwapMb(4 * gb)).toBe(4096);
  expect(suggestSwapMb(64 * gb)).toBe(4096);
  expect(suggestSwapMb(256 * 1024 * 1024)).toBe(1024);
});
