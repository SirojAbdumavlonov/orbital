import { expect, it, vi } from "vitest";
vi.mock("../lib/api", () => ({ api: {}, asApiError: (e: any) => e }));
vi.mock("../views/SshKeysView", () => ({ NewKeyDialog: () => null }));
const { withKeyLine, hasKeyLine, withoutKeyLine } = await import("./LocalKeyPicker");

it("adds a key once, whatever its comment", () => {
  const a = "ssh-ed25519 AAAAC3a anna@laptop";
  expect(withKeyLine("", a)).toBe(a);
  expect(withKeyLine(a, "ssh-ed25519 AAAAC3a other-comment")).toBe(a);
  expect(withKeyLine(`${a}\n\n`, "ssh-rsa BBBB bob")).toBe(`${a}\nssh-rsa BBBB bob`);
  expect(hasKeyLine(`x y\n${a}`, a)).toBe(true);
  expect(hasKeyLine("x y", a)).toBe(false);
});

it("removes a key whatever comment it was pasted with", () => {
  expect(withoutKeyLine("ssh-ed25519 AAAAC3a pasted\nssh-rsa BBBB bob", "ssh-ed25519 AAAAC3a anna@laptop")).toBe("ssh-rsa BBBB bob");
});
