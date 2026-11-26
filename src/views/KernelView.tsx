import { CheckCircle2, Cpu, Plus, RefreshCw, Trash2, Undo2 } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { confirm } from "../components/ui";
import { api, asApiError, KernelReport, KernelSetting } from "../lib/api";
import { runJob } from "../lib/jobs";
import { useKept } from "../lib/kept";
import { SessionState } from "../store";

const CMP = { eq: "", atLeast: "at least ", atMost: "at most " };

/** The values Orbital's file would hold after the user's edits. */
function fileValues(r: KernelReport): Record<string, string> {
  const v: Record<string, string> = { ...r.custom };
  for (const s of r.settings) if (s.ours != null) v[s.key] = s.ours;
  return v;
}

/**
 * Kernel tuning: well-known sysctl settings with their value now and a
 * recommendation. Chosen values go into one file Orbital owns
 * (/etc/sysctl.d/90-orbital.conf); if the kernel refuses one, the previous
 * file is put back.
 */
export function KernelView({ s }: { s: SessionState }) {
  const sid = s.info.id;
  const [r, setR] = useState<KernelReport | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [busy, setBusy] = useState(false);
  const [draft, setDraft] = useKept<Record<string, string> | null>(`${sid}:kernel:draft`, null);
  const [newKey, setNewKey] = useState("");
  const [newValue, setNewValue] = useState("");
  const tools = new Set((s.tools ?? []).map((t) => t.id));

  const load = async () => {
    setLoading(true);
    try {
      const rep = await api.kernelReport(sid);
      setR(rep);
      setError(null);
    } catch (e) {
      setError(asApiError(e).message);
    } finally {
      setLoading(false);
    }
  };
  useEffect(() => { void load(); }, [sid]);

  const saved = useMemo(() => (r ? fileValues(r) : {}), [r]);
  const values = draft ?? saved;
  const dirty = JSON.stringify(Object.entries(values).sort()) !== JSON.stringify(Object.entries(saved).sort());
  const set = (k: string, v: string | null) => {
    const next = { ...values };
    if (v == null) delete next[k];
    else next[k] = v;
    setDraft(next);
  };
  const shown = (r?.settings ?? []).filter((x) => x.current != null && (!x.onlyWith || tools.has(x.onlyWith)));
  const groups = [...new Set(shown.map((x) => x.group))];
  const todo = shown.filter((x) => !x.ok && values[x.key] == null);

  async function apply(next: Record<string, string>) {
    const removing = Object.keys(saved).filter((k) => !(k in next));
    if (removing.length) {
      const ok = await confirm({ title: `Stop setting ${removing.join(", ")}?`, body: "Orbital's file no longer sets them. The kernel keeps the current values until the next reboot, then uses its defaults.", confirmLabel: "Apply" });
      if (!ok) return;
    }
    setBusy(true);
    try {
      const rep = await runJob((p) => api.kernelApply(sid, next, p), {
        key: `${sid}:kernel`, title: "Kernel settings", sid,
        done: () => (Object.keys(next).length ? `${Object.keys(next).length} settings loaded` : "Orbital's sysctl file removed"),
      });
      setR(rep);
      setDraft(null);
    } catch {
      /* the jobs layer reports it */
    } finally {
      setBusy(false);
    }
  }

  const row = (x: KernelSetting) => {
    const mine = values[x.key];
    const pending = mine != null && mine !== x.current;
    return (
      <tr key={x.key}>
        <td>
          <div className="kernel-title">{x.ok ? <CheckCircle2 size={13} className="ok-text" /> : <span className="kernel-dot" />}<b>{x.title}</b></div>
          <div className="mono faint kernel-key">{x.key}</div>
          <div className="muted kernel-why">{x.why}</div>
        </td>
        <td className="mono nowrap">{x.current}</td>
        <td className="mono nowrap faint">{CMP[x.cmp]}{x.recommended}</td>
        <td className="nowrap" style={{ textAlign: "right" }}>
          {mine != null ? (
            <>
              <input className="input mono kernel-input" value={mine} onChange={(e) => set(x.key, e.target.value)} aria-label={`Value for ${x.key}`} />
              <button className="icon-btn" title="Don't set it in Orbital's file" aria-label={`Stop setting ${x.key}`} onClick={() => set(x.key, null)}><Undo2 size={13} /></button>
              {pending && <span className="badge amber">to apply</span>}
            </>
          ) : x.ok ? <span className="faint">fine</span> : <button className="btn" onClick={() => set(x.key, x.recommended)}>Use {x.recommended}</button>}
        </td>
      </tr>
    );
  };

  return (
    <div className="view">
      <div className="toolbar view-toolbar">
        <span className="faint" style={{ fontSize: 12.5 }}>Kernel settings (sysctl) and what's recommended for a server.</span>
        <span className="spacer" />
        {todo.length > 0 && <button className="btn" disabled={busy} onClick={() => setDraft({ ...values, ...Object.fromEntries(todo.map((x) => [x.key, x.recommended])) })}>Use all {todo.length} recommendations</button>}
        {dirty && <button className="btn" disabled={busy} onClick={() => setDraft(null)}>Discard</button>}
        <button className="btn primary" disabled={!dirty || busy} onClick={() => void apply(values)}>{busy ? "Applying…" : "Apply"}</button>
        <button className="icon-btn" aria-label="Refresh" onClick={() => void load()}><RefreshCw size={14} className={loading ? "spin" : ""} /></button>
      </div>
      <div className="scroll">
        {error && <div className="alert" style={{ margin: 16 }}>{error}</div>}
        {r && groups.map((g) => (
          <div key={g}>
            <div className="section-title">{g}</div>
            <table className="table kernel-table">
              <thead><tr><th>Setting</th><th>Now</th><th>Recommended</th><th style={{ width: 230 }} /></tr></thead>
              <tbody>{shown.filter((x) => x.group === g).map(row)}</tbody>
            </table>
          </div>
        ))}
        {r && (
          <>
            <div className="section-title">Other settings in Orbital's file</div>
            <table className="table kernel-table"><tbody>
              {Object.entries(values).filter(([k]) => !r.settings.some((x) => x.key === k)).map(([k, v]) => (
                <tr key={k}>
                  <td className="mono">{k}</td>
                  <td className="mono faint">{r.customCurrent[k] ?? "—"}</td>
                  <td className="nowrap" style={{ textAlign: "right" }}>
                    <input className="input mono kernel-input" value={v} onChange={(e) => set(k, e.target.value)} aria-label={`Value for ${k}`} />
                    <button className="icon-btn" aria-label={`Remove ${k}`} onClick={() => set(k, null)}><Trash2 size={13} /></button>
                  </td>
                </tr>
              ))}
              <tr>
                <td><input className="input mono" placeholder="net.ipv4.ip_local_port_range" value={newKey} onChange={(e) => setNewKey(e.target.value.trim())} /></td>
                <td><input className="input mono" placeholder="value" value={newValue} onChange={(e) => setNewValue(e.target.value)} /></td>
                <td style={{ textAlign: "right" }}><button className="btn" disabled={!newKey || !newValue.trim()} onClick={() => { set(newKey, newValue.trim()); setNewKey(""); setNewValue(""); }}><Plus size={13} /> Add</button></td>
              </tr>
            </tbody></table>
            <p className="footnote">
              <Cpu size={12} style={{ verticalAlign: -2 }} /> Applying writes <span className="mono">{r.file}</span> and loads it with <span className="mono">sysctl -p</span>; if the kernel refuses a value, the previous file is put back. The file is read again at every boot. Settings outside it (other files in /etc/sysctl.d) still apply; later files win.
            </p>
          </>
        )}
      </div>
    </div>
  );
}
