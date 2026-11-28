import { expect, it, vi } from "vitest";
vi.mock("../lib/api", () => ({ api: {}, asApiError: (e: any) => e }));
vi.mock("../store", () => ({ isSsh: () => true, useStore: () => null }));
vi.mock("../components/ui", () => ({ confirm: async () => true, Modal: () => null }));
const { freeKeyName } = await import("./SshKeysView");

it("picks a key file name that isn't taken", () => {
  expect(freeKeyName("id_ed25519_anna", [])).toBe("id_ed25519_anna");
  expect(freeKeyName("id_ed25519", ["id_ed25519", "id_ed25519_2"])).toBe("id_ed25519_3");
});
