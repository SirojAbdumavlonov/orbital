import { describe, expect, it } from "vitest";
import { defaultLabel, favProblem } from "./favourites";

describe("favourites", () => {
  it("checks what a favourite points at", () => {
    expect(favProblem("restart", "nginx")).toBeNull();
    expect(favProblem("restart", "docker.service")).toBeNull();
    expect(favProblem("restart", "nginx; rm -rf /")).toMatch(/letters/);
    expect(favProblem("restart", "a\\b")).toMatch(/letters/);
    expect(favProblem("file", "etc/hosts")).toMatch(/start with \//);
    expect(favProblem("folder", "/var/log")).toBeNull();
    expect(favProblem("snippet", "")).toMatch(/choose/);
    expect(favProblem("container", "shop-api")).toBeNull();
    expect(favProblem("container", "a;b")).toMatch(/letters/);
  });
  it("makes short default labels", () => {
    expect(defaultLabel("restart", "nginx.service")).toBe("Restart nginx");
    expect(defaultLabel("reload", "php8.3-fpm")).toBe("Reload php8.3-fpm");
    expect(defaultLabel("container", "shop-api")).toBe("Restart shop-api");
    expect(defaultLabel("file", "/etc/nginx/nginx.conf")).toBe("nginx.conf");
    expect(defaultLabel("folder", "/var/log/")).toBe("log/");
    expect(defaultLabel("folder", "/")).toBe("/");
    expect(defaultLabel("snippet", "s1", "Clear cache")).toBe("Clear cache");
  });
});
