import { describe, expect, it } from "vitest";
import { issueUrl, REPO_URL } from "./links";

describe("issueUrl", () => {
  it("fills in a new issue with the message and version, never a contact", () => {
    const u = new URL(issueUrl("problem", "Docker", "  logs & stuff #1\n", "1.4.1", "macOS"));
    expect(u.origin + u.pathname).toBe(`${REPO_URL}/issues/new`);
    expect(u.searchParams.get("title")).toBe("Problem: Docker");
    expect(u.searchParams.get("body")).toBe("logs & stuff #1\n\n---\nOrbital 1.4.1 · macOS");
    expect(new URL(issueUrl("praise", "", "x", "1", "Linux")).searchParams.get("title")).toBe("Feedback");
  });
});
