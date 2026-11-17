import { describe, expect, it } from "vitest";
import type { CertbotStatus, Metrics, SshdReport, SystemInfo, UfwStatus } from "./api";
import { checkup } from "./checkup";

const GB = 1024 ** 3;
const metrics = (over: Partial<Metrics> = {}): Metrics =>
  ({
    cores: 4,
    load: [0.5, 0.4, 0.3],
    memTotal: 8 * GB,
    memAvailable: 5 * GB,
    swapTotal: 0,
    disks: [{ filesystem: "/dev/sda1", fsType: "ext4", mount: "/", totalBytes: 100 * GB, usedBytes: 40 * GB, availableBytes: 60 * GB, usedPercent: 40 }],
    ...over,
  }) as unknown as Metrics;
const info = (over: Partial<SystemInfo> = {}): SystemInfo => ({ failedUnits: [], updates: 0, securityUpdates: 0, rebootRequired: false, ...over }) as unknown as SystemInfo;
const ufw = (active: boolean): UfwStatus => ({ active, defaultIncoming: "deny", defaultOutgoing: "allow", logging: null, rules: [], added: [], apps: [] });

describe("checkup", () => {
  it("a healthy server is all ok", () => {
    const f = checkup({ info: info(), metrics: metrics(), sshd: null, ufw: ufw(true), certs: null });
    expect(f.every((x) => x.level === "ok")).toBe(true);
    expect(f.map((x) => x.title)).toContain("Up to date");
  });

  it("finds the problems, worst first, each with where to fix it", () => {
    const f = checkup({
      info: info({ failedUnits: ["shop.service"], updates: 12, securityUpdates: 3, rebootRequired: true }),
      metrics: metrics({ memAvailable: 0.5 * GB, disks: [{ filesystem: "/dev/sda1", fsType: "ext4", mount: "/", totalBytes: 100 * GB, usedBytes: 93 * GB, availableBytes: 7 * GB, usedPercent: 93 }] } as Partial<Metrics>),
      sshd: { readable: true, checks: [{ id: "root", title: "Root can sign in with a password", level: "bad", setting: "PermitRootLogin", value: "yes", advice: "Set it to prohibit-password" }] } as unknown as SshdReport,
      ufw: "absent",
      certs: { certificates: [{ name: "shop.example.com", domains: [], expires: null, daysLeft: 3, valid: true, keyType: null, certificatePath: null, privateKeyPath: null, status: "" }], autoRenew: { via: "certbot.timer", active: false, nextRun: null }, error: null } as unknown as CertbotStatus,
    });
    const bad = f.filter((x) => x.level === "bad").map((x) => x.title);
    expect(bad).toEqual(expect.arrayContaining(["Disk / is 93% full", "1 service has failed", "3 security updates waiting", "SSH: Root can sign in with a password", "Certificate shop.example.com expires in 3 days"]));
    expect(f.findIndex((x) => x.level !== "bad")).toBeGreaterThan(bad.length - 1);
    expect(f.find((x) => x.title === "1 service has failed")?.go?.tab).toBe("services");
    expect(f.map((x) => x.title)).toEqual(expect.arrayContaining(["A reboot is needed", "No firewall found", "Certificates don't renew automatically"]));
    expect(f.find((x) => x.title.startsWith("Only "))?.level).toBe("bad");
  });

  it("missing data is skipped, not reported as a problem", () => {
    expect(checkup({ info: null, metrics: null, sshd: null, ufw: null, certs: null })).toEqual([]);
  });
});
