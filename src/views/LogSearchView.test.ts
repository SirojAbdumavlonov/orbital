import { expect, it, vi } from "vitest";
vi.mock("../store", () => ({ useStore: () => null }));
const { marks } = await import("./LogSearchView");

it("highlights every match, in any case", () => {
  expect(marks("Upstream TIMED out, timed out again", "timed out")).toEqual([
    { text: "Upstream ", hit: false }, { text: "TIMED out", hit: true }, { text: ", ", hit: false }, { text: "timed out", hit: true }, { text: " again", hit: false },
  ]);
  expect(marks("abc", "")).toEqual([{ text: "abc", hit: false }]);
});
