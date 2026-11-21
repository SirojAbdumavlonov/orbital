import { describe, expect, it } from "vitest";
import { forgetKept, keepValue, keptValue } from "./kept";

describe("kept view state", () => {
  it("is kept per server and view, and forgotten for a server that disconnects", () => {
    expect(keptValue("s1:docker:sel", null)).toBeNull();
    keepValue("s1:docker:sel", "grafana");
    keepValue("s1:services:filter", "failed");
    expect(keptValue("s1:docker:sel", null)).toBe("grafana");
    expect(keptValue("s2:docker:sel", null)).toBeNull();
    forgetKept("s1:");
    expect(keptValue("s1:docker:sel", null)).toBeNull();
    expect(keptValue("s1:services:filter", "running")).toBe("running");
  });
});
