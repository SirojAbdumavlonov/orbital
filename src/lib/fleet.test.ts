import { expect, it, vi } from "vitest";
vi.mock("../store", () => ({ isSsh: () => true, useStore: { getState: () => ({ sessions: {} }), subscribe: () => () => {} } }));
const { summarize, updatesText } = await import("./fleet");

it("summarizes pending updates for a card", () => {
  const u = summarize({ manager: "apt", rebootRequired: true, refreshedAt: null, packages: [
    { name: "openssl", current: "3.0.1", available: "3.0.2", security: true },
    { name: "vim", current: "9.0", available: "9.1", security: false },
  ] });
  expect(updatesText(u)).toBe("2 updates (1 security) · reboot needed");
  expect(updatesText({ count: 1, security: 0, reboot: false, at: 0 })).toBe("1 update");
  expect(updatesText({ count: 0, security: 0, reboot: false, at: 0 })).toBeNull();
  expect(updatesText(undefined)).toBeNull();
});
