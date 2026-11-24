import { expect, it, vi } from "vitest";
vi.mock("../components/ui", () => ({ Modal: () => null }));
vi.mock("../store", () => ({}));
const { unitName, previewUnit } = await import("./NewService");

it("names units and previews them without secret values", () => {
  expect(unitName(" Shop API.service ")).toBe("shop-api");
  const p = previewUnit({ name: "shop-api", description: "", command: 'echo "$HOME" 100%', workdir: "/srv/shop", user: "deploy", restart: "onFailure", env: [["TOKEN", "secret"]] });
  expect(p).toContain('ExecStart=/bin/sh -c "echo \\"$$HOME\\" 100%%"');
  expect(p).toContain("EnvironmentFile=/etc/orbital/env/shop-api.env");
  expect(p).not.toContain("secret");
  expect(p).toContain("Restart=on-failure");
});
