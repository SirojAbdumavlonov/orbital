import { describe, expect, it } from "vitest";
import { publishedPorts } from "./ports";

describe("publishedPorts", () => {
  it("finds host ports, server-only ones first", () => {
    expect(publishedPorts("0.0.0.0:443->443/tcp, :::443->443/tcp, 127.0.0.1:8080->8080/tcp, 5432/tcp")).toEqual([
      { host: "127.0.0.1", port: 8080, localOnly: true },
      { host: "0.0.0.0", port: 443, localOnly: false },
    ]);
  });
  it("handles [::1], bare host ports and nothing published", () => {
    expect(publishedPorts("[::1]:9000->9000/tcp")).toEqual([{ host: "::1", port: 9000, localOnly: true }]);
    expect(publishedPorts("3000->3000/tcp")).toEqual([{ host: "0.0.0.0", port: 3000, localOnly: false }]);
    expect(publishedPorts("80/tcp")).toEqual([]);
    expect(publishedPorts("")).toEqual([]);
  });
});
