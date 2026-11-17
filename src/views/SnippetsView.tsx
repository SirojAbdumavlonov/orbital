import { CheckCircle2, Copy, Play, Plus, Save, Shield, Sparkles, Trash2, XCircle } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { confirm } from "../components/ui";
import { api, asApiError, Snippet, SnippetInput, SnippetRun } from "../lib/api";
import { isSsh, useStore } from "../store";

const PARAM = /\{\{\s*([A-Za-z_][A-Za-z0-9_]{0,31})\s*\}\}/g;
const paramsOf = (cmd: string) => [...new Set([...cmd.matchAll(PARAM)].map((m) => m[1]))];
const blank: SnippetInput = { name: "", command: "", description: null, sudo: false };

/** Saved commands, run on one or many connected servers at once. */
export function SnippetsView() {
  const toast = useStore((x) => x.toast);
  const [list, setList] = useState<Snippet[] | null>(null);
  const [selId, setSelId] = useState<string | "new" | null>(null);
  const load = () => api.snippets().then((l) => { setList(l); return l; }).catch(() => { setList([]); return []; });
  useEffect(() => { load().then((l) => setSelId((cur) => cur ?? l[0]?.id ?? null)); }, []);
  const sel = selId === "new" ? null : list?.find((s) => s.id === selId) ?? null;

  async function examples() {
    try {
      const l = await api.addExampleSnippets();
      setList(l);
      setSelId(l[0]?.id ?? null);
    } catch (e) {
      toast({ kind: "error", title: "Couldn't add the examples", body: asApiError(e).message });
    }
  }

  return (
    <div className="view">
      <div className="domains">
        <nav className="domains-nav">
          <button className="btn domains-add" style={{ marginTop: 0, marginBottom: 8 }} onClick={() => setSelId("new")}><Plus size={13} /> New snippet</button>
          {list?.map((s) => (
            <button key={s.id} className={`nav-item ${selId === s.id ? "on" : ""}`} onClick={() => setSelId(s.id)} title={s.description ?? s.command}>
              {s.sudo ? <Shield size={13} /> : <Play size={13} />} <span className="grow">{s.name}</span>
            </button>
          ))}
          {list && list.length === 0 && (
            <div className="domains-note faint">
              <p style={{ margin: "4px 0 10px" }}>No snippets yet. Save commands you run often, with blanks to fill in, and run them on several servers at once.</p>
              <button className="btn" onClick={examples}><Sparkles size={13} /> Add examples</button>
            </div>
          )}
        </nav>
        {selId ? (
          <SnippetPage key={selId} snippet={sel} onSaved={(s) => { load(); setSelId(s.id); }} onDeleted={() => { load().then((l) => setSelId(l[0]?.id ?? null)); }} />
        ) : (
          <div className="empty"><div><h3>Snippets</h3><p>Pick a snippet, or make a new one.</p></div></div>
        )}
      </div>
    </div>
  );
}

function SnippetPage({ snippet, onSaved, onDeleted }: { snippet: Snippet | null; onSaved: (s: Snippet) => void; onDeleted: () => void }) {
  const toast = useStore((x) => x.toast);
  const sessions = useStore((x) => x.sessions);
  const servers = useMemo(() => Object.values(sessions).filter((s) => isSsh(s) && !s.lost), [sessions]);
  const [f, setF] = useState<SnippetInput>(() => snippet ? { name: snippet.name, command: snippet.command, description: snippet.description, sudo: snippet.sudo } : blank);
  const [values, setValues] = useState<Record<string, string>>({});
  const [targets, setTargets] = useState<Set<string>>(() => new Set(servers.length === 1 ? [servers[0].info.id] : []));
  const [runs, setRuns] = useState<SnippetRun[] | null>(null);
  const [busy, setBusy] = useState<"save" | "run" | null>(null);
  const [error, setError] = useState<string | null>(null);
  const dirty = !snippet || f.name !== snippet.name || f.command !== snippet.command || (f.description ?? "") !== (snippet.description ?? "") || f.sudo !== snippet.sudo;
  const params = paramsOf(f.command);
  const missing = params.filter((p) => !values[p]?.trim());

  async function save(): Promise<Snippet | null> {
    setBusy("save");
    setError(null);
    try {
      const s = await api.saveSnippet(snippet?.id ?? null, f);
      onSaved(s);
      return s;
    } catch (e) {
      setError(asApiError(e).message);
      return null;
    } finally {
      setBusy(null);
    }
  }

  async function run() {
    // Run what is on screen: unsaved edits are saved first.
    const s = dirty ? await save() : snippet;
    if (!s) return;
    if (s.sudo || targets.size > 1) {
      const names = servers.filter((x) => targets.has(x.info.id)).map((x) => x.profile.name).join(", ");
      const ok = await confirm({
        title: `Run "${s.name}" on ${targets.size === 1 ? names : `${targets.size} servers`}?`,
        body: `${s.sudo ? "As root (sudo). " : ""}${targets.size > 1 ? `On: ${names}.` : ""}`.trim(),
        confirmLabel: "Run",
      });
      if (!ok) return;
    }
    setBusy("run");
    setRuns(null);
    try {
      setRuns(await api.runSnippet(s.id, [...targets], values));
    } catch (e) {
      toast({ kind: "error", title: "Couldn't run the snippet", body: asApiError(e).message });
    } finally {
      setBusy(null);
    }
  }

  async function remove() {
    if (!snippet) return;
    if (!(await confirm({ title: `Delete "${snippet.name}"?`, body: "Only the saved command is deleted; nothing changes on any server.", confirmLabel: "Delete", danger: true }))) return;
    await api.deleteSnippet(snippet.id);
    onDeleted();
  }

  const okCount = runs?.filter((r) => r.output?.exitCode === 0).length ?? 0;
  return (
    <div className="domains-main scroll">
      <div className="settings" style={{ maxWidth: 900 }}>
        <div className="snippet-edit">
          <div className="row-fields">
            <label className="field" style={{ flex: 2 }}><span>Name</span><input className="input" autoFocus={!snippet} value={f.name} onChange={(e) => setF({ ...f, name: e.target.value })} placeholder="Restart a service" /></label>
            <label className="field" style={{ flex: 3 }}><span>Description (optional)</span><input className="input" value={f.description ?? ""} onChange={(e) => setF({ ...f, description: e.target.value })} /></label>
          </div>
          <label className="field"><span>Command</span>
            <textarea className="input mono snippet-command" rows={4} spellCheck={false} value={f.command} onChange={(e) => setF({ ...f, command: e.target.value })} placeholder="systemctl restart {{service}}" />
            <small className="hint">Runs with <span className="mono">sh</span> on the server. Write <span className="mono">{"{{name}}"}</span> for a blank to fill in when you run it; each value is passed as one quoted word, so it can't run anything else.</small>
          </label>
          <div className="snippet-actions">
            <label className="check"><input type="checkbox" checked={f.sudo} onChange={(e) => setF({ ...f, sudo: e.target.checked })} /><span>Run as root (sudo)</span></label>
            <span className="spacer" />
            {snippet && <button className="btn danger" onClick={remove}><Trash2 size={13} /> Delete</button>}
            <button className="btn" disabled={!dirty || busy !== null || !f.name.trim() || !f.command.trim()} onClick={save}><Save size={13} /> {snippet ? "Save" : "Create"}</button>
          </div>
          {error && <div className="notice bad-text" style={{ margin: 0 }}>{error}</div>}
        </div>

        <h3 className="settings-title">Run</h3>
        {params.length > 0 && (
          <div className="row-fields snippet-params">
            {params.map((p) => (
              <label key={p} className="field"><span className="mono">{p}</span>
                <input className="input mono" value={values[p] ?? ""} onChange={(e) => setValues({ ...values, [p]: e.target.value })} />
              </label>
            ))}
          </div>
        )}
        {servers.length === 0 ? (
          <p className="muted">Connect to a server (SSH) to run this snippet on it.</p>
        ) : (
          <div className="chips snippet-targets">
            <span className="faint">On:</span>
            {servers.map((s) => (
              <button key={s.info.id} className={`chip ${targets.has(s.info.id) ? "on" : ""}`} onClick={() => setTargets((t) => { const n = new Set(t); if (n.has(s.info.id)) n.delete(s.info.id); else n.add(s.info.id); return n; })}>
                {s.profile.name}
              </button>
            ))}
            {servers.length > 1 && <button className="chip" onClick={() => setTargets(new Set(targets.size === servers.length ? [] : servers.map((s) => s.info.id)))}>{targets.size === servers.length ? "None" : "All"}</button>}
          </div>
        )}
        <div className="snippet-actions">
          <button className="btn primary" disabled={busy !== null || targets.size === 0 || !f.command.trim() || !f.name.trim() || missing.length > 0} onClick={run}>
            <Play size={13} /> {busy === "run" ? "Running…" : `Run${targets.size > 1 ? ` on ${targets.size} servers` : ""}`}
          </button>
          {missing.length > 0 && targets.size > 0 && <span className="faint">Fill in {missing.join(", ")}</span>}
          {runs && <span className="faint">{okCount} of {runs.length} succeeded</span>}
        </div>

        {runs?.map((r) => {
          const ok = r.output?.exitCode === 0;
          const text = r.output ? [r.output.stdout, r.output.stderr].filter((x) => x.trim()).join("\n") : "";
          return (
            <div key={r.sessionId} className={`snippet-result ${ok ? "ok" : "bad"}`}>
              <div className="snippet-result-head">
                {ok ? <CheckCircle2 size={14} className="ok-text" /> : <XCircle size={14} className="bad-text" />}
                <b>{servers.find((s) => s.info.id === r.sessionId)?.profile.name ?? r.server}</b>
                <span className="faint mono">{r.server}</span>
                <span className="spacer" />
                {r.output && <span className="faint">exit {r.output.exitCode} · {(r.output.durationMs / 1000).toFixed(1)} s</span>}
                {text && <button className="icon-btn" aria-label="Copy output" onClick={() => { navigator.clipboard?.writeText(text).catch(() => {}); toast({ kind: "success", title: "Output copied" }); }}><Copy size={13} /></button>}
              </div>
              {r.error && <div className="bad-text" style={{ padding: "0 12px 10px" }}>{r.error}</div>}
              {r.output && <pre className="output">{text || "(no output)"}</pre>}
            </div>
          );
        })}
        {runs && <p className="footnote">Sent: <span className="mono">{runs[0]?.command}</span>. Every run is in the activity log, per server.</p>}
      </div>
    </div>
  );
}
