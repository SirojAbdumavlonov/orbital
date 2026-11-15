import { Fingerprint, Trash2 } from "lucide-react";
import { useEffect, useState } from "react";
import { confirm } from "../components/ui";
import { api, asApiError, KnownHost, PresenceStatus } from "../lib/api";
import { useStore } from "../store";

export function KnownHostsView() {
  const [hosts, setHosts] = useState<KnownHost[]>([]);
  const load = () => api.knownHosts().then(setHosts);
  useEffect(() => { load(); }, []);
  const toast = useStore((x) => x.toast);
  const [gate, setGate] = useState<PresenceStatus | null>(null);
  const [busy, setBusy] = useState(false);
  useEffect(() => { api.presenceStatus().then(setGate).catch(() => {}); }, []);
  async function toggleGate(on: boolean) {
    setBusy(true);
    try {
      // The engine asks for Touch ID / the login password before changing this, in either direction.
      setGate(await api.setPresenceGate(on));
    } catch (e) {
      toast({ kind: "error", title: on ? "Couldn't turn on Touch ID" : "Touch ID is still required", body: asApiError(e).message });
    } finally {
      setBusy(false);
    }
  }
  return (
    <div className="view">
      {gate && (
        <div className="toolbar" style={{ alignItems: "flex-start", gap: 12, height: "auto", padding: "12px 16px" }}>
          <Fingerprint size={20} className="muted" style={{ flexShrink: 0, marginTop: 2 }} />
          <label className="check" style={{ flex: 1 }}>
            <input type="checkbox" checked={gate.enabled} disabled={busy || (!gate.available && !gate.enabled)} onChange={(e) => toggleGate(e.target.checked)} />
            <span>
              Require Touch ID before using saved credentials
              <br />
              <span className="faint" style={{ fontSize: 12 }}>
                {gate.available
                  ? "Orbital asks for Touch ID or your Mac password each time it connects with a password, passphrase or sudo password it has saved."
                  : "This Mac has no Touch ID or login password that Orbital can ask for."}
              </span>
            </span>
          </label>
        </div>
      )}
      <div className="toolbar"><span className="muted">Servers whose identity you've confirmed. Orbital also trusts keys already in <span className="mono">~/.ssh/known_hosts</span>.</span></div>
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
