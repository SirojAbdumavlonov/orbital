import { Download, PackageSearch, RefreshCw, Search, Trash2 } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { Modal } from "../components/ui";
import { api, asApiError, FoundPackage, Package, PkgAction, PkgPlan } from "../lib/api";
import { bytes } from "../lib/format";
import { runJob } from "../lib/jobs";
import { useKept } from "../lib/kept";
import { withSafetyOverride } from "../lib/remote";
import { SessionState, useStore } from "../store";

/** The dry run first: everything that installing or removing would change, then do it. */
function PlanDialog({ s, name, action, onClose, onDone }: { s: SessionState; name: string; action: PkgAction; onClose: () => void; onDone: () => void }) {
  const sid = s.info.id;
  const [plan, setPlan] = useState<PkgPlan | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  useEffect(() => { api.packagePlan(sid, name, action).then(setPlan).catch((e) => setError(asApiError(e).message)); }, []);
  const verb = action === "install" ? "Install" : "Remove";
  async function go() {
    setBusy(true);
    try {
      const r = await withSafetyOverride((force) => runJob((p) => api.packageChange(sid, name, action, force, p), {
        key: `${sid}:package:${name}`, title: `${verb} ${name}`, sid,
        done: () => `${name} ${action === "install" ? "installed" : "removed"}`,
      }), "this could cut the server off");
      if (r) onDone();
    } catch {
      /* the jobs layer reports it */
    } finally {
      setBusy(false);
    }
  }
  const changes = plan ? (action === "install" ? plan.install : plan.remove) : [];
  return (
    <Modal title={`${verb} ${name}?`} subtitle={`On ${s.profile.name}, with the system's package manager.`} onClose={onClose}
      footer={<><button className="btn" onClick={onClose}>Cancel</button><button className={`btn ${action === "remove" ? "danger" : "primary"}`} disabled={busy || !plan} onClick={() => void go()}>{busy ? `${verb === "Install" ? "Installing" : "Removing"}…` : verb}</button></>}>
      {error && <div className="alert">{error}</div>}
      {!plan && !error && <p className="muted">Asking the package manager what would change (a dry run, nothing is changed) …</p>}
      {plan && (
        <div className="form">
          {plan.essential.length > 0 && <div className="alert">This would also remove <b className="mono">{plan.essential.join(", ")}</b>, which the server needs to boot or to let you in. Orbital will ask you to confirm by typing.</div>}
          {plan.install.length > 0 && <div><b>Installs {plan.install.length}</b><div className="mono pkg-list">{plan.install.join("  ")}</div></div>}
          {plan.remove.length > 0 && <div><b className="bad-text">Removes {plan.remove.length}</b><div className="mono pkg-list">{plan.remove.join("  ")}</div></div>}
          {changes.length === 0 && plan.install.length + plan.remove.length === 0 && (
            plan.text ? <pre className="output" style={{ maxHeight: 260 }}>{plan.text}</pre> : <p className="muted">Nothing to change.</p>
          )}
        </div>
      )}
    </Modal>
  );
}

/** Packages: what's installed, searching the repositories, install and remove. */
export function PackagesView({ s }: { s: SessionState }) {
  const sid = s.info.id;
  const toast = useStore((x) => x.toast);
  const [installed, setInstalled] = useState<Package[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [filter, setFilter] = useKept(`${sid}:pkgs:filter`, "");
  const [term, setTerm] = useKept(`${sid}:pkgs:term`, "");
  const [found, setFound] = useKept<FoundPackage[] | null>(`${sid}:pkgs:found`, null);
  const [searching, setSearching] = useState(false);
  const [plan, setPlan] = useState<{ name: string; action: PkgAction } | null>(null);
  const [limit, setLimit] = useState(300);

  const load = () => api.installedPackages(sid).then((l) => { setInstalled(l); setError(null); }).catch((e) => setError(asApiError(e).message));
  useEffect(() => { void load(); }, [sid]);
  const shown = useMemo(() => {
    const n = filter.trim().toLowerCase();
    return (installed ?? []).filter((p) => !n || p.name.toLowerCase().includes(n) || p.summary.toLowerCase().includes(n));
  }, [installed, filter]);

  async function search() {
    if (!term.trim()) return;
    setSearching(true);
    try {
      setFound(await api.searchPackages(sid, term.trim()));
    } catch (e) {
      toast({ kind: "error", title: "Search failed", body: asApiError(e).message });
    } finally {
      setSearching(false);
    }
  }

  return (
    <div className="view">
      <div className="toolbar view-toolbar">
        <div className="search"><PackageSearch size={14} /><input className="input" placeholder="Find software to install (e.g. htop)" value={term} onChange={(e) => setTerm(e.target.value)} onKeyDown={(e) => e.key === "Enter" && void search()} /></div>
        <button className="btn" disabled={searching || !term.trim()} onClick={() => void search()}>{searching ? "Searching…" : "Search"}</button>
        <span className="spacer" />
        <button className="icon-btn" aria-label="Refresh" onClick={() => void load()}><RefreshCw size={14} /></button>
      </div>
      <div className="scroll">
        {error && <div className="alert" style={{ margin: 16 }}>{error}</div>}
        {found && (
          <>
            <div className="section-title">In the repositories <span className="faint">{found.length}</span> <button className="link-btn" style={{ marginLeft: 8 }} onClick={() => setFound(null)}>Hide</button></div>
            {found.length === 0 ? <p className="footnote">Nothing found for "{term}". Package names are often short (php-fpm, redis-server, postgresql).</p> : (
              <table className="table"><tbody>
                {found.map((f) => (
                  <tr key={f.name}>
                    <td className="mono nowrap"><b>{f.name}</b></td>
                    <td className="muted">{f.summary}</td>
                    <td style={{ textAlign: "right" }} className="nowrap">{f.installed ? <span className="badge ok">installed</span> : <button className="btn" onClick={() => setPlan({ name: f.name, action: "install" })}><Download size={13} /> Install</button>}</td>
                  </tr>
                ))}
              </tbody></table>
            )}
          </>
        )}
        <div className="section-title pkgs-head">
          <span>Installed <span className="faint">{installed?.length ?? ""}</span></span>
          <div className="search"><Search size={14} /><input className="input" placeholder="Filter installed" value={filter} onChange={(e) => { setFilter(e.target.value); setLimit(300); }} /></div>
        </div>
        <table className="table">
          <thead><tr><th>Package</th><th>Version</th><th className="num">Size</th><th>What it is</th><th /></tr></thead>
          <tbody>
            {shown.slice(0, limit).map((p) => (
              <tr key={p.name}>
                <td className="mono nowrap"><b>{p.name}</b></td>
                <td className="mono faint nowrap"><div className="cell-clip" style={{ maxWidth: 180 }} title={p.version}>{p.version}</div></td>
                <td className="num faint nowrap">{p.size != null ? bytes(p.size) : ""}</td>
                <td className="muted"><div className="cell-clip" style={{ maxWidth: 420 }} title={p.summary}>{p.summary}</div></td>
                <td style={{ textAlign: "right" }}><button className="icon-btn" aria-label={`Remove ${p.name}`} title="Remove (shows what else would go first)" onClick={() => setPlan({ name: p.name, action: "remove" })}><Trash2 size={13} /></button></td>
              </tr>
            ))}
          </tbody>
        </table>
        {shown.length > limit && <p className="footnote"><button className="link-btn" onClick={() => setLimit(limit + 500)}>Show {Math.min(500, shown.length - limit)} more</button> of {shown.length}</p>}
        <p className="footnote">Installing and removing run the system's package manager (apt or dnf) as root; a dry run shows everything that would change first. Removing keeps configuration files (apt remove, not purge).</p>
      </div>
      {plan && <PlanDialog s={s} name={plan.name} action={plan.action} onClose={() => setPlan(null)} onDone={() => { setPlan(null); void load(); if (found) void search(); }} />}
    </div>
  );
}
