import { Trash2 } from "lucide-react";
import { useEffect, useState } from "react";
import { confirm } from "../components/ui";
import { api, KnownHost } from "../lib/api";

export function KnownHostsView() {
  const [hosts, setHosts] = useState<KnownHost[]>([]);
  const load = () => api.knownHosts().then(setHosts);
  useEffect(() => { load(); }, []);
  return (
    <div className="view">
      <div className="toolbar"><span className="muted">Servers whose identity you've confirmed. Outpost also trusts keys already in <span className="mono">~/.ssh/known_hosts</span>.</span></div>
      <div className="scroll">
        <table className="table">
          <thead><tr><th>Host</th><th>Key type</th><th>Fingerprint</th><th>Trusted on</th><th /></tr></thead>
          <tbody>
            {hosts.map((h) => (
              <tr key={`${h.host}:${h.port}:${h.algorithm}`}>
                <td className="mono">{h.host}{h.port !== 22 ? `:${h.port}` : ""}</td>
                <td className="faint">{h.algorithm}</td>
                <td className="mono selectable" style={{ fontSize: 12 }}>{h.fingerprint}</td>
                <td className="faint">{new Date(h.addedAt).toLocaleDateString()}</td>
                <td>
                  <button className="icon-btn" aria-label={`Forget ${h.host}`} onClick={async () => {
                    if (await confirm({ title: `Forget ${h.host}?`, body: "You'll be asked to verify its fingerprint again on the next connection.", confirmLabel: "Forget host key", danger: true })) {
                      await api.forgetHost(h.host, h.port);
                      load();
                    }
                  }}><Trash2 size={14} /></button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
        {hosts.length === 0 && <div className="empty"><p>No host keys trusted yet.</p></div>}
      </div>
    </div>
  );
}
