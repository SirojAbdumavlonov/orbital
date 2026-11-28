import { describe, expect, it } from "vitest";
import { remember, resolvePath, splitForSuggest, suggest } from "./goto";

describe("go to folder", () => {
  it("resolves what was typed", () => {
    expect(resolvePath("/etc/nginx/", "/home/a", "/home/a")).toBe("/etc/nginx");
    expect(resolvePath("~", "/srv", "/home/a")).toBe("/home/a");
    expect(resolvePath("~/apps/../logs", "/srv", "/home/a")).toBe("/home/a/logs");
    expect(resolvePath("sites-enabled", "/etc/nginx", "/root")).toBe("/etc/nginx/sites-enabled");
    expect(resolvePath("../..", "/var/log/nginx", "/root")).toBe("/var");
    expect(resolvePath("//var///www/./html", "/", "/root")).toBe("/var/www/html");
    expect(resolvePath("/..", "/", "/root")).toBe("/");
    expect(resolvePath("  ", "/tmp", "/root")).toBe("/tmp");
  });
  it("splits for suggestions", () => {
    expect(splitForSuggest("/etc/ngi", "/", "/root")).toEqual({ dir: "/etc", prefix: "ngi" });
    expect(splitForSuggest("/etc/", "/", "/root")).toEqual({ dir: "/etc", prefix: "" });
    expect(splitForSuggest("/e", "/", "/root")).toEqual({ dir: "/", prefix: "e" });
    expect(splitForSuggest("~/ap", "/", "/home/a")).toEqual({ dir: "/home/a", prefix: "ap" });
    expect(splitForSuggest("lo", "/var", "/root")).toEqual({ dir: "/var", prefix: "lo" });
  });
  it("suggests matching folders, hidden ones only when asked", () => {
    const names = ["nginx", "NetworkManager", "network", ".ssh", "nftables"];
    expect(suggest(names, "n")).toEqual(["network", "nftables", "nginx", "NetworkManager"]);
    expect(suggest(names, "N")).toEqual(["NetworkManager", "network", "nftables", "nginx"]);
    expect(suggest(names, ".s")).toEqual([".ssh"]);
    expect(suggest(names, "")).not.toContain(".ssh");
  });
  it("remembers recent folders", () => {
    expect(remember(["/a", "/b"], "/b")).toEqual(["/b", "/a"]);
    expect(remember(["/1", "/2", "/3"], "/4", 3)).toEqual(["/4", "/1", "/2"]);
  });
});
