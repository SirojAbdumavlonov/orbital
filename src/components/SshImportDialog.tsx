import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { AlertTriangle, ArrowRight, FileCog, Info, KeyRound, RotateCcw } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { api, asApiError, SshImportCandidate, SshImportPreview } from "../lib/api";
import { useStore } from "../store";
import { useSettings } from "../lib/settings";
import { Modal } from "./ui";

const addr = (c: SshImportCandidate) => `${c.username}@${c.host}${c.port !== 22 ? `:${c.port}` : ""}`;
const hasWarning = (c: SshImportCandidate) => c.notes.some((n) => n.level === "warn");

/** Checked by default: new, importable, a Host entry, and nothing to review. */
const preselect = (cs: SshImportCandidate[]) => new Set(cs.filter((c) => c.importable && !c.existingId && !c.jumpOnly && !hasWarning(c)).map((c) => c.key));

/** Import from ~/.ssh/config (or another config file the user picks): preview every host, pick which to save. */
export function SshImportDialog() {
  const { importingSsh, setImportingSsh, loadProfiles, toast } = useStore();
  const [preview, setPreview] = useState<SshImportPreview | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [checked, setChecked] = useState<Set<string>>(new Set());
  const [busy, setBusy] = useState(false);
  // The file to read: ~/.ssh/config unless the user chose another (remembered).
  const configPath = useSettings((x) => x.sshConfigPath);
  const setSettings = useSettings((x) => x.set);
  const path = configPath.trim() || null;

  useEffect(() => {
    if (!importingSsh) return;
    setPreview(null);
    setError(null);
    api.sshConfigPreview(path).then((p) => {
      setPreview(p);
      setChecked(preselect(p.candidates));
    }, (e) => setError(asApiError(e).message));
  }, [importingSsh, path]);

  async function chooseFile() {
    const home = preview?.path.startsWith("~") ? undefined : preview?.path;
    const picked = await openDialog({ title: "Choose an SSH config file", multiple: false, directory: false, defaultPath: home });
    if (typeof picked === "string") setSettings({ sshConfigPath: picked });
  }

  const byKey = useMemo(() => new Map((preview?.candidates ?? []).map((c) => [c.key, c])), [preview]);
  const route = (c: SshImportCandidate) => {
    const hops: SshImportCandidate[] = [];
    for (let j = c.jumpKey ? byKey.get(c.jumpKey) : undefined; j && hops.length < 8; j = j.jumpKey ? byKey.get(j.jumpKey) : undefined) hops.unshift(j);
    return hops;
  };
  // Jump hosts a checked server needs (and that aren't saved yet) come along: name of the first host that needs each.
  const required = useMemo(() => {
    const need = new Map<string, string>();
    for (const key of checked) {
      const c = byKey.get(key);
      for (let j = c?.jumpKey ? byKey.get(c.jumpKey) : undefined; j && !j.existingId; j = j.jumpKey ? byKey.get(j.jumpKey) : undefined) {
        if (!checked.has(j.key) && !need.has(j.key)) need.set(j.key, c!.name);
      }
    }
    return need;
  }, [checked, byKey]);

  if (!importingSsh) return null;
  const close = () => !busy && setImportingSsh(false);
  const total = checked.size + required.size;
  const selectable = (preview?.candidates ?? []).filter((c) => c.importable && !c.existingId);
  const toggle = (key: string) => setChecked((s) => {
    const n = new Set(s);
    if (n.has(key)) n.delete(key);
    else n.add(key);
    return n;
  });

  const run = async () => {
    setBusy(true);
    try {
      const r = await api.importSshConfig(path, [...checked]);
      await loadProfiles();
      const n = r.created.length;
      if (n) toast({ kind: r.failed.length ? "info" : "success", title: `Imported ${n} server${n === 1 ? "" : "s"}`, body: r.failed.length ? `Not imported: ${r.failed.map((f) => `${f.name} (${f.error})`).join("; ")}` : undefined });
      else toast({ kind: r.failed.length ? "error" : "info", title: r.failed.length ? "Nothing was imported" : "Everything is already saved", body: r.failed.map((f) => `${f.name}: ${f.error}`).join("; ") || undefined });
      setImportingSsh(false);
    } catch (e) {
      toast({ kind: "error", title: "Import failed", body: asApiError(e).message });
    } finally {
      setBusy(false);
    }
  };

  return (
    <Modal
      wide
      title={`Import from ${preview?.path ?? path ?? "~/.ssh/config"}`}
      subtitle="Orbital reads only this file and the files it includes. Keys stay where they are: imported servers point to them by path, and nothing secret is copied."
      onClose={close}
      footer={
        <>
          {selectable.length > 0 && (
            <span className="import-select faint">
              <button className="btn ghost" onClick={() => setChecked(new Set(selectable.map((c) => c.key)))}>Select all</button>
              <button className="btn ghost" onClick={() => setChecked(new Set())}>None</button>
            </span>
          )}
          <button className="btn" onClick={close} disabled={busy}>Cancel</button>
          <button className="btn primary" onClick={run} disabled={busy || total === 0}>
            {busy ? "Importing…" : total ? `Import ${total} server${total === 1 ? "" : "s"}` : "Import"}
          </button>
        </>
      }
    >
      <div className="import-source">
        <FileCog size={14} className="faint" />
        <span className="mono">{preview?.path ?? path ?? "~/.ssh/config"}</span>
        <span className="spacer" />
        {path && <button className="btn ghost" disabled={busy} onClick={() => setSettings({ sshConfigPath: "" })} title="Read ~/.ssh/config again"><RotateCcw size={13} /> Use ~/.ssh/config</button>}
        <button className="btn" disabled={busy} onClick={chooseFile}>Choose file…</button>
      </div>
      {error && <div className="note bad">{error}</div>}
      {!preview && !error && <p className="faint">Reading the config…</p>}
      {preview && !preview.found && <div className="note">There's no SSH config at <span className="mono">{preview.path}</span>. Hosts you add with <span className="mono">Host</span> lines there can be imported here.</div>}
      {preview?.found && preview.candidates.length === 0 && <div className="note">The config has no hosts to import: only patterns such as <span className="mono">Host *</span>.</div>}
      {preview && preview.notes.length > 0 && (
        <details className="import-file-notes">
          <summary>{preview.notes.length} note{preview.notes.length === 1 ? "" : "s"} about the file</summary>
          {preview.notes.map((n, i) => <div key={i} className={`note-item ${n.level}`}>{n.text}</div>)}
        </details>
      )}
      {preview && preview.candidates.length > 0 && (
        <div className="import-list" role="list">
          {preview.candidates.map((c) => {
            const saved = !!c.existingId;
            const neededBy = required.get(c.key);
            const on = checked.has(c.key) || !!neededBy;
            const hops = route(c);
            return (
              <label key={c.key} role="listitem" className={`import-row ${on ? "on" : ""} ${saved || !c.importable ? "off" : ""}`}>
                <input type="checkbox" checked={on && !saved} disabled={saved || !c.importable || !!neededBy} onChange={() => toggle(c.key)} />
                <div className="import-main">
                  <div className="import-title">
                    <strong>{c.name}</strong>
                    <span className="mono faint">{addr(c)}</span>
                    {c.jumpOnly && <span className="badge">jump host</span>}
                    {saved && <span className="badge">already saved{c.existingName && c.existingName !== c.name ? ` as ${c.existingName}` : ""}</span>}
                    {!c.importable && <span className="badge bad">can't import</span>}
                    {neededBy && <span className="badge amber">needed by {neededBy}</span>}
                  </div>
                  <div className="import-meta">
                    <span><KeyRound size={12} /> {c.authMethod === "key" ? <span className="mono">{c.keyPath}</span> : "ssh-agent"}</span>
                    {hops.length > 0 && (
                      <span className="import-route">
                        via {hops.map((h, i) => <span key={h.key}>{i > 0 && <ArrowRight size={11} />}<span className="mono">{h.name}</span>{h.existingId && <span className="faint"> (saved)</span>}</span>)}
                      </span>
                    )}
                    {c.source && <span className="mono faint">{c.source}</span>}
                  </div>
                  {c.notes.map((n, i) => (
                    <div key={i} className={`import-note ${n.level}`}>
                      {n.level === "warn" ? <AlertTriangle size={12} /> : <Info size={12} />} {n.text}
                    </div>
                  ))}
                </div>
              </label>
            );
          })}
        </div>
      )}
    </Modal>
  );
}
