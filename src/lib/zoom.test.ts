import { expect, it, vi } from "vitest";
vi.mock("./settings", () => ({ useSettings: Object.assign(() => 1, { getState: () => ({}) }) }));
const { zoomKey, nextUiZoom, nextFont } = await import("./zoom");

const k = (key: string, code: string, mods: Partial<KeyboardEvent> = {}) => ({ key, code, metaKey: false, ctrlKey: false, altKey: false, ...mods });

it("reads the zoom shortcuts per platform", () => {
  expect(zoomKey(k("=", "Equal", { metaKey: true }), true)).toBe("in");
  expect(zoomKey(k("+", "Equal", { metaKey: true }), true)).toBe("in");
  expect(zoomKey(k("-", "Minus", { metaKey: true }), true)).toBe("out");
  expect(zoomKey(k("0", "Digit0", { metaKey: true }), true)).toBe("reset");
  expect(zoomKey(k("=", "Equal", { ctrlKey: true }), true)).toBeNull();
  expect(zoomKey(k("=", "Equal", { ctrlKey: true }), false)).toBe("in");
  expect(zoomKey(k("=", "Equal", { metaKey: true, altKey: true }), true)).toBeNull();
  expect(zoomKey(k("a", "KeyA", { metaKey: true }), true)).toBeNull();
});

it("steps the sizes within limits", () => {
  expect(nextUiZoom(1, "in")).toBe(1.1);
  expect(nextUiZoom(1.6, "in")).toBe(1.6);
  expect(nextUiZoom(0.7, "out")).toBe(0.7);
  expect(nextUiZoom(1.3, "reset")).toBe(1);
  expect(nextFont(13, "in")).toBe(14);
  expect(nextFont(8, "out")).toBe(8);
  expect(nextFont(20, "reset")).toBe(13);
});
