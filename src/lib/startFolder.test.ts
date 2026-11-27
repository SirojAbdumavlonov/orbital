import { expect, it } from "vitest";
import { startFolder } from "./startFolder";

it("picks the Files start folder", () => {
  const p = { initialPath: null };
  expect(startFolder("/home/ann", p, "home", "", null)).toBe("/home/ann");
  expect(startFolder("/home/ann", p, "root", "", null)).toBe("/");
  expect(startFolder("/home/ann", p, "last", "", "/var/www")).toBe("/var/www");
  expect(startFolder("/home/ann", p, "last", "", null)).toBe("/home/ann");
  expect(startFolder("/home/ann", p, "custom", " /srv/apps/ ", null)).toBe("/srv/apps");
  expect(startFolder("/home/ann", p, "custom", "relative", null)).toBe("/home/ann");
  // A server's own start folder wins (the engine already put it in `home`).
  expect(startFolder("/opt/app", { initialPath: "/opt/app" }, "root", "", "/tmp")).toBe("/opt/app");
});
