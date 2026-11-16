import { beforeAll, describe, expect, it, vi } from "vitest";

// The platform is read when the module loads: pretend to be Windows first.
let p: typeof import("./platform");
beforeAll(async () => {
  vi.stubGlobal("navigator", { platform: "Win32", userAgent: "Mozilla/5.0 (Windows NT 10.0; Win64; x64)" });
  p = await import("./platform");
});
const key = (k: Partial<KeyboardEvent>) => ({ metaKey: false, ctrlKey: false, altKey: false, shiftKey: false, key: "", ...k }) as KeyboardEvent;

describe("shortcuts on Windows", () => {
  it("uses Ctrl as the command key", () => {
    expect(p.isMac).toBe(false);
    expect(p.isWindows).toBe(true);
    expect(p.mod(key({ ctrlKey: true }))).toBe(true);
    expect(p.mod(key({ metaKey: true }))).toBe(false);
    expect(p.KEYS.commandMenu).toBe("Ctrl+K");
  });
  it("goes back and forward with Alt+arrows only", () => {
    expect(p.navKey(key({ altKey: true, key: "ArrowLeft" }))).toBe(-1);
    expect(p.navKey(key({ altKey: true, key: "ArrowRight" }))).toBe(1);
    expect(p.navKey(key({ key: "ArrowLeft" }))).toBe(0);
    expect(p.navKey(key({ altKey: true, ctrlKey: true, key: "ArrowLeft" }))).toBe(0);
    expect(p.navKey(key({ metaKey: true, key: "[" }))).toBe(0);
  });
});
