import { describe, expect, it } from "vitest";
import { parseTarget } from "./TlsView";

describe("TLS inspector targets", () => {
  it("reads hosts, ports, URLs and IPv6", () => {
    expect(parseTarget("Example.com")).toEqual({ host: "example.com", port: 443 });
    expect(parseTarget("mail.example.com:993")).toEqual({ host: "mail.example.com", port: 993 });
    expect(parseTarget("https://example.com/shop?x=1")).toEqual({ host: "example.com", port: 443 });
    expect(parseTarget("imaps://mail.example.com")).toEqual({ host: "mail.example.com", port: 993 });
    expect(parseTarget("[2001:db8::1]:8443")).toEqual({ host: "[2001:db8::1]", port: 8443 });
    expect(parseTarget("example.com:99999")).toBeNull();
    expect(parseTarget("  ")).toBeNull();
  });
});
