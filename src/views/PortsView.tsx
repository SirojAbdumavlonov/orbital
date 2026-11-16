import { Cable, RefreshCw, Search } from "lucide-react";
import { forward } from "./TunnelsView";
import { useEffect, useMemo, useState } from "react";
import { api, Container, ListeningPort } from "../lib/api";
import { useRemote } from "../lib/remote";
import { SessionState } from "../store";

type Filter = "all" | "network" | "local";

const WHERE: Record<ListeningPort["scope"], string> = {
  all: "All interfaces",
  local: "This server only",
  address: "One address",
};

function FirewallBadge({ p, firewallActive }: { p: ListeningPort; firewallActive: boolean }) {
  if (p.scope === "local") return <span className="faint">Not reachable from outside</span>;
  if (!firewallActive) return <span className="badge warn" title="No active ufw firewall: anyone who can reach the server can reach this port">No firewall</span>;
  switch (p.firewall) {
    case "open": return <span className="badge" title="ufw lets the internet reach this port">Allowed</span>;
    case "blocked": return <span className="badge ok" title="ufw blocks this port for the internet">Blocked</span>;
    case "restricted": return <span className="badge" title="ufw allows only some addresses">Some addresses</span>;
    case "docker": return <span className="badge warn" title="Docker publishes container ports with its own iptables rules, past ufw. Bind it to 127.0.0.1 in the compose file if it shouldn't be public.">Docker bypasses ufw</span>;
    default: return <span className="faint">—</span>;
  }
}

/** Every listening port, who listens on it, and whether the outside world can reach it. */
export function PortsView({ s }: { s: SessionState }) {
  const sid = s.info.id;
  const { data, error, loading, reload } = useRemote(() => api.listeningPorts(sid), [sid]);
  const [filter, setFilter] = useState<Filter>("all");
  const [q, setQ] = useState("");
  const [containers, setContainers] = useState<Container[]>([]);

  const usesDocker = !!data?.ports.some((p) => p.process?.includes("docker-proxy"));
  useEffect(() => {
    if (usesDocker && s.info.dockerVersion) api.listContainers(sid).then(setContainers).catch(() => setContainers([]));
  }, [usesDocker, sid]);

  const containerFor = (p: ListeningPort) =>
    containers.find((c) => c.ports.split(",").some((m) => m.trim().match(new RegExp(`:${p.port}->\\d+/${p.protocol}$`))))?.names;

  const rows = useMemo(() => {
    const needle = q.trim().toLowerCase();
    return (data?.ports ?? []).filter((p) => {
      if (filter === "network" && p.scope === "local") return false;
      if (filter === "local" && p.scope !== "local") return false;
      return !needle || [p.port, p.protocol, p.address, p.process, p.unit].join(" ").toLowerCase().includes(needle);
    });
  }, [data, filter, q]);

  const exposed = (data?.ports ?? []).filter((p) => p.scope !== "local");
  const unguarded = data && !data.firewallActive ? exposed.length : exposed.filter((p) => p.firewall === "open" || p.firewall === "docker").length;

  return (
    <div className="view">
      <div className="toolbar">
        <div className="segmented" role="tablist" aria-label="Show">
          {([["all", "All"], ["network", "Reachable from the network"], ["local", "This server only"]] as [Filter, string][]).map(([id, label]) => (
            <button key={id} className={filter === id ? "on" : ""} onClick={() => setFilter(id)}>{label}</button>
          ))}
        </div>
        <div className="search"><Search size={14} /><input className="input" placeholder="Port, program or service" value={q} onChange={(e) => setQ(e.target.value)} /></div>
        <span className="spacer" />
        {data && <span className="faint">{data.ports.length} open · {unguarded} reachable from the internet</span>}
        <button className="icon-btn" aria-label="Refresh" title="Look again" onClick={reload}><RefreshCw size={14} className={loading ? "spin" : ""} /></button>
      </div>
      {error && <div className="empty"><div><h3>Couldn't list open ports</h3><p>{error.message}</p><button className="btn" onClick={reload}>Try again</button></div></div>}
      {data && !data.complete && (
        <div className="notice">Some program names may be missing: seeing other users' processes needs root. Turn on sudo in this server's settings, then reconnect.</div>
      )}
      {data && (
        <div className="scroll">
          <table className="table">
            <thead><tr><th style={{ width: 80 }}>Port</th><th style={{ width: 70 }}>Protocol</th><th>Listening on</th><th>Program</th><th>Service</th><th>Firewall</th><th /></tr></thead>
            <tbody>
              {rows.map((p) => (
                <tr key={`${p.protocol}:${p.address}:${p.port}:${p.pid}`}>
                  <td className="mono num" style={{ textAlign: "left" }}><b>{p.port}</b></td>
                  <td className="mono muted">{p.protocol.toUpperCase()}</td>
                  <td><span className="mono">{p.address}</span> <span className="faint">· {WHERE[p.scope]}</span></td>
                  <td className="mono">{p.process ?? <span className="faint">hidden</span>}{p.pid ? <span className="faint"> ({p.pid})</span> : null}
                    {p.process?.includes("docker-proxy") && containerFor(p) && <div className="faint">container {containerFor(p)}</div>}
                  </td>
                  <td className="mono muted">{p.unit ?? "—"}</td>
                  <td><FirewallBadge p={p} firewallActive={data.firewallActive} /></td>
                  <td style={{ textAlign: "right" }}>{p.protocol === "tcp" && p.port !== 22 && <button className="icon-btn" aria-label={`Open port ${p.port} on this computer`} title="Open on this computer (tunnel)" onClick={() => forward(s, p.port, p.scope === "address" ? p.address : "localhost")}><Cable size={13} /></button>}</td>
                </tr>
              ))}
            </tbody>
          </table>
          {rows.length === 0 && <div className="empty" style={{ flex: "none" }}><p>{q || filter !== "all" ? "No ports match." : "Nothing is listening."}</p></div>}
          <p className="footnote">
            “All interfaces” (0.0.0.0, ::, *) means anyone who can reach the server can try this port, unless the firewall blocks it. “This server only” (127.0.0.1, ::1) is safe from outside. Read with <span className="mono">{data.source}</span>.
          </p>
        </div>
      )}
    </div>
  );
}
