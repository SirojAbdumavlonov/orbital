// Ports Docker publishes on the server, for one-click port forwarding.
/**
 * Host ports a container publishes (`127.0.0.1:8080->8080/tcp, 0.0.0.0:443->443/tcp`).
 * Those bound to 127.0.0.1 are reachable only on the server itself, so a
 * tunnel is the way to use them from here.
 */
export function publishedPorts(ports: string): { host: string; port: number; localOnly: boolean }[] {
  const out: { host: string; port: number; localOnly: boolean }[] = [];
  for (const m of ports.matchAll(/(?:(\d+\.\d+\.\d+\.\d+|\[?[0-9a-f:]*\]?):)?(\d+)->\d+\/tcp/gi)) {
    const host = (m[1] ?? "0.0.0.0").replace(/[[\]]/g, "");
    const port = Number(m[2]);
    if (out.some((p) => p.port === port)) continue;
    out.push({ host, port, localOnly: host === "127.0.0.1" || host === "::1" });
  }
  return out.sort((a, b) => Number(b.localOnly) - Number(a.localOnly) || a.port - b.port);
}
