import { expect, it, vi } from "vitest";
vi.mock("./api", () => ({ api: {} }));
const { parseStart } = await import("./windows");

it("reads what a window was opened for, ignoring anything odd", () => {
  expect(parseStart("?attach=ab12-cd&tab=tool:docker")).toEqual({ attach: "ab12-cd", connect: null, tab: "tool:docker" });
  expect(parseStart("?connect=p1")).toEqual({ attach: null, connect: "p1", tab: null });
  expect(parseStart("")).toEqual({ attach: null, connect: null, tab: null });
  expect(parseStart("?attach=x%26y&tab=<b>")).toEqual({ attach: null, connect: null, tab: null });
});
