import { expect, it } from "vitest";
import { sshCommand } from "./sshCommand";

const base = { authMethod: "password", keyPath: null, jumpProfileId: null, protocol: "sftp", port: 22 } as any;
const web = { ...base, id: "w", name: "web", host: "203.0.113.10", username: "deploy" };
const bastion = { ...base, id: "b", name: "bastion", host: "198.51.100.2", username: "ops", port: 2222 };
const edge = { ...base, id: "e", name: "edge", host: "198.51.100.1", username: "ops", jumpProfileId: null };

it("builds the ssh command with port, key and jump hosts", () => {
  expect(sshCommand(web, [web])).toBe("ssh deploy@203.0.113.10");
  expect(sshCommand({ ...web, port: 2200, authMethod: "key", keyPath: "~/.ssh/id ed25519" }, [])).toBe("ssh -p 2200 -i '~/.ssh/id ed25519' deploy@203.0.113.10");
  const inner = { ...web, jumpProfileId: "b" };
  const b2 = { ...bastion, jumpProfileId: "e" };
  expect(sshCommand(inner, [inner, b2, edge])).toBe("ssh -J ops@198.51.100.1,ops@198.51.100.2:2222 deploy@203.0.113.10");
  expect(sshCommand({ ...web, protocol: "s3" }, [])).toBeNull();
  // A loop in the route stops instead of hanging.
  const a = { ...web, id: "a", jumpProfileId: "c" };
  const c = { ...bastion, id: "c", jumpProfileId: "a" };
  expect(sshCommand(a, [a, c])).toContain("-J");
});
