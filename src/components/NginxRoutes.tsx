import { ArrowRight, Pencil, Plus, Trash2 } from "lucide-react";
import { useEffect, useState } from "react";
import { api, asApiError, Route, RouteChange, RouteChangeReport, RouteInfo, RoutesView } from "../lib/api";
import { runJob } from "../lib/jobs";
import { confirm, Modal } from "./ui";

// The routes (location blocks) of one nginx server block: list, add, change,
// remove. Changes are previewed, then written to the site's own file through
// safe save (backup, nginx -t, rolled back if it fails) and nginx is reloaded.

/** Where a route goes, in a few words. */
export function routeTarget(r: RouteInfo): string {
  const t = r.route?.target;
  if (!t) return r.summary || "—";
  if (t.kind === "app") return `app on ${t.host}:${t.port}${t.keepPrefix ? "" : " (path without the prefix)"}${t.websockets ? " · WebSockets" : ""}`;
  if (t.kind === "folder") return `files from ${t.dir}`;
  return `redirect ${t.code} → ${t.to}`;
}

/** The lines that differ between two texts, with a little context: [" ", "-", "+"] + text. */
export function lineDiff(before: string, after: string, context = 2): [string, string][] {
  const a = before.split("\n");
  const b = after.split("\n");
  let p = 0;
  while (p < a.length && p < b.length && a[p] === b[p]) p++;
  let s = 0;
  while (s < a.length - p && s < b.length - p && a[a.length - 1 - s] === b[b.length - 1 - s]) s++;
  const out: [string, string][] = [];
  for (let i = Math.max(0, p - context); i < p; i++) out.push([" ", a[i]]);
  for (let i = p; i < a.length - s; i++) out.push(["-", a[i]]);
  for (let i = p; i < b.length - s; i++) out.push(["+", b[i]]);
  for (let i = b.length - s; i < Math.min(b.length, b.length - s + context); i++) out.push([" ", b[i]]);
  return out;
}

export function RoutesPanel({ sid, file, line, onChanged }: { sid: string; file: string; line: number; onChanged: () => void }) {
  const [view, setView] = useState<RoutesView | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [editing, setEditing] = useState<{ spec?: string; route?: Route } | null>(null);
  const load = () => {
    api.nginxRoutes(sid, file, line).then((v) => { setView(v); setError(null); }, (e) => { setView(null); setError(asApiError(e).message); });
  };
  useEffect(load, [sid, file, line]);

  async function remove(r: RouteInfo) {
    const ok = await confirm({
      title: `Remove the route ${r.spec}?`,
      body: <>Requests for <span className="mono">{r.spec}</span> will be handled by the site's other routes. A backup of the file is kept, and nothing changes if nginx -t fails.</>,
      confirmLabel: "Remove route",
      danger: true,
    });
    if (!ok) return;
    try {
      await runJob((p) => api.nginxRouteChange(sid, file, line, { op: "remove", spec: r.spec }, true, p), { key: `route:${sid}`, title: `Removing route ${r.spec}`, sid, done: (x) => (x.reloaded ? `Route ${r.spec} removed; nginx reloaded` : `Route ${r.spec} removed, but nginx didn't reload: ${x.reloadOutput.trim()}`) });
      load();
      onChanged();
    } catch {
      // The job reports the error.
    }
  }

  return (
    <div className="routes">
      <div className="routes-head">
        <span className="section-title" style={{ margin: 0 }}>Routes</span>
        <span className="spacer" />
        <button className="btn" disabled={!view} onClick={() => setEditing({})}><Plus size={13} /> Add route</button>
      </div>
      {error && <div className="alert" style={{ margin: "0 14px 10px" }}>{error}</div>}
      {view && view.routes.length === 0 && <div className="faint routes-empty">No routes yet: nginx answers with its defaults. Add one, for example / to your app.</div>}
      {view && view.routes.map((r) => (
        <div key={`${r.line}:${r.spec}`} className="route" title={r.text}>
          <span className="mono route-path">{r.spec}</span>
          <ArrowRight size={12} className="faint" />
          <span className="route-to">{routeTarget(r)}</span>
          <span className="spacer" />
          {r.route
            ? <button className="icon-btn" aria-label={`Change route ${r.spec}`} title="Change" onClick={() => setEditing({ spec: r.spec, route: r.route! })}><Pencil size={13} /></button>
            : <span className="faint route-hand" title="Written by hand in a way this form doesn't cover; change it in the editor">custom</span>}
          <button className="icon-btn" aria-label={`Remove route ${r.spec}`} title="Remove" onClick={() => remove(r)}><Trash2 size={13} /></button>
        </div>
      ))}
      {view && <div className="faint routes-file">In <span className="mono">{view.file}</span></div>}
      {editing && (
        <RouteDialog
          sid={sid}
          file={file}
          line={line}
          spec={editing.spec}
          initial={editing.route}
          onClose={() => setEditing(null)}
          onDone={() => { setEditing(null); load(); onChanged(); }}
        />
      )}
    </div>
  );
}

type Kind = "app" | "folder" | "redirect";

function RouteDialog({ sid, file, line, spec, initial, onClose, onDone }: {
  sid: string; file: string; line: number; spec?: string; initial?: Route; onClose: () => void; onDone: () => void;
}) {
  const t = initial?.target;
  const [path, setPath] = useState(initial?.path ?? "/api");
  const [kind, setKind] = useState<Kind>(t?.kind ?? "app");
  const [host, setHost] = useState(t?.kind === "app" ? t.host : "127.0.0.1");
  const [port, setPort] = useState(t?.kind === "app" ? String(t.port) : "3000");
  const [keepPrefix, setKeepPrefix] = useState(t?.kind === "app" ? t.keepPrefix : false);
  const [ws, setWs] = useState(t?.kind === "app" ? t.websockets : true);
  const [dir, setDir] = useState(t?.kind === "folder" ? t.dir : "/var/www/");
  const [to, setTo] = useState(t?.kind === "redirect" ? t.to : "https://");
  const [code, setCode] = useState(t?.kind === "redirect" ? t.code : 301);
  const [exact, setExact] = useState(initial?.exact ?? false);
  const [preview, setPreview] = useState<RouteChangeReport | null>(null);
  const [problem, setProblem] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const route: Route = {
    path: path.trim(),
    exact: kind === "redirect" && exact,
    target: kind === "app"
      ? { kind, host: host.trim(), port: Number(port) || 0, keepPrefix, websockets: ws }
      : kind === "folder" ? { kind, dir: dir.trim() } : { kind, to: to.trim(), code },
  };
  const change: RouteChange = spec ? { op: "replace", spec, route } : { op: "add", route };
  const key = JSON.stringify(change);

  // Preview as you type: what the file will look like (nothing is written).
  useEffect(() => {
    let alive = true;
    const h = setTimeout(() => {
      api.nginxRouteChange(sid, file, line, change, false).then(
        (r) => { if (alive) { setPreview(r); setProblem(null); } },
        (e) => { if (alive) { setPreview(null); setProblem(asApiError(e).message.replace(/^Invalid input: /, "")); } },
      );
    }, 300);
    return () => { alive = false; clearTimeout(h); };
  }, [key]);

  async function save() {
    setBusy(true);
    setProblem(null);
    try {
      await runJob((p) => api.nginxRouteChange(sid, file, line, change, true, p), {
        key: `route:${sid}`, title: `${spec ? "Changing" : "Adding"} route ${route.path}`, sid,
        done: (x) => (x.reloaded ? `Route ${route.path} saved; nginx -t passed and nginx reloaded` : `Route ${route.path} saved, but nginx didn't reload: ${x.reloadOutput.trim()}`),
      });
      onDone();
    } catch (e) {
      const err = asApiError(e);
      setProblem(err.kind === "ValidationFailed" ? `nginx -t rejected it, so the file was put back:\n${err.data?.output ?? err.message}` : err.message);
    } finally {
      setBusy(false);
    }
  }

  const diff = preview ? lineDiff(preview.before, preview.after) : [];
  return (
    <Modal
      title={spec ? `Change route ${spec}` : "Add a route"}
      subtitle="Written to this site's own file; nginx -t checks it before it's kept, and nginx reloads."
      wide
      keepOnOutsideClick
      onClose={onClose}
      footer={<><button className="btn" onClick={onClose}>Cancel</button><button className="btn primary" disabled={busy || !preview} onClick={save}>{busy ? "Saving…" : spec ? "Save route" : "Add route"}</button></>}
    >
      <div className="route-form">
        <div className="grid2">
          <label className="field">
            <span>Path</span>
            <input className="input mono" value={path} onChange={(e) => setPath(e.target.value)} placeholder="/api" autoFocus />
            <span className="hint">Everything under it, e.g. /api covers /api/users. Written with a trailing slash.</span>
          </label>
          <div className="field">
            <span>Sends requests to</span>
            <div className="segmented" style={{ alignSelf: "flex-start" }}>
              {([["app", "An app on a port"], ["folder", "Files in a folder"], ["redirect", "Another address"]] as [Kind, string][]).map(([k, label]) => (
                <button key={k} className={kind === k ? "on" : ""} onClick={() => setKind(k)}>{label}</button>
              ))}
            </div>
          </div>
        </div>
        {kind === "app" && (
          <div className="grid2">
            <label className="field">
              <span>Address and port</span>
              <div className="input-suffix">
                <input className="input mono" value={host} onChange={(e) => setHost(e.target.value)} placeholder="127.0.0.1" />
                <span>:</span>
                <input className="input mono" style={{ maxWidth: 100 }} inputMode="numeric" value={port} onChange={(e) => setPort(e.target.value.replace(/\D/g, "").slice(0, 5))} />
              </div>
              <span className="hint">Where the app listens on the server; usually 127.0.0.1.</span>
            </label>
            <div className="field">
              <span>Options</span>
              <label className="check"><input type="checkbox" checked={keepPrefix} onChange={(e) => setKeepPrefix(e.target.checked)} /> The app expects the full path ({(path.replace(/\/?$/, "/") || "/") + "users"}, not /users)</label>
              <label className="check"><input type="checkbox" checked={ws} onChange={(e) => setWs(e.target.checked)} /> WebSockets and long requests</label>
            </div>
          </div>
        )}
        {kind === "folder" && (
          <label className="field">
            <span>Folder on the server</span>
            <input className="input mono" value={dir} onChange={(e) => setDir(e.target.value)} placeholder="/var/www/example.com/static" />
            <span className="hint">nginx must be able to read it (usually www-data or nginx).</span>
          </label>
        )}
        {kind === "redirect" && (
          <div className="grid2">
            <label className="field">
              <span>Redirect to</span>
              <input className="input mono" value={to} onChange={(e) => setTo(e.target.value)} placeholder="https://example.com/new" />
              <label className="check"><input type="checkbox" checked={exact} onChange={(e) => setExact(e.target.checked)} /> Only this exact path, not what's under it</label>
            </label>
            <label className="field">
              <span>Kind</span>
              <select className="input" value={code} onChange={(e) => setCode(Number(e.target.value))}>
                <option value={301}>301 · moved for good</option>
                <option value={302}>302 · for now</option>
                <option value={308}>308 · for good, keeps POST</option>
                <option value={307}>307 · for now, keeps POST</option>
              </select>
            </label>
          </div>
        )}
        {problem && <div className="alert" style={{ whiteSpace: "pre-wrap" }}>{problem}</div>}
        {preview && (
          <div className="field">
            <span>Change to <span className="mono">{preview.file}</span></span>
            <pre className="route-diff mono">{diff.map(([m, l], i) => <div key={i} className={m === "+" ? "add" : m === "-" ? "del" : ""}>{m} {l}</div>)}</pre>
          </div>
        )}
      </div>
    </Modal>
  );
}
