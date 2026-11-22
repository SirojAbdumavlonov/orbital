import { describe, expect, it, vi } from "vitest";
vi.mock("./ui", () => ({ confirm: async () => true, Modal: () => null }));
const { lineDiff, routeTarget } = await import("./NginxRoutes");

describe("route preview", () => {
  it("shows only the changed lines with a little context", () => {
    const before = "server {\n    listen 80;\n    location / {\n    }\n}\n";
    const after = "server {\n    listen 80;\n    location / {\n    }\n    location /api/ {\n        proxy_pass http://127.0.0.1:4000/;\n    }\n}\n";
    expect(lineDiff(before, after)).toEqual([
      [" ", "    location / {"],
      [" ", "    }"],
      ["+", "    location /api/ {"],
      ["+", "        proxy_pass http://127.0.0.1:4000/;"],
      ["+", "    }"],
      [" ", "}"],
      [" ", ""],
    ]);
    expect(lineDiff("a\nb\nc", "a\nc").filter(([m]) => m !== " ")).toEqual([["-", "b"]]);
  });
  it("says where a route goes", () => {
    expect(routeTarget({ spec: "/api/", line: 1, summary: "", text: "", route: { path: "/api/", exact: false, target: { kind: "app", host: "127.0.0.1", port: 3000, keepPrefix: false, websockets: true } } }))
      .toBe("app on 127.0.0.1:3000 (path without the prefix) · WebSockets");
    expect(routeTarget({ spec: "~ \\.php$", line: 1, summary: "fastcgi_pass unix:/run/php.sock", text: "", route: null })).toBe("fastcgi_pass unix:/run/php.sock");
  });
});
