import { expect, it } from "vitest";
import { noteTitle } from "./notes";

it("previews a note by its first line", () => {
  expect(noteTitle("\n\n# Shop API\n- deploy")).toBe("Shop API");
  expect(noteTitle("   ")).toBe("");
  expect(noteTitle("x".repeat(200)).length).toBe(80);
});
