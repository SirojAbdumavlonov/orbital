import { FileText, FolderOpen, Lock, Pencil, RefreshCw, Search, ShieldAlert, ShieldCheck, Zap } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { DrawerClose } from "../components/ui";
import { api, asApiError, NginxConfig, NginxSite } from "../lib/api";
import { parentPath } from "../lib/format";
import { SessionState, useStore } from "../store";

const siteKey = (x: NginxSite) => `${x.file}:${x.line}`;
const siteName = (x: NginxSite) => x.serverNames.join(", ") || "Default (any name)";

/** What a site does, in a few words: redirect, proxy targets, or a folder of files. */
function serves(x: NginxSite): string {
  if (x.redirect) return `Redirects: ${x.redirect}`;
  const proxies = [...new Set(x.locations.map((l) => l.proxyPass).filter(Boolean))] as string[];
  if (proxies.length) return `Proxy to ${proxies.join(", ")}`;
  const root = x.root ?? x.locations.find((l) => l.root)?.root;
  return root ? `Files in ${root}` : "—";
}

/** nginx as it is actually configured: every site, where it points, and the files behind it. */
export function NginxView({ s }: { s: SessionState }) {
  const { toast, openFile, showFolder, openTool, patch } = useStore();
  const hasCertbot = !!s.tools?.some((t) => t.id === "certbot");
  const sid = s.info.id;
  const unit = s.tools?.find((t) => t.id === "nginx")?.unit ?? null;
  const [cfg, setCfg] = useState<NginxConfig | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [q, setQ] = useState("");
  const [sel, setSel] = useState<string | null>(null);
  const [reloading, setReloading] = useState(false);

  const load = async () => {
    setLoading(true);
    try {
      setCfg(await api.nginxConfig(sid));
      setError(null);
    } catch (e) {
      setError(asApiError(e).message);
    } finally {
      setLoading(false);
    }
  };
  useEffect(() => { load(); }, [sid]);

  async function reload() {
    if (!unit) return;
    setReloading(true);
    try {
      const rep = await api.serviceAction(sid, unit, "reload");
      if (rep.output.exitCode === 0) toast({ kind: "success", title: "nginx reloaded", body: rep.preflight ? `${rep.preflight[0]} passed` : undefined });
      else toast({ kind: "error", title: "Reload failed", body: rep.output.stderr || rep.output.stdout });
    } catch (e) {
      const err = asApiError(e);
      if (err.kind === "PreflightFailed") toast({ kind: "error", title: `Not reloaded: ${err.data.check} found errors`, body: err.data.output });
      else toast({ kind: "error", title: "Reload failed", body: err.message });
    } finally {
      setReloading(false);
      load();
    }
  }

  const sites = useMemo(() => {
    const needle = q.trim().toLowerCase();
    return (cfg?.sites ?? []).filter((x) => !needle || [siteName(x), x.file, serves(x), x.listens.join(" ")].join(" ").toLowerCase().includes(needle));
  }, [cfg, q]);
  const cur = cfg?.sites.find((x) => siteKey(x) === sel) ?? null;
  const mainFile = cfg?.files[0]?.path ?? "/etc/nginx/nginx.conf";

  return (
    <div className="view">
      <div className="toolbar">
        {cfg && (cfg.testOk
          ? <span className="badge ok" title={cfg.testOutput}><ShieldCheck size={12} /> Configuration is valid</span>
          : <span className="badge bad"><ShieldAlert size={12} /> Configuration has errors</span>)}
        <div className="search"><Search size={14} /><input className="input" placeholder="Find a domain, port or file" value={q} onChange={(e) => setQ(e.target.value)} /></div>
        <span className="spacer" />
        <button className="btn" onClick={() => showFolder(sid, parentPath(mainFile))}><FolderOpen size={13} /> Open folder</button>
        {unit && <button className="btn" disabled={reloading} title="Apply saved changes without dropping connections. nginx -t runs first." onClick={reload}><Zap size={13} className={reloading ? "spin" : ""} /> Reload nginx</button>}
        <button className="icon-btn" aria-label="Refresh" title="Read the configuration again" onClick={load}><RefreshCw size={14} className={loading ? "spin" : ""} /></button>
      </div>
      {error && <div className="empty"><div><h3>Couldn't read the nginx configuration</h3><p>{error}</p><button className="btn" onClick={load}>Try again</button></div></div>}
      {!error && cfg && !cfg.testOk && (
        <div style={{ padding: 14, display: "flex", flexDirection: "column", gap: 10 }}>
          <div className="alert">
            <b>nginx can't load its configuration, so the sites can't be listed.</b> Fix the problem below, then refresh.
            <pre className="output" style={{ marginTop: 8 }}>{cfg.testOutput || "nginx -T produced no output."}</pre>
          </div>
          <div><button className="btn" onClick={() => showFolder(sid, "/etc/nginx")}><FolderOpen size={13} /> Open /etc/nginx in Files</button></div>
        </div>
      )}
      {!error && cfg?.testOk && (
        <div className="split">
          <div className="scroll">
            <table className="table">
              <thead><tr><th>Domain names</th><th>Listens on</th><th>Serves</th><th>Defined in</th></tr></thead>
              <tbody>
                {sites.map((x) => (
                  <tr key={siteKey(x)} className={`click ${sel === siteKey(x) ? "sel" : ""}`} onClick={() => setSel(siteKey(x))} onDoubleClick={() => openFile(sid, x.file)}>
                    <td className={x.serverNames.length ? "mono" : "faint"}><div className="cell-clip" style={{ maxWidth: 230 }}>{siteName(x)}</div></td>
                    <td className="mono muted nowrap">{x.ssl && <Lock size={11} style={{ marginRight: 5, verticalAlign: -1 }} aria-label="HTTPS" />}{x.listens.map((l) => l.split(" ")[0]).join(", ") || "80"}</td>
                    <td className="muted"><div className="cell-clip" style={{ maxWidth: 210 }} title={serves(x)}>{serves(x)}</div></td>
                    <td className="mono faint"><div className="cell-clip" style={{ maxWidth: 170 }} title={`${x.file}:${x.line}`}>{x.file.replace(/^\/etc\/nginx\//, "")}</div></td>
                  </tr>
                ))}
              </tbody>
            </table>
            {sites.length === 0 && <div className="empty" style={{ flex: "none" }}><p>{q ? "No sites match your search." : "nginx has no server blocks configured."}</p></div>}
            <div className="section-title">Configuration files nginx loads ({cfg.files.length})</div>
            <table className="table">
              <tbody>
                {cfg.files.map((f) => (
                  <tr key={f.path} className="click" onClick={() => openFile(sid, f.path)} title="Open in the editor">
                    <td style={{ width: 24 }}><FileText size={14} className="faint" /></td>
                    <td className="mono">{f.path}</td>
                    <td className="num faint">{f.lines} lines</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          <aside className="drawer">
            {!cur ? (
              <div className="empty"><p>Select a site to see where it points. Double-click to edit its configuration.</p></div>
            ) : (
              <>
                <div className="drawer-head">
                <DrawerClose onClose={() => setSel(null)} />
                  <h3 className="mono" style={{ wordBreak: "break-all" }}>{siteName(cur)}</h3>
                  <div className="muted">{serves(cur)}</div>
                </div>
                <div className="actions">
                  <button className="btn primary" onClick={() => openFile(sid, cur.file)}><Pencil size={13} /> Edit configuration</button>
                  {!cur.ssl && hasCertbot && cur.serverNames.some((n) => n.includes(".")) && (
                    <button className="btn" title="Request a free Let's Encrypt certificate for these domains" onClick={() => { patch(sid, { certDomains: cur.serverNames.filter((n) => n.includes(".") && !/^[~*]/.test(n)) }); openTool(sid, "certbot"); }}>
                      <Lock size={13} /> Get HTTPS certificate
                    </button>
                  )}
                </div>
                <dl className="kv">
                  <dt>Domain names</dt><dd className="mono">{cur.serverNames.join(" ") || "—"}</dd>
                  <dt>Listens on</dt><dd className="mono">{cur.listens.join(", ") || "80"}</dd>
                  <dt>HTTPS</dt><dd className="mono">{cur.ssl ? cur.sslCertificate ?? "on" : "off"}</dd>
                  {cur.root && <><dt>Files from</dt><dd className="mono">{cur.root}</dd></>}
                  {cur.redirect && <><dt>Redirect</dt><dd className="mono">{cur.redirect}</dd></>}
                  <dt>Defined in</dt><dd className="mono">{cur.file}:{cur.line}</dd>
                </dl>
                <div className="scroll" style={{ borderTop: "1px solid var(--slate-4)" }}>
                  <table className="table">
                    <thead><tr><th>Location</th><th>Goes to</th></tr></thead>
                    <tbody>
                      {cur.locations.map((l) => {
                        const up = cfg.upstreams.find((u) => l.proxyPass?.replace(/^[a-z]+:\/\//, "").split("/")[0] === u.name);
                        return (
                          <tr key={`${l.line}:${l.path}`}>
                            <td className="mono">{l.path}</td>
                            <td className="mono muted" style={{ wordBreak: "break-all" }}>
                              {l.proxyPass ?? l.root ?? "—"}
                              {up && <div className="faint">{up.servers.map((x) => x.split(" ")[0]).join(", ")}</div>}
                            </td>
                          </tr>
                        );
                      })}
                    </tbody>
                  </table>
                  {cur.locations.length === 0 && <div className="empty" style={{ flex: "none" }}><p>No location blocks.</p></div>}
                </div>
              </>
            )}
          </aside>
        </div>
      )}
    </div>
  );
}
