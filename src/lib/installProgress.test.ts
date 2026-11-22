import { describe, expect, it } from "vitest";
import { installPercent, isProgressLine } from "./installProgress";

describe("installPercent", () => {
  it("follows apt through update, download and install", () => {
    const lines = ["@@progress 2", "Hit:1 http://archive.ubuntu.com noble InRelease", "@@progress 10"];
    expect(installPercent(lines)).toBe(10);
    expect(installPercent([...lines, "dlstatus:1:50:Retrieving file 3 of 6"])).toBe(30);
    expect(installPercent([...lines, "dlstatus:1:100:Retrieving", "pmstatus:docker.io:25.0000:Unpacking docker.io"])).toBe(60);
    expect(installPercent([...lines, "pmstatus:x:100:Installed x", "@@progress 92", "@@progress 96"])).toBe(96);
  });

  it("follows dnf and dnf5 package counts", () => {
    expect(installPercent(["@@progress 10", "(3/12): moby-engine-24.0.x86_64.rpm  3.1 MB/s"])).toBe(20);
    expect(installPercent(["@@progress 10", "  Installing       : containerd-1.7-1.fc40.x86_64     6/12"])).toBe(70);
    expect(installPercent(["@@progress 10", "[ 9/12] Installing nginx-1.26 100% |"])).toBe(80);
  });

  it("never goes back and ignores other output", () => {
    expect(installPercent(["@@progress 92", "dlstatus:1:5:late line", "Setting up nginx (1.24) ..."])).toBe(92);
    expect(installPercent([])).toBe(0);
    expect(installPercent(["@@progress 400"])).toBe(100);
  });
});

it("hides the machine lines from the output", () => {
  expect(isProgressLine("pmstatus:x:5:y")).toBe(true);
  expect(isProgressLine("@@progress 10")).toBe(true);
  expect(isProgressLine("Setting up nginx")).toBe(false);
});
