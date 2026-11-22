import { describe, expect, it } from "vitest";
import { DnsCheck, ResolverAnswer } from "./api";
import { readiness } from "./dnsReady";

const ans = (resolver: string, status: string, values: string[], error: string | null = null): ResolverAnswer => ({ resolver, status, values, ok: false, error });
const check = (answers: ResolverAnswer[]): DnsCheck => ({ name: "stats.example.com", type: "A", expected: ["198.51.100.7"], answers, verified: false, checkedAt: "" });

describe("readiness", () => {
  it("points here when one resolver confirms and the other hasn't caught up", () => {
    expect(readiness(check([ans("Cloudflare", "NOERROR", ["198.51.100.7"]), ans("Google", "ERROR", [], "timed out")]))).toEqual({ state: "here", lagging: ["Google"] });
    expect(readiness(check([ans("Cloudflare", "NOERROR", ["198.51.100.7"]), ans("Google", "NXDOMAIN", [])]))).toEqual({ state: "here", lagging: ["Google"] });
    expect(readiness(check([ans("Cloudflare", "NOERROR", ["198.51.100.7"]), ans("Google", "NOERROR", ["198.51.100.7"])]))).toEqual({ state: "here", lagging: [] });
  });
  it("says elsewhere only for a different address", () => {
    expect(readiness(check([ans("Cloudflare", "NOERROR", ["198.51.100.7"]), ans("Google", "NOERROR", ["203.0.113.9"])]))).toEqual({ state: "elsewhere", others: ["203.0.113.9"] });
  });
  it("no record, or nobody could be asked", () => {
    expect(readiness(check([ans("Cloudflare", "NXDOMAIN", []), ans("Google", "NXDOMAIN", [])]))).toEqual({ state: "none" });
    expect(readiness(check([ans("Cloudflare", "ERROR", [], "offline"), ans("Google", "ERROR", [], "offline")]))).toEqual({ state: "unknown", error: "offline" });
  });
});
