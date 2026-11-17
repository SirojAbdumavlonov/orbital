import { AlertTriangle, Globe, Loader2, Pause, Pencil, Play, Plus, RefreshCw, Trash2 } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { confirm, DrawerClose, Modal, Sparkline } from "../components/ui";
import { api, asApiError, CheckInput, CheckKind, CheckStatus, NginxConfig } from "../lib/api";
import { useUptimeEvents } from "../lib/uptime";
import { isSsh, useStore } from "../store";

const INTERVALS = [1, 5, 15] as const;

const blank = (kind: CheckKind = "http"): CheckInput => ({ name: "", kind, target: kind === "http" ? "https://" : "", expectedStatus: 200, contains: "", intervalMinutes: 5 });

/** "for 37 min": how long the current state has lasted. */
function lasted(iso: string): string {
  const s = Math.max(0, (Date.now() - new Date(iso).getTime()) / 1000);
  if (s < 60) return "for under a minute";
  if (s < 3600) return `for ${Math.floor(s / 60)} min`;
  if (s < 86400) return `for ${Math.floor(s / 3600)} h`;
  return `for ${Math.floor(s / 86400)} days`;
}

function ago(iso: string | null | undefined): string {
  if (!iso) return "—";
  const s = Math.max(0, (Date.now() - new Date(iso).getTime()) / 1000);
  if (s < 60) return "just now";
  if (s < 3600) return `${Math.floor(s / 60)} min ago`;
  if (s < 86400) return `${Math.floor(s / 3600)} h ago`;
  return `${Math.floor(s / 86400)} d ago`;
}

const pct = (v: number | null) => (v == null ? "—" : v === 100 ? "100%" : `${v.toFixed(v >= 99 ? 2 : 1)}%`);

function stateOf(s: CheckStatus): { cls: string; label: string } {
  if (s.check.paused) return { cls: "paused", label: "Paused" };
  if (s.state === "up") return { cls: s.last?.warning ? "warn" : "up", label: "Up" };
  if (s.state === "down") return { cls: "down", label: "Down" };
  return { cls: "unknown", label: "Waiting" };
}

function CheckDialog({ check, initial, onClose, onSaved }: { check: CheckStatus["check"] | null; initial?: CheckInput; onClose: () => void; onSaved: () => void }) {
  const toast = useStore((x) => x.toast);
  const [f, setF] = useState<CheckInput>(
    check
      ? { name: check.name === check.target ? "" : check.name, kind: check.kind, target: check.target, expectedStatus: check.expectedStatus || 200, contains: check.contains ?? "", intervalMinutes: check.intervalMinutes, paused: check.paused }
      : initial ?? blank(),
  );
  const [busy, setBusy] = useState(false);
  const set = (p: Partial<CheckInput>) => setF((x) => ({ ...x, ...p }));
  const http = f.kind === "http";
  const ok = http ? /^https?:\/\/[^/\s]+/.test(f.target.trim()) : /^(\[[0-9a-fA-F:]+\]|[^\s:]+):\d{1,5}$/.test(f.target.trim());

  async function save() {
    setBusy(true);
    try {
      await api.saveUptimeCheck(check?.id ?? null, { ...f, expectedStatus: http ? Number(f.expectedStatus) || 200 : null, contains: http ? f.contains : null });
      onSaved();
      onClose();
    } catch (e) {
      toast({ kind: "error", title: "Couldn't save the check", body: asApiError(e).message });
    } finally {
      setBusy(false);
    }
  }

  return (
    <Modal title={check ? `Edit ${check.name}` : "Add an uptime check"} subtitle="Checks run from this computer while Orbital is open. You get a notification when one goes down and when it's back."
      onClose={busy ? undefined : onClose}
      footer={<><button className="btn" onClick={onClose}>Cancel</button><button className="btn primary" disabled={busy || !ok} onClick={save}>{check ? "Save" : "Add check"}</button></>}>
      <div className="form" onKeyDown={(e) => e.key === "Enter" && ok && !busy && (e.target as HTMLElement).tagName === "INPUT" && save()}>
        <div className="field"><span>Check</span>
          <div className="segmented">
            <button className={http ? "on" : ""} onClick={() => set({ kind: "http", target: f.target || "https://" })}>Website (HTTP/HTTPS)</button>
            <button className={!http ? "on" : ""} onClick={() => set({ kind: "tcp", target: f.target.startsWith("http") ? "" : f.target })}>Port (TCP)</button>
          </div>
        </div>
        <label className="field"><span>{http ? "URL" : "Host and port"}</span>
          <input className="input mono" autoFocus value={f.target} onChange={(e) => set({ target: e.target.value })} placeholder={http ? "https://shop.example.com/health" : "db.example.com:5432"} />
          {!http && <small className="hint">Up when a TCP connection opens, for databases, mail and other services without a web page.</small>}
        </label>
        <label className="field"><span>Name <span className="faint">(optional)</span></span>
          <input className="input" value={f.name} onChange={(e) => set({ name: e.target.value })} placeholder={f.target || "Shop"} />
        </label>
        {http && (
          <div className="row2" style={{ gridTemplateColumns: "120px 1fr" }}>
            <label className="field"><span>Expected status</span>
              <input className="input mono" inputMode="numeric" value={f.expectedStatus ?? ""} onChange={(e) => set({ expectedStatus: Number(e.target.value.replace(/\D/g, "").slice(0, 3)) || null })} />
            </label>
            <label className="field"><span>Page must contain <span className="faint">(optional)</span></span>
              <input className="input" value={f.contains ?? ""} onChange={(e) => set({ contains: e.target.value })} placeholder="Add to cart" />
            </label>
          </div>
        )}
        <div className="field"><span>Check every</span>
          <div className="segmented">
            {INTERVALS.map((m) => <button key={m} className={f.intervalMinutes === m ? "on" : ""} onClick={() => set({ intervalMinutes: m })}>{m} min</button>)}
          </div>
          {http && f.target.startsWith("https://") && <small className="hint">Orbital also warns when the site's certificate expires within 14 days.</small>}
        </div>
      </div>
    </Modal>
  );
}

/** Domains the connected servers' nginx serves that aren't checked yet. */
function useNginxSuggestions(checked: Set<string>) {
  const sessions = useStore((x) => x.sessions);
  const [configs, setConfigs] = useState<Record<string, { server: string; config: NginxConfig }>>({});
  const withNginx = Object.values(sessions).filter((s) => isSsh(s) && !s.lost && s.tools?.some((t) => t.id === "nginx"));
  const key = withNginx.map((s) => s.info.id).join(",");
  useEffect(() => {
    for (const s of withNginx) {
      if (configs[s.info.id]) continue;
      api.nginxConfig(s.info.id).then((config) => setConfigs((c) => ({ ...c, [s.info.id]: { server: s.profile.name, config } }))).catch(() => {});
    }
  }, [key]);
  return useMemo(() => {
    const live = new Set(withNginx.map((s) => s.info.id));
    const out: { url: string; domain: string; server: string }[] = [];
    const seen = new Set<string>();
    for (const [sid, { server, config }] of Object.entries(configs)) {
      if (!live.has(sid)) continue;
      const https = new Set(config.sites.filter((x) => x.ssl || x.listens.some((l) => /\b443\b|\bssl\b/.test(l))).flatMap((x) => x.serverNames));
      for (const site of config.sites) {
        for (const d of site.serverNames) {
          if (!d.includes(".") || /^[~*.]/.test(d) || d.includes("$") || seen.has(d)) continue;
          seen.add(d);
          const url = `${https.has(d) ? "https" : "http"}://${d}/`;
          if (!checked.has(url) && !checked.has(url.slice(0, -1))) out.push({ url, domain: d, server });
        }
      }
    }
    return out;
  }, [configs, key, checked]);
}

function Ticks({ s }: { s: CheckStatus }) {
  const last = s.recent.slice(-30);
  return (
    <div className="uptime-ticks" aria-hidden>
      {Array.from({ length: 30 - last.length }, (_, i) => <i key={`e${i}`} />)}
      {last.map((r) => <i key={r.at} className={r.up ? (r.warning ? "warn" : "up") : "down"} title={`${new Date(r.at).toLocaleString()}: ${r.up ? `up, ${r.latencyMs} ms` : r.error}`} />)}
    </div>
  );
}

function Detail({ s, onClose, onEdit }: { s: CheckStatus; onClose: () => void; onEdit: () => void }) {
  const st = stateOf(s);
  const latency = s.recent.filter((r) => r.up && r.latencyMs != null).map((r) => r.latencyMs!);
  const avg = latency.length ? Math.round(latency.reduce((a, b) => a + b, 0) / latency.length) : null;
  return (
    <aside className="drawer">
      <div className="drawer-head">
        <h3>{s.check.name}</h3>
        <div className="mono faint selectable" style={{ fontSize: 12, overflowWrap: "anywhere" }}>{s.check.target}</div>
        <DrawerClose onClose={onClose} />
      </div>
      <div className="scroll">
        <div className="actions">
          <button className="btn" onClick={onEdit}><Pencil size={13} /> Edit check</button>
        </div>
        <dl className="kv">
          <dt>State</dt><dd><span className={`uptime-dot ${st.cls}`} /> {st.label}{s.since && !s.check.paused ? ` since ${new Date(s.since).toLocaleString()}` : ""}</dd>
          <dt>Checks</dt><dd>{s.check.kind === "http" ? `HTTP ${s.check.expectedStatus}${s.check.contains ? `, page contains “${s.check.contains}”` : ""}` : "TCP connection"}, every {s.check.intervalMinutes} min</dd>
          <dt>Uptime</dt><dd>{pct(s.uptime24h)} in 24 h · {pct(s.uptime7d)} in 7 days</dd>
          <dt>Latency</dt><dd>{s.last?.latencyMs != null ? `${s.last.latencyMs} ms now` : "—"}{avg != null ? ` · ${avg} ms average` : ""}</dd>
          {s.last?.certExpires && <><dt>Certificate</dt><dd>expires {new Date(s.last.certExpires).toLocaleDateString()}</dd></>}
        </dl>
        {s.last?.warning && <div className="notice uptime-warning"><AlertTriangle size={14} /> {s.last.warning}</div>}
        <div className="section-title">Latency, last {s.recent.length} checks</div>
        <div style={{ padding: "0 14px" }}><Sparkline values={s.recent.map((r) => r.latencyMs ?? 0)} max={0} width={300} height={46} fluid /></div>
        <div className="section-title">Last error</div>
        <div style={{ padding: "0 14px", fontSize: 12 }}>
          {s.lastError ? <><div className="faint">{new Date(s.lastError.at).toLocaleString()}</div><div className="mono selectable">{s.lastError.error}</div></> : <span className="faint">None in the last 7 days.</span>}
        </div>
        <div className="section-title">Up and down</div>
        {s.changes.length === 0 && <p className="faint" style={{ margin: "0 14px", fontSize: 12 }}>No changes recorded yet.</p>}
        <ul className="uptime-changes">
          {s.changes.map((c) => (
            <li key={c.at}><span className={`uptime-dot ${c.up ? "up" : "down"}`} /> <b>{c.up ? "Back up" : "Went down"}</b> <span className="faint">{new Date(c.at).toLocaleString()}</span>{c.error && <div className="faint mono">{c.error}</div>}</li>
          ))}
        </ul>
      </div>
    </aside>
  );
}

/** Global page: website and port checks run from this computer, with alerts. */
export function UptimeView() {
  const toast = useStore((x) => x.toast);
  const [list, setList] = useState<CheckStatus[] | null>(null);
  const [editing, setEditing] = useState<{ check: CheckStatus["check"] | null; initial?: CheckInput } | null>(null);
  const [sel, setSel] = useState<string | null>(null);
  const [running, setRunning] = useState<Record<string, boolean>>({});
  const load = useCallback(() => api.listUptime().then(setList).catch((e) => { setList([]); toast({ kind: "error", title: "Couldn't load the uptime checks", body: asApiError(e).message }); }), []);
  useEffect(() => { load(); const t = setInterval(load, 30_000); return () => clearInterval(t); }, []);
  useUptimeEvents(useCallback(() => { load(); }, []));

  const checked = useMemo(() => new Set((list ?? []).map((s) => s.check.target)), [list]);
  const suggestions = useNginxSuggestions(checked);

  async function runNow(id: string) {
    setRunning((r) => ({ ...r, [id]: true }));
    try {
      const s = await api.runUptimeCheck(id);
      setList((l) => l?.map((x) => (x.check.id === id ? s : x)) ?? l);
    } catch (e) {
      toast({ kind: "error", title: "Couldn't run the check", body: asApiError(e).message });
    } finally {
      setRunning((r) => ({ ...r, [id]: false }));
    }
  }
  async function add(url: string) {
    try {
      const c = await api.saveUptimeCheck(null, { name: "", kind: "http", target: url, expectedStatus: 200, intervalMinutes: 5 });
      await load();
      runNow(c.id);
    } catch (e) {
      toast({ kind: "error", title: `Couldn't add ${url}`, body: asApiError(e).message });
    }
  }
  async function togglePause(s: CheckStatus) {
    try {
      await api.setUptimePaused(s.check.id, !s.check.paused);
      load();
    } catch (e) {
      toast({ kind: "error", title: "Couldn't change the check", body: asApiError(e).message });
    }
  }
  async function remove(s: CheckStatus) {
    if (!(await confirm({ title: `Delete ${s.check.name}?`, body: "Its history is deleted too. The activity log keeps its up and down changes.", confirmLabel: "Delete check", danger: true }))) return;
    try {
      await api.deleteUptimeCheck(s.check.id);
      if (sel === s.check.id) setSel(null);
      load();
    } catch (e) {
      toast({ kind: "error", title: "Couldn't delete the check", body: asApiError(e).message });
    }
  }

  const down = list?.filter((s) => !s.check.paused && s.state === "down").length ?? 0;
  const selected = list?.find((s) => s.check.id === sel) ?? null;

  return (
    <div className="view">
      <div className="toolbar">
        {list && list.length > 0 && <span className={down ? "badge bad" : "badge ok"}>{down ? `${down} down` : "All up"}</span>}
        <span className="faint">{list ? `${list.length} check${list.length === 1 ? "" : "s"} · run from this computer while Orbital is open` : "Loading…"}</span>
        <span className="spacer" />
        <button className="btn" onClick={() => setEditing({ check: null, initial: blank("tcp") })}><Plus size={14} /> Port</button>
        <button className="btn primary" onClick={() => setEditing({ check: null })}><Plus size={14} /> Website</button>
      </div>
      {suggestions.length > 0 && (
        <div className="toolbar uptime-suggest">
          <Globe size={14} className="muted" />
          <span className="muted">Sites on {[...new Set(suggestions.map((x) => x.server))].join(", ")}:</span>
          <div className="chips">
            {suggestions.slice(0, 12).map((x) => <button key={x.url} className="chip" title={`Check ${x.url} every 5 minutes`} onClick={() => add(x.url)}><Plus size={11} /> {x.domain}</button>)}
          </div>
        </div>
      )}
      <div className={`split ${selected ? "" : "no-drawer"}`} style={{ flex: 1, minHeight: 0 }}>
        <div className="scroll">
          {list && list.length === 0 ? (
            <div className="empty">
              <div>
                <Globe size={28} />
                <h3>Know when a site goes down</h3>
                <p>Add a website or a port. Orbital checks it every few minutes from this computer and notifies you when it stops answering and when it's back.</p>
                <button className="btn primary" onClick={() => setEditing({ check: null })}><Plus size={14} /> Add a website</button>
              </div>
            </div>
          ) : (
            <table className="table">
              <thead><tr><th>Check</th><th>Last 30</th><th>Latency</th><th className="num">24 h</th><th className="num">7 days</th><th>Last error</th><th /></tr></thead>
              <tbody>
                {(list ?? []).map((s) => {
                  const st = stateOf(s);
                  return (
                    <tr key={s.check.id} className={`click ${sel === s.check.id ? "sel" : ""} ${s.check.paused ? "dim" : ""}`} onClick={() => setSel(s.check.id)}>
                      <td style={{ maxWidth: 320 }}>
                        <div className="uptime-name"><span className={`uptime-dot ${st.cls}`} title={st.label} /><b>{s.check.name}</b>{s.last?.warning && <AlertTriangle size={13} className="warn-ink" aria-label={s.last.warning} />}</div>
                        {s.check.name !== s.check.target && <div className="mono faint uptime-target">{s.check.target}</div>}
                        <div className="faint" style={{ fontSize: 11.5 }}>{st.label}{s.since && !s.check.paused ? ` ${lasted(s.since)}` : ""} · every {s.check.intervalMinutes} min · checked {ago(s.last?.at)}</div>
                      </td>
                      <td><Ticks s={s} /></td>
                      <td className="nowrap"><div className="uptime-latency"><Sparkline values={s.recent.map((r) => r.latencyMs ?? 0)} max={0} width={80} height={20} /><span className="mono">{s.last?.latencyMs != null ? `${s.last.latencyMs} ms` : "—"}</span></div></td>
                      <td className="num">{pct(s.uptime24h)}</td>
                      <td className="num">{pct(s.uptime7d)}</td>
                      <td className="faint" style={{ maxWidth: 260, fontSize: 12 }}>{s.lastError ? <><span className="uptime-error">{s.lastError.error}</span><br />{ago(s.lastError.at)}</> : "—"}</td>
                      <td className="nowrap" onClick={(e) => e.stopPropagation()}>
                        <button className="icon-btn" title="Check now" aria-label={`Check ${s.check.name} now`} disabled={running[s.check.id] || s.check.paused} onClick={() => runNow(s.check.id)}>{running[s.check.id] ? <Loader2 size={14} className="spin" /> : <RefreshCw size={14} />}</button>
                        <button className="icon-btn" title={s.check.paused ? "Resume" : "Pause"} aria-label={`${s.check.paused ? "Resume" : "Pause"} ${s.check.name}`} onClick={() => togglePause(s)}>{s.check.paused ? <Play size={14} /> : <Pause size={14} />}</button>
                        <button className="icon-btn" title="Edit" aria-label={`Edit ${s.check.name}`} onClick={() => setEditing({ check: s.check })}><Pencil size={14} /></button>
                        <button className="icon-btn" title="Delete" aria-label={`Delete ${s.check.name}`} onClick={() => remove(s)}><Trash2 size={14} /></button>
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          )}
        </div>
        {selected && <Detail s={selected} onClose={() => setSel(null)} onEdit={() => setEditing({ check: selected.check })} />}
      </div>
      {editing && <CheckDialog check={editing.check} initial={editing.initial} onClose={() => setEditing(null)} onSaved={load} />}
    </div>
  );
}
