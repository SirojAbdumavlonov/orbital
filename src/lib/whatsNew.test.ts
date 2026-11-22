import changelog from "../../CHANGELOG.md?raw";
import pkgJson from "../../package.json";
import { describe, expect, it } from "vitest";
import { announcement, anchor, compareVersions, currentRelease, firstSentence, parseChangelog, releasesSince, shouldAnnounce } from "./whatsNew";

const MD = `# Changelog

Intro text.

## Unreleased

## 1.4.0 (2026-11-22)

### Added
- **Test a website** (Nginx tab → a site → **Test**): nginx's own check (\`nginx -t\`), a request on the server. A failure says where to look.
- **Delete a website**: removes its nginx file.

### Fixed
- **Reconnect keeps your place.** The message now says so.

### Added
- **Adjust a container** before moving it:
  - **ports**: publish each one;
  - **arguments** after the image.

  The preview checks the target again.

## 1.3.9 (2026-11-21)

### Changed
- Switching tabs keeps your place in Docker, Services, Nginx, Users, Cron and more.

## 0.1.0
- First version.
`;

describe("parseChangelog", () => {
  const all = parseChangelog(MD);

  it("reads releases newest first and skips an empty Unreleased", () => {
    expect(all.map((r) => r.version)).toEqual(["1.4.0", "1.3.9", "0.1.0"]);
    expect(all[0].date).toBe("2026-11-22");
    expect(all[2].date).toBeUndefined();
  });

  it("keeps repeated sections and their kinds in order", () => {
    expect(all[0].items.map((i) => [i.kind, i.title])).toEqual([
      ["added", "Test a website"],
      ["added", "Delete a website"],
      ["fixed", "Reconnect keeps your place"],
      ["added", "Adjust a container"],
    ]);
  });

  it("summarises with the first sentence, without markdown or the leading place", () => {
    expect(all[0].items[0].summary).toBe("nginx's own check (nginx -t), a request on the server.");
    expect(all[0].items[2].summary).toBe("The message now says so.");
  });

  it("keeps sub-points and later paragraphs as details", () => {
    expect(all[0].items[3].details).toEqual(["before moving it:", "• ports: publish each one;", "• arguments after the image.", "The preview checks the target again."]);
    expect(all[0].items[3].summary).toBe("The preview checks the target again.");
  });

  it("titles a bullet without bold from its first words", () => {
    expect(all[1].items[0].title).toBe("Switching tabs keeps your place in Docker, Services,…");
    expect(all[1].items[0].kind).toBe("changed");
    expect(all[2].items[0]).toMatchObject({ title: "First version", kind: "other" });
  });

  it("links each release to its heading in the changelog", () => {
    expect(all[0].url).toBe("https://github.com/SirojAbdumavlonov/orbital/blob/main/CHANGELOG.md#140-2026-11-22");
    expect(anchor("0.1.0")).toBe("010");
  });

  it("reads the real CHANGELOG: every release has titled items", () => {
    const real = parseChangelog(changelog);
    expect(real.length).toBeGreaterThan(10);
    const pkg = pkgJson.version;
    expect(currentRelease(real, pkg)?.version).toBe(pkg);
    for (const r of real) for (const i of r.items) {
      expect(i.title.length).toBeGreaterThan(2);
      expect(i.summary.length).toBeLessThanOrEqual(151);
      expect(i.title + i.summary).not.toMatch(/\*\*|`/);
    }
  });
});

describe("firstSentence", () => {
  it("doesn't end inside parentheses or at e.g.", () => {
    expect(firstSentence("Opens it (see Settings. Then more). Next one.")).toBe("Opens it (see Settings. Then more).");
    expect(firstSentence("a cloud firewall, e.g. Oracle's list, is shut. Then")).toBe("a cloud firewall, e.g. Oracle's list, is shut.");
  });
  it("cuts a long sentence at a word", () => {
    const s = firstSentence("word ".repeat(80));
    expect(s.endsWith("…")).toBe(true);
    expect(s.length).toBeLessThanOrEqual(151);
  });
});

describe("versions", () => {
  it("compares numbers and pre-releases", () => {
    expect(compareVersions("1.10.0", "1.9.9")).toBe(1);
    expect(compareVersions("1.4.0", "v1.4.0")).toBe(0);
    expect(compareVersions("1.4.0-beta.2", "1.4.0")).toBe(-1);
    expect(compareVersions("1.4.0-beta.10", "1.4.0-beta.2")).toBe(1);
  });

  it("announces only after an update", () => {
    const all = parseChangelog(MD);
    expect(shouldAnnounce(null, "1.4.0", all)).toBe(false);
    expect(shouldAnnounce("1.4.0", "1.4.0", all)).toBe(false);
    expect(shouldAnnounce("1.4.0", "1.3.9", all)).toBe(false);
    expect(shouldAnnounce("1.3.8", "1.4.0", all)).toBe(true);
    expect(releasesSince(all, "1.3.8", "1.4.0").map((r) => r.version)).toEqual(["1.4.0", "1.3.9"]);
    expect(releasesSince(all, "1.3.9", "1.3.9")).toEqual([]);
  });

  it("announces after an update, and for users of a version that didn't record it", () => {
    const all = parseChangelog(MD);
    expect(announcement("1.3.8", "1.4.0", all, true)?.map((r) => r.version)).toEqual(["1.4.0", "1.3.9"]);
    expect(announcement("1.4.0", "1.4.0", all, true)).toBeNull();
    expect(announcement(null, "1.4.0", all, false)).toBeNull();
    expect(announcement(null, "1.4.0", all, true)?.map((r) => r.version)).toEqual(["1.4.0"]);
    expect(announcement(null, "1.4.1", all, true)).toBeNull();
  });
});
