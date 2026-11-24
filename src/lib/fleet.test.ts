import { expect, it, vi } from "vitest";
vi.mock("../store", () => ({ isSsh: () => true, useStore: { getState: () => ({ sessions: {} }), subscribe: () => () => {} } }));
const { summarize, updatesText, expiring, certText } = await import("./fleet");

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

it("warns about certificates that expire soon", () => {
  const cert = (d: string, daysLeft: number | null) => ({ name: d, domains: [d], expires: null, daysLeft, valid: true, keyType: null, certificatePath: null, privateKeyPath: null, status: "" });
  const e = expiring([cert("a.example.com", 60), cert("b.example.com", 9), cert("c.example.com", -2), cert("d.example.com", null)]);
  expect(e.map((x) => x.name)).toEqual(["c.example.com", "b.example.com"]);
  expect(certText(e)).toBe("HTTPS for c.example.com expired 2 days ago (+1 more)");
  expect(certText([{ name: "x", daysLeft: 1 }])).toBe("HTTPS for x expires in 1 day");
  expect(certText([])).toBeNull();
});
