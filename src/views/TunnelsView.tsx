import { ArrowRight, Cable, Copy, Plug, X } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { api, asApiError, ListeningPort, TunnelInfo } from "../lib/api";
import { bytes } from "../lib/format";
import { SessionState, useStore } from "../store";

/** Well-known ports, so suggestions can say what they are and how to use them. */
export const KNOWN_PORTS: Record<number, { name: string; hint: string; web?: boolean }> = {
  5432: { name: "PostgreSQL", hint: "Connect TablePlus, DBeaver or psql to" },
  3306: { name: "MySQL", hint: "Connect TablePlus, Sequel Ace or mysql to" },
  6379: { name: "Redis", hint: "Connect RedisInsight or redis-cli to" },
  27017: { name: "MongoDB", hint: "Connect Compass or mongosh to" },
  9200: { name: "Elasticsearch", hint: "Open", web: true },
  15672: { name: "RabbitMQ admin", hint: "Open", web: true },
  3000: { name: "Web app / Grafana", hint: "Open", web: true },
  8080: { name: "Web app", hint: "Open", web: true },
  8000: { name: "Web app", hint: "Open", web: true },
  9090: { name: "Prometheus", hint: "Open", web: true },
  5601: { name: "Kibana", hint: "Open", web: true },
  8025: { name: "Mailpit / MailHog", hint: "Open", web: true },
};

const address = (t: TunnelInfo) => (KNOWN_PORTS[t.remotePort]?.web ? `http://localhost:${t.localPort}` : `127.0.0.1:${t.localPort}`);

function copy(text: string) {
  navigator.clipboard?.writeText(text).catch(() => {});
}

/** Open a tunnel and tell the user where it is. Shared with the Open ports tab. */
export async function forward(s: SessionState, remotePort: number, remoteHost = "localhost", localPort: number | null = null) {
  const toast = useStore.getState().toast;
  const known = KNOWN_PORTS[remotePort];
  try {
    const t = await api.openTunnel(s.info.id, remoteHost, remotePort, localPort, known?.name ?? null);
    const at = address(t);
    copy(at);
    toast({ kind: "success", title: `${known?.name ?? `Port ${remotePort}`} is on your Mac at ${at}`, body: `Copied to the clipboard. It stays open until you close it or disconnect.${t.localPort !== remotePort ? ` (${remotePort} was taken on this Mac, so ${t.localPort} is used.)` : ""}` });
    return t;
  } catch (e) {
    toast({ kind: "error", title: "Couldn't open the tunnel", body: asApiError(e).message });
    return null;
  }
}

/** Ports on the server, available on this Mac through the SSH connection. */
export function TunnelsView({ s }: { s: SessionState }) {
  const toast = useStore((x) => x.toast);
  const sid = s.info.id;
  const [tunnels, setTunnels] = useState<TunnelInfo[]>([]);
  const [ports, setPorts] = useState<ListeningPort[]>([]);
  const [remotePort, setRemotePort] = useState("");
  const [remoteHost, setRemoteHost] = useState("localhost");
  const [localPort, setLocalPort] = useState("");
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let stop = false;
    const tick = () => api.listTunnels().then((t) => !stop && setTunnels(t.filter((x) => x.sessionId === sid))).catch(() => {});
    tick();
    const timer = setInterval(tick, 2000);
    api.listeningPorts(sid).then((r) => setPorts(r.ports)).catch(() => {});
    return () => { stop = true; clearInterval(timer); };
  }, [sid]);

  // Ports the server listens on, deduplicated, known ones first.
  const suggestions = useMemo(() => {
    const seen = new Set<number>();
    return ports
      .filter((p) => p.protocol === "tcp" && p.port !== 22 && !seen.has(p.port) && seen.add(p.port))
      .filter((p) => !tunnels.some((t) => t.remotePort === p.port))
      .sort((a, b) => Number(!KNOWN_PORTS[a.port]) - Number(!KNOWN_PORTS[b.port]) || a.port - b.port)
      .slice(0, 12);
  }, [ports, tunnels]);

  async function open(port: number, host = "localhost", local: number | null = null) {
    setBusy(true);
    const t = await forward(s, port, host, local);
    if (t) setTunnels((x) => [...x, t]);
    setBusy(false);
  }

  async function close(t: TunnelInfo) {
    await api.closeTunnel(t.id).catch((e) => toast({ kind: "error", title: "Couldn't close the tunnel", body: asApiError(e).message }));
    setTunnels((x) => x.filter((y) => y.id !== t.id));
  }

  return (
    <div className="view">
      <div className="scroll">
        <div className="section-title">Open on this Mac</div>
        {tunnels.length === 0 ? (
          <div className="empty" style={{ flex: "none", padding: 18 }}><div><Cable size={24} className="faint" /><p>No tunnels yet. Pick a port below to use it from apps on your Mac, as if it ran locally.</p></div></div>
        ) : (
          <table className="table">
            <thead><tr><th>On your Mac</th><th /><th>On the server</th><th className="num">Connections</th><th className="num">Traffic</th><th /></tr></thead>
            <tbody>
              {tunnels.map((t) => (
                <tr key={t.id}>
                  <td><b className="mono">{address(t)}</b>{t.label && <div className="faint" style={{ fontSize: 12 }}>{KNOWN_PORTS[t.remotePort]?.hint ?? "Use"} this address</div>}</td>
                  <td style={{ width: 20 }}><ArrowRight size={14} className="faint" /></td>
                  <td className="mono muted">{t.remoteHost}:{t.remotePort}{t.label && <div className="faint" style={{ fontSize: 12, fontFamily: "var(--sans)" }}>{t.label}</div>}</td>
                  <td className="num">{t.openConnections} open <span className="faint">/ {t.totalConnections}</span></td>
                  <td className="num muted">{bytes(t.bytes)}</td>
                  <td className="nowrap" style={{ textAlign: "right" }}>
                    <button className="icon-btn" aria-label="Copy address" title="Copy address" onClick={() => { copy(address(t)); toast({ kind: "info", title: `Copied ${address(t)}` }); }}><Copy size={13} /></button>
                    <button className="icon-btn" aria-label="Close tunnel" title="Close" onClick={() => close(t)}><X size={14} /></button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}

        {suggestions.length > 0 && <>
          <div className="section-title">Ports this server listens on</div>
          <div className="chips" style={{ padding: "0 12px 4px" }}>
            {suggestions.map((p) => (
              <button key={p.port} className="chip" disabled={busy} onClick={() => open(p.port)} title={`${p.process ?? "unknown program"} on ${p.address}`}>
                <Plug size={11} style={{ verticalAlign: -1 }} /> {KNOWN_PORTS[p.port]?.name ?? p.process ?? "port"} {p.port}
              </button>
            ))}
          </div>
        </>}

        <div className="section-title">Forward any port</div>
        <div className="tunnel-form">
          <label className="field"><span>Port on the server</span><input className="input mono" inputMode="numeric" placeholder="5432" value={remotePort} onChange={(e) => setRemotePort(e.target.value.replace(/\D/g, ""))} /></label>
          <label className="field"><span>Host, as the server sees it</span><input className="input mono" value={remoteHost} onChange={(e) => setRemoteHost(e.target.value)} /></label>
          <label className="field"><span>Port on this Mac</span><input className="input mono" inputMode="numeric" placeholder="same, if free" value={localPort} onChange={(e) => setLocalPort(e.target.value.replace(/\D/g, ""))} /></label>
          <button className="btn primary" disabled={busy || !remotePort} onClick={() => open(Number(remotePort), remoteHost.trim() || "localhost", localPort ? Number(localPort) : null)}><Cable size={13} /> Open tunnel</button>
        </div>
        <p className="footnote">
          A tunnel listens only on 127.0.0.1, so other computers on your network can't use it, and it travels inside the SSH connection: nothing is opened in the server's firewall.
          "Host as the server sees it" can also be another machine on the server's network, for example a database server only it can reach.
        </p>
      </div>
    </div>
  );
}
