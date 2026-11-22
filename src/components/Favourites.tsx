import { Boxes, FileText, Folder, Play, RotateCcw, RefreshCw, Star, X } from "lucide-react";
import { useEffect, useState } from "react";
import { api, asApiError, Snippet } from "../lib/api";
import { defaultLabel, FAV_KINDS, Favourite, FavKind, favProblem, useFavourites } from "../lib/favourites";
import { runJob } from "../lib/jobs";
import { isSsh, SessionState, useStore } from "../store";
import { Modal } from "./ui";

const ICONS: Record<FavKind, typeof Star> = { restart: RotateCcw, reload: RefreshCw, container: Boxes, file: FileText, folder: Folder, snippet: Play };

/** One-click buttons under the server's name; nothing shows until one is added. */
export function FavouriteBar({ s }: { s: SessionState }) {
  const profileId = s.profile.id;
  const favs = useFavourites((f) => f.byProfile[profileId]);
  useEffect(() => { useFavourites.getState().list(profileId); }, [profileId]);
  if (!favs?.length) return null;
  return (
    <div className="fav-bar" aria-label="Favourite actions">
      {favs.map((f) => <FavButton key={f.id} s={s} f={f} />)}
    </div>
  );
}

function FavButton({ s, f }: { s: SessionState; f: Favourite }) {
  const Icon = ICONS[f.kind];
  const [busy, setBusy] = useState(false);
  const run = async () => {
    const st = useStore.getState();
    const sid = s.info.id;
    if (f.kind === "folder") return st.patch(sid, { tab: "files", cwd: f.target });
    if (f.kind === "file") {
      st.patch(sid, { tab: "files" });
      return void st.openFile(sid, f.target);
    }
    setBusy(true);
    try {
      if (f.kind === "container") {
        await runJob((p) => api.containerAction(sid, f.target, "restart", p), {
          key: `fav:${sid}:${f.id}`, title: f.label, sid, done: () => `${f.label}: done`,
        });
      } else if (f.kind === "restart" || f.kind === "reload") {
        await runJob((p) => api.serviceAction(sid, f.target, f.kind as "restart" | "reload", p), {
          key: `fav:${sid}:${f.id}`, title: f.label, sid, done: () => `${f.label}: done`,
        });
      } else {
        const [r] = await api.runSnippet(f.target, [sid], {});
        const out = r?.output;
        const ok = !!out && out.exitCode === 0 && !r.error;
        const text = (r?.error ?? [out?.stdout, out?.stderr].filter(Boolean).join("\n")).trim();
        st.toast({
          kind: ok ? "success" : "error",
          title: ok ? `${f.label}: done` : r?.needsSudo ? `${f.label}: refused without sudo` : `${f.label} failed`,
          body: (r?.needsSudo ? "Run it from Snippets with “Run again with sudo”. " : "") + (text ? text.split("\n").slice(-4).join("\n") : ""),
        });
      }
    } catch (e) {
      // runJob reports its own errors; a snippet's are shown here.
      if (f.kind === "snippet") st.toast({ kind: "error", title: `${f.label} failed`, body: asApiError(e).message });
    } finally {
      setBusy(false);
    }
  };
  return (
    <span className="fav">
      <button className="fav-btn" disabled={busy} onClick={run} title={`${FAV_KINDS.find((k) => k.kind === f.kind)?.name}: ${f.target}`}>
        <Icon size={12} className={busy ? "spin" : ""} /> {f.label}
      </button>
      <button className="fav-x" aria-label={`Remove ${f.label}`} title="Remove this favourite" onClick={() => useFavourites.getState().remove(s.profile.id, f.id)}>
        <X size={10} />
      </button>
    </span>
  );
}

/** The star at the end of the tab row: add a favourite action for this server. */
export function FavouriteAdd({ s }: { s: SessionState }) {
  const [open, setOpen] = useState(false);
  return (
    <>
      <button className="icon-btn fav-add" aria-label="Add a favourite action" title="Add a favourite action (a one-click button under the server's name)" onClick={() => setOpen(true)}>
        <Star size={15} />
      </button>
      {open && <AddDialog s={s} onClose={() => setOpen(false)} />}
    </>
  );
}

function AddDialog({ s, onClose }: { s: SessionState; onClose: () => void }) {
  const ssh = isSsh(s);
  // Offer what this server has: systemd services, Docker containers, files.
  const hasDocker = !!s.info.dockerVersion || !!s.tools?.some((t) => t.id === "docker");
  const kinds = FAV_KINDS.filter((k) =>
    k.kind === "file" || k.kind === "folder" ||
    (ssh && (k.kind === "snippet" || ((k.kind === "restart" || k.kind === "reload") && s.info.hasSystemd) || (k.kind === "container" && hasDocker))));
  const [kind, setKind] = useState<FavKind>(ssh && s.info.hasSystemd ? "restart" : ssh && hasDocker ? "container" : "folder");
  const [containers, setContainers] = useState<string[]>([]);
  const [target, setTarget] = useState("");
  const [label, setLabel] = useState("");
  const [units, setUnits] = useState<string[]>([]);
  const [snippets, setSnippets] = useState<Snippet[]>([]);
  useEffect(() => {
    if (ssh && s.info.hasSystemd) api.listServices(s.info.id).then((l) => setUnits(l.map((x) => x.unit))).catch(() => {});
    api.snippets().then((l) => setSnippets(l.filter((x) => x.params.length === 0))).catch(() => {});
    if (ssh && hasDocker) api.listContainers(s.info.id).then((l) => setContainers(l.map((c) => c.names))).catch(() => {});
  }, [s.info.id]);
  // Sensible starting points: where you are now.
  useEffect(() => {
    if (kind === "folder") setTarget(s.cwd);
    else if (kind === "file") setTarget(s.activeFile ?? "");
    else setTarget("");
  }, [kind]);
  const problem = favProblem(kind, target);
  const snippetName = snippets.find((x) => x.id === target)?.name;
  const save = () => {
    if (problem) return;
    const t = target.trim();
    useFavourites.getState().add(s.profile.id, { kind, target: t, label: label.trim() || defaultLabel(kind, t, snippetName) });
    onClose();
  };
  return (
    <Modal
      title={`Add a favourite action for ${s.profile.name}`}
      subtitle="It shows as a one-click button under the server's name, on this computer."
      onClose={onClose}
      footer={<><button className="btn" onClick={onClose}>Cancel</button><button className="btn primary" disabled={!!problem} onClick={save}>Add</button></>}
    >
      <div className="fav-form">
        <label className="field">
          <span>What it does</span>
          <select className="input" value={kind} onChange={(e) => setKind(e.target.value as FavKind)}>
            {kinds.map((k) => <option key={k.kind} value={k.kind}>{k.name}</option>)}
          </select>
        </label>
        {(kind === "restart" || kind === "reload") && (
          <label className="field">
            <span>Service</span>
            <input className="input mono" list="fav-units" placeholder="nginx" value={target} onChange={(e) => setTarget(e.target.value)} autoFocus />
            <datalist id="fav-units">{units.map((u) => <option key={u} value={u} />)}</datalist>
            <span className="hint">The pre-flight checks still run (for example nginx -t before nginx restarts).</span>
          </label>
        )}
        {kind === "container" && (
          <label className="field">
            <span>Container</span>
            <select className="input mono" value={target} onChange={(e) => setTarget(e.target.value)}>
              <option value="">Choose…</option>
              {containers.map((c) => <option key={c} value={c}>{c}</option>)}
            </select>
          </label>
        )}
        {(kind === "file" || kind === "folder") && (
          <label className="field">
            <span>{kind === "file" ? "File" : "Folder"}</span>
            <input className="input mono" placeholder={kind === "file" ? "/etc/nginx/nginx.conf" : "/var/log"} value={target} onChange={(e) => setTarget(e.target.value)} autoFocus />
          </label>
        )}
        {kind === "snippet" && (
          <label className="field">
            <span>Snippet</span>
            <select className="input" value={target} onChange={(e) => setTarget(e.target.value)}>
              <option value="">Choose…</option>
              {snippets.map((x) => <option key={x.id} value={x.id}>{x.name}</option>)}
            </select>
            <span className="hint">{snippets.length ? "Snippets that need values aren't listed; run those from Snippets." : "No snippets without values yet; save one in Snippets (sidebar)."}</span>
          </label>
        )}
        <label className="field">
          <span>Button text (optional)</span>
          <input className="input" placeholder={target.trim() ? defaultLabel(kind, target, snippetName) : ""} value={label} maxLength={40} onChange={(e) => setLabel(e.target.value)} />
        </label>
        {target.trim() && problem && <span className="hint" style={{ color: "var(--bad)" }}>{problem}</span>}
      </div>
    </Modal>
  );
}
