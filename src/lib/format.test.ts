import { describe, expect, it } from "vitest";
import { breadcrumbs, bytes, duration, parentPath, joinPath, relativeTime, octal } from "./format";
import { languageFor } from "./language";

describe("format", () => {
  it("bytes", () => {
    expect(bytes(0)).toBe("0 B");
    expect(bytes(1536)).toBe("1.5 KB");
    expect(bytes(8_143_288 * 1024)).toBe("7.8 GB");
    expect(bytes(null)).toBe("—");
  });
  it("duration", () => {
    expect(duration(864_123)).toBe("10d 0h");
    expect(duration(3_700)).toBe("1h 1m");
    expect(duration(59)).toBe("0m");
  });
  it("paths", () => {
    expect(parentPath("/etc/nginx/nginx.conf")).toBe("/etc/nginx");
    expect(parentPath("/etc")).toBe("/");
    expect(joinPath("/", "etc")).toBe("/etc");
    expect(breadcrumbs("/etc/nginx").map((b) => b.path)).toEqual(["/", "/etc", "/etc/nginx"]);
    expect(octal(0o644)).toBe("644");
  });
  it("relative time", () => {
    const now = 1_700_000_000_000;
    expect(relativeTime(now / 1000 - 30, now)).toBe("just now");
    expect(relativeTime(now / 1000 - 7200, now)).toBe("2 h ago");
  });
});

describe("languageFor", () => {
  it("detects common server files", () => {
    expect(languageFor("/etc/nginx/sites-enabled/shop")).toBe("nginx");
    expect(languageFor("/etc/nginx/nginx.conf")).toBe("nginx");
    expect(languageFor("/srv/app/docker-compose.prod.yml")).toBe("yaml");
    expect(languageFor("/srv/app/Dockerfile.dev")).toBe("dockerfile");
    expect(languageFor("/etc/systemd/system/app.service")).toBe("ini");
    expect(languageFor("/srv/app/.env.production")).toBe("ini");
    expect(languageFor("/home/u/app/Main.java")).toBe("java");
    expect(languageFor("/var/log/syslog")).toBe("plaintext");
  });
});
