import { describe, expect, it } from "vitest";
import { compare, dateChangelog, next } from "./release.mjs";

describe("release versions", () => {
  it("bumps by kind", () => {
    expect(next("1.0.0", "patch")).toBe("1.0.1");
    expect(next("1.0.1", "minor")).toBe("1.1.0");
    expect(next("1.1.0", "major")).toBe("2.0.0");
    expect(next("1.1.0", "beta")).toBe("1.1.1-beta.1");
    expect(next("1.1.1-beta.1", "beta")).toBe("1.1.1-beta.2");
    expect(next("1.1.1-beta.2", "patch")).toBe("1.1.1");
    expect(next("1.2.0-beta.3", "minor")).toBe("1.2.0");
    expect(next("1.0.0", "1.4.2")).toBe("1.4.2");
  });

  it("orders like the updater does", () => {
    expect(compare("1.0.10", "1.0.9")).toBe(1);
    expect(compare("1.1.0-beta.1", "1.1.0")).toBe(-1);
    expect(compare("1.1.0-beta.10", "1.1.0-beta.2")).toBe(1);
    expect(compare("2.0.0", "2.0.0")).toBe(0);
  });

  it("dates the Unreleased section and opens a new one", () => {
    const before = "# Changelog\n\n## Unreleased\n\n### Fixed\n- a bug\n\n## 1.0.0 (2026-11-15)\n- first\n";
    expect(dateChangelog(before, "1.0.1", "2026-12-01")).toBe(
      "# Changelog\n\n## Unreleased\n\n## 1.0.1 (2026-12-01)\n\n### Fixed\n- a bug\n\n## 1.0.0 (2026-11-15)\n- first\n",
    );
    expect(() => dateChangelog("# C\n\n## Unreleased\n\n## 1.0.0\n- x\n", "1.0.1", "d")).toThrow(/nothing is written/);
    expect(() => dateChangelog("# C\n\n## 1.0.0\n", "1.0.1", "d")).toThrow(/no "## Unreleased"/);
    expect(dateChangelog("## Unreleased\n- only\n", "1.0.1", "d")).toBe("## Unreleased\n\n## 1.0.1 (d)\n- only\n");
  });
});
