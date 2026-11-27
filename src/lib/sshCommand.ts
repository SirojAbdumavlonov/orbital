import { Profile } from "./api";

// The `ssh` command that reaches a saved server, for a terminal outside
// Orbital: port, key file and the jump-host route (-J) included. Pure; tested.

const q = (s: string) => (/^[A-Za-z0-9_.@:,\/~+-]+$/.test(s) ? s : `'${s.replace(/'/g, `'\\''`)}'`);

const hop = (p: Profile) => `${p.username}@${p.host.includes(":") ? `[${p.host}]` : p.host}${p.port && p.port !== 22 ? `:${p.port}` : ""}`;

export function sshCommand(p: Profile, all: Profile[]): string | null {
  if ((p.protocol ?? "sftp") !== "sftp") return null;
  const parts = ["ssh"];
  if (p.port && p.port !== 22) parts.push("-p", String(p.port));
  if (p.authMethod === "key" && p.keyPath) parts.push("-i", q(p.keyPath));
  // The route, outermost first; a loop or a missing hop stops it.
  const route: Profile[] = [];
  let j = p.jumpProfileId ? all.find((x) => x.id === p.jumpProfileId) : undefined;
  while (j && !route.includes(j) && route.length < 8) {
    route.unshift(j);
    j = j.jumpProfileId ? all.find((x) => x.id === j!.jumpProfileId) : undefined;
  }
  if (route.length) parts.push("-J", q(route.map(hop).join(",")));
  parts.push(q(`${p.username}@${p.host}`));
  return parts.join(" ");
}
