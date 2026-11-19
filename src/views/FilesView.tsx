import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import {
  ArrowRightLeft, ArrowUp, ChevronRight, Download, Eye, EyeOff, FilePlus, FileText, Folder, FolderPlus, FolderUp, History, Link2, Lock, Pencil, RefreshCw, RotateCcw, Save,
  Search, ShieldCheck, SquareTerminal, Star, Trash2, Upload, X,
} from "lucide-react";
import { isMac, mod } from "../lib/platform";
import Editor, { DiffEditor, DiffOnMount, OnMount } from "@monaco-editor/react";
import { useCallback, useEffect, useRef, useState } from "react";
import { confirm, Modal } from "../components/ui";
import { FindReport, api, asApiError, FileEntry, SafeWriteReport } from "../lib/api";
import { runJob, useRunningJob } from "../lib/jobs";
import { JobProgress } from "../components/JobProgress";
import { baseName, breadcrumbs, bytes, joinPath, octal, parentPath, relativeTime } from "../lib/format";
import { languageFor } from "../lib/language";
import "../lib/monaco";
import { useResolvedTheme, useSettings } from "../lib/settings";
import { isSsh, OpenFile, SessionState, useStore } from "../store";

export function FilesView({ s }: { s: SessionState }) {
  return (
    <div className="files">
      <Browser s={s} />
      <EditorPane s={s} />
    </div>
  );
}

// ---------------------------------------------------------------- browser
function Browser({ s }: { s: SessionState }) {
  const { patch, toast, openTerminal, openFile } = useStore();
  const ssh = isSsh(s);
  const sid = s.info.id;
  const [entries, setEntries] = useState<FileEntry[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [sel, setSel] = useState<string | null>(null);
  const [showHidden, setShowHidden] = useState(false);
  const [filter, setFilter] = useState("");
  const [rename, setRename] = useState<FileEntry | null>(null);
  const [chmodOf, setChmodOf] = useState<FileEntry | null>(null);
  const [newItem, setNewItem] = useState<"file" | "folder" | null>(null);
  const [copyOf, setCopyOf] = useState<FileEntry[] | null>(null);
  // "Send to server" with nothing selected: first choose what to send.
  const [pickSend, setPickSend] = useState(false);
  const [marks, setMarks] = useBookmarks(s.profile.id);
  const marked = marks.includes(s.cwd);
  const [finding, setFinding] = useState(false);
  const [query, setQuery] = useState("");
  const [found, setFound] = useState<FindReport | null>(null);
  const [searching, setSearching] = useState(false);
  const [findErr, setFindErr] = useState<string | null>(null);
  const folderJob = useRunningJob(`folder:${sid}`);
  async function findNow() {
    if (!query.trim()) return;
    setSearching(true);
    setFindErr(null);
    try {
      setFound(await api.findFiles(sid, s.cwd, query.trim()));
    } catch (e) {
      setFound(null);
      setFindErr(asApiError(e).message.replace(/^Invalid input: /, ""));
    } finally {
      setSearching(false);
    }
  }
  const closeFind = () => { setFinding(false); setFound(null); setQuery(""); setFindErr(null); };
  async function downloadFolder(e: FileEntry) {
    const local = await saveDialog({ defaultPath: `${e.name}.tar.gz`, title: `Download ${e.name} as one archive` });
    if (!local) return;
    try {
      await runJob((p) => api.downloadFolder(sid, e.path, local, p), { key: `folder:${sid}`, title: `Downloading ${e.name}`, sid, quietHere: false, done: (n) => `Downloaded ${e.name} (${bytes(n)}) to ${local}` });
    } catch {
      // The job reports the error.
    }
  }
  const sendRequest = useStore((x) => x.sendRequest);
  useEffect(() => {
    if (sendRequest !== sid) return;
    useStore.setState({ sendRequest: null });
    setPickSend(true);
  }, [sendRequest]);
  const uploadJob = useRunningJob(`upload:${sid}`);
  const copyJob = useRunningJob(`copy:${sid}`);

  const load = useCallback(async (path = s.cwd) => {
    setLoading(true);
    setError(null);
    try {
      setEntries(await api.listDir(sid, path));
    } catch (e) {
      setError(asApiError(e).message);
      setEntries([]);
    } finally {
      setLoading(false);
    }
  }, [sid, s.cwd]);

  useEffect(() => { load(s.cwd); setSel(null); setFilter(""); setFound(null); setFindErr(null); }, [s.cwd]);
  const go = (path: string) => patch(sid, { cwd: path });

  async function openEntry(e: FileEntry) {
    if (e.kind === "dir") return go(e.path);
    return openFile(sid, e.path);
  }

  async function remove(e: FileEntry) {
    const dir = e.kind === "dir";
    const ok = await confirm({
      title: `Delete ${e.name}?`,
      body: dir ? <>This deletes the folder <span className="mono">{e.path}</span> and everything inside it. This can't be undone.</> : <>This permanently deletes <span className="mono">{e.path}</span>.</>,
      confirmLabel: dir ? "Delete folder" : "Delete file",
      danger: true,
      requireText: dir ? e.name : undefined,
    });
    if (!ok) return;
    try {
      await api.remove(sid, e.path, dir);
      toast({ kind: "success", title: `Deleted ${e.name}` });
      load();
    } catch (err) {
      toast({ kind: "error", title: "Couldn't delete", body: asApiError(err).message });
    }
  }

  async function download(e: FileEntry) {
    const local = await saveDialog({ defaultPath: e.name, title: `Download ${e.name}` });
    if (!local) return;
    try {
      const n = await api.download(sid, e.path, local);
      toast({ kind: "success", title: `Downloaded ${e.name}`, body: bytes(n) });
    } catch (err) {
      toast({ kind: "error", title: "Download failed", body: asApiError(err).message });
    }
  }

  async function uploadPaths(paths: string[]) {
    for (const p of paths) {
      const name = p.split(/[\\/]/).pop()!;
      const to = joinPath(s.cwd, name);
      try {
        await runJob((pr) => api.upload(sid, p, to, pr), { key: `upload:${sid}`, title: `Uploading ${name}`, sid, quietHere: false, done: (n) => `Uploaded ${name} (${bytes(n)})` });
        load();
      } catch {
        // The job reports the error.
      }
    }
    load();
  }

  async function pickUpload() {
    const picked = await openDialog({ multiple: true, title: `Upload to ${s.cwd}` });
    if (picked) uploadPaths(Array.isArray(picked) ? picked : [picked]);
  }

  async function pickUploadFolder() {
    const picked = await openDialog({ directory: true, multiple: false, title: `Upload a folder to ${s.cwd}` });
    if (typeof picked === "string") uploadPaths([picked]);
  }

  // Native drag & drop from Finder.
  const [dropping, setDropping] = useState(false);
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    if (useStore.getState().active !== sid) return;
    import("@tauri-apps/api/webview").then(({ getCurrentWebview }) =>
      getCurrentWebview()
        .onDragDropEvent((ev) => {
          if (useStore.getState().sessions[sid]?.tab !== "files") return;
          if (ev.payload.type === "over" || ev.payload.type === "enter") setDropping(true);
          else if (ev.payload.type === "leave") setDropping(false);
          else if (ev.payload.type === "drop") {
            setDropping(false);
            uploadPaths(ev.payload.paths);
          }
        })
        .then((u) => (unlisten = u)),
    ).catch(() => {});
    return () => unlisten?.();
  }, [sid, s.cwd]);

  const visible = entries.filter((e) => (showHidden || !e.hidden) && (!filter || e.name.toLowerCase().includes(filter.toLowerCase())));
  const selected = entries.find((e) => e.path === sel) ?? null;

  return (
    <section className={`browser ${dropping ? "dropzone" : ""}`} aria-label="Remote files">
      <div className="toolbar">
        <button className="icon-btn" aria-label="Up one folder" title="Up" disabled={s.cwd === "/"} onClick={() => go(parentPath(s.cwd))}><ArrowUp size={15} /></button>
        <div className="crumbs mono">
          {breadcrumbs(s.cwd).map((b) => <button key={b.path} onClick={() => go(b.path)}>{b.name}</button>)}
        </div>
        {ssh && <button className={`icon-btn ${marked ? "on" : ""}`} aria-label={marked ? "Remove bookmark" : "Bookmark this folder"} title={marked ? "Remove this folder from bookmarks" : "Bookmark this folder"} onClick={() => setMarks(marked ? marks.filter((m) => m !== s.cwd) : [...marks, s.cwd])}><Star size={14} fill={marked ? "currentColor" : "none"} /></button>}
        <button className="icon-btn" aria-label="Refresh" title="Refresh" onClick={() => load()}><RefreshCw size={14} className={loading ? "spin" : ""} /></button>
      </div>
      {marks.length > 0 && (
        <div className="bookmarks" aria-label="Bookmarked folders">
          {marks.map((m) => (
            <span key={m} className={`bookmark ${m === s.cwd ? "on" : ""}`}>
              <button className="mono" title={m} onClick={() => go(m)}><Star size={11} /> {m === "/" ? "/" : m.split("/").filter(Boolean).pop()}</button>
              <button className="bookmark-x" aria-label={`Remove bookmark ${m}`} title="Remove bookmark" onClick={() => setMarks(marks.filter((x) => x !== m))}><X size={11} /></button>
            </span>
          ))}
        </div>
      )}
      <div className="toolbar">
        <input className="input" style={{ height: 26 }} placeholder="Filter this folder" value={filter} onChange={(e) => setFilter(e.target.value)} />
        <button className="icon-btn" title={showHidden ? "Hide dotfiles" : "Show dotfiles"} aria-label="Toggle hidden files" onClick={() => setShowHidden(!showHidden)}>{showHidden ? <Eye size={15} /> : <EyeOff size={15} />}</button>
        <button className="icon-btn" title="New file" aria-label="New file" onClick={() => setNewItem("file")}><FilePlus size={15} /></button>
        <button className="icon-btn" title="New folder" aria-label="New folder" onClick={() => setNewItem("folder")}><FolderPlus size={15} /></button>
        <button className="icon-btn" title="Upload files from this computer (or drop files and folders here)" aria-label="Upload files" onClick={pickUpload}><Upload size={15} /></button>
        <button className="icon-btn" title="Upload a folder from this computer" aria-label="Upload folder" onClick={pickUploadFolder}><FolderUp size={15} /></button>
        {ssh && <button className="icon-btn" title="Open terminal in this folder" aria-label="Open terminal here" onClick={() => openTerminal(sid, s.cwd)}><SquareTerminal size={15} /></button>}
        {ssh && <button className="icon-btn" title="Send files or folders to another server" aria-label="Send to another server" onClick={() => setPickSend(true)}><ArrowRightLeft size={15} /></button>}
        {ssh && <button className={`icon-btn ${finding ? "on" : ""}`} title="Find files in this folder and below" aria-label="Find files" onClick={() => (finding ? closeFind() : setFinding(true))}><Search size={15} /></button>}
      </div>
      {finding && (
        <form className="find-bar" onSubmit={(e) => { e.preventDefault(); void findNow(); }}>
          <input className="input" autoFocus placeholder={`Find by name in ${s.cwd === "/" ? "/" : s.cwd.split("/").pop()} and below…`} value={query} onChange={(e) => setQuery(e.target.value)} onKeyDown={(e) => e.key === "Escape" && closeFind()} />
          <button className="btn" disabled={!query.trim() || searching}>{searching ? "Searching…" : "Find"}</button>
        </form>
      )}
      {(uploadJob || copyJob || folderJob) && (
        <div className="job-stack">
          {uploadJob && <JobProgress job={uploadJob} compact />}
          {folderJob && <JobProgress job={folderJob} compact />}
          {copyJob && !copyOf && <JobProgress job={copyJob} compact />}
        </div>
      )}
      <div className="list" role="listbox" tabIndex={0} onKeyDown={(e) => {
        if (e.key === "Enter" && selected) openEntry(selected);
        if (((e.key === "Backspace" && mod(e)) || (!isMac && e.key === "Delete")) && selected) remove(selected);
      }}>
        {finding && (findErr || found) && (
          <div className="find-results">
            {findErr && <div className="empty"><p>{findErr}</p></div>}
            {found && found.files.length === 0 && <div className="empty"><p>Nothing named like “{query}” in this folder or below.</p></div>}
            {found?.files.map((f) => (
              <button key={f.path} className="entry find-hit" onClick={() => (f.dir ? (closeFind(), go(f.path)) : openFile(sid, f.path))} title={f.path}>
                {f.dir ? <Folder size={15} color="var(--amber)" /> : <FileText size={15} className="faint" />}
                <span className="n mono">{f.path.startsWith(s.cwd === "/" ? "/" : s.cwd + "/") ? f.path.slice(s.cwd === "/" ? 1 : s.cwd.length + 1) : f.path}</span>
              </button>
            ))}
            {found?.truncated && <div className="faint" style={{ padding: "8px 12px", fontSize: 12 }}>Showing the first 200 matches; type more of the name to narrow it down.</div>}
          </div>
        )}
        {!(finding && (findErr || found)) && error && <div className="empty"><div><h3>Can't read this folder</h3><p>{error}</p><button className="btn" onClick={() => go(s.info.home)}>Go to home folder</button></div></div>}
        {!(finding && (findErr || found)) && !error && visible.length === 0 && !loading && <div className="empty"><p>{filter ? "No matches in this folder." : "This folder is empty. Drop files here to upload."}</p></div>}
        {!(finding && (findErr || found)) && visible.map((e) => (
          <div
            key={e.path}
            role="option"
            aria-selected={sel === e.path}
            className={`entry ${e.kind === "dir" ? "dir" : ""} ${e.hidden ? "hidden" : ""} ${sel === e.path ? "sel" : ""}`}
            onClick={() => setSel(e.path)}
            onDoubleClick={() => openEntry(e)}
            title={`${e.name}\n${ssh ? `${e.mode}  ${e.owner ?? e.uid ?? ""}:${e.group ?? e.gid ?? ""}\n` : ""}Modified ${relativeTime(e.mtime)}`}
          >
            {e.kind === "dir" ? <Folder size={15} color="var(--amber)" /> : e.kind === "symlink" ? <Link2 size={15} className="faint" /> : <FileText size={15} className="faint" />}
            <span className="n">{e.name}</span>
            <span className="s">{e.kind === "dir" ? "" : bytes(e.size)}</span>
            {/* Permissions, then this item's actions; the click doesn't select or open the row. */}
            <span className="entry-side" onClick={(ev) => ev.stopPropagation()} onDoubleClick={(ev) => ev.stopPropagation()}>
              <span className="mode mono">{ssh ? e.mode : relativeTime(e.mtime)}</span>
              <button className="row-btn" title="Rename" aria-label={`Rename ${e.name}`} onClick={() => setRename(e)}><Pencil size={13} /></button>
              {ssh && <button className="row-btn" title="Permissions" aria-label={`Permissions of ${e.name}`} onClick={() => setChmodOf(e)}><Lock size={13} /></button>}
              <button className="row-btn danger" title="Delete" aria-label={`Delete ${e.name}`} onClick={() => remove(e)}><Trash2 size={13} /></button>
            </span>
          </div>
        ))}
      </div>
      {selected && (
        <div className="toolbar" style={{ borderTop: "1px solid var(--slate-4)", borderBottom: 0 }}>
          {ssh ? <span className="mono faint" title="Owner:group, last modified">{selected.owner ?? selected.uid}:{selected.group ?? selected.gid} · {relativeTime(selected.mtime)}</span> : <span className="mono faint">{selected.kind === "dir" ? "Folder" : bytes(selected.size)} · {relativeTime(selected.mtime)}</span>}
          <span className="spacer" />
          {selected.kind !== "dir" && <button className="icon-btn" title="Download" aria-label="Download" onClick={() => download(selected)}><Download size={14} /></button>}
          {selected.kind === "dir" && ssh && <button className="btn" style={{ height: 26 }} disabled={!!folderJob} title="Download this folder as one .tar.gz archive" onClick={() => downloadFolder(selected)}><Download size={13} /> Download folder</button>}
        </div>
      )}
      {rename && (
        <NameDialog
          title={`Rename ${rename.name}`}
          initial={rename.name}
          action="Rename"
          onClose={() => setRename(null)}
          onSubmit={async (name) => {
            await api.rename(sid, rename.path, joinPath(parentPath(rename.path), name));
            setRename(null);
            load();
          }}
        />
      )}
      {newItem && (
        <NameDialog
          title={newItem === "file" ? "New file" : "New folder"}
          initial=""
          action="Create"
          onClose={() => setNewItem(null)}
          onSubmit={async (name) => {
            const path = joinPath(s.cwd, name);
            if (newItem === "file") await api.createFile(sid, path);
            else await api.mkdir(sid, path);
            setNewItem(null);
            await load();
            if (newItem === "file") openEntry({ path, name, kind: "file" } as FileEntry);
          }}
        />
      )}
      {copyOf && <CopyDialog s={s} entries={copyOf} onClose={() => setCopyOf(null)} />}
      {pickSend && <PickSourceDialog s={s} initial={selected} onClose={() => setPickSend(false)} onPick={(es) => { setPickSend(false); setCopyOf(es); }} />}
      {chmodOf && <ChmodDialog entry={chmodOf} onClose={() => setChmodOf(null)} onSubmit={async (mode) => { await api.chmod(sid, chmodOf.path, mode); setChmodOf(null); load(); }} />}
    </section>
  );
}

function NameDialog(p: { title: string; initial: string; action: string; onClose: () => void; onSubmit: (name: string) => Promise<void> }) {
  const [v, setV] = useState(p.initial);
  const [err, setErr] = useState<string | null>(null);
  const valid = v.trim() && !v.includes("/");
  const submit = async () => {
    if (!valid) return;
    try {
      await p.onSubmit(v.trim());
    } catch (e) {
      setErr(asApiError(e).message);
    }
  };
  return (
    <Modal title={p.title} onClose={p.onClose} footer={<><button className="btn" onClick={p.onClose}>Cancel</button><button className="btn primary" disabled={!valid} onClick={submit}>{p.action}</button></>}>
      {err && <div className="alert">{err}</div>}
      <input className="input mono" autoFocus value={v} onChange={(e) => setV(e.target.value)} onKeyDown={(e) => e.key === "Enter" && submit()} />
    </Modal>
  );
}

/**
 * Step one of "Send to server" when nothing is selected: choose the file or
 * folder on this server. Click selects, double-click (or the arrow) opens a
 * folder; "Next" goes on to the destination.
 */
function PickSourceDialog({ s, initial, onClose, onPick }: { s: SessionState; initial: FileEntry | null; onClose: () => void; onPick: (e: FileEntry[]) => void }) {
  const [dir, setDir] = useState(s.cwd);
  const [list, setList] = useState<FileEntry[] | null>(null);
  const [err, setErr] = useState<string | null>(null);
  // Chosen items, by path; they can come from different folders.
  const [chosen, setChosen] = useState<Map<string, FileEntry>>(() => new Map(initial ? [[initial.path, initial]] : []));
  useEffect(() => {
    let alive = true;
    setList(null);
    api.listDir(s.info.id, dir).then(
      (l) => alive && (setList(l.filter((f) => !f.hidden)), setErr(null)),
      (e) => alive && setErr(asApiError(e).message),
    );
    return () => { alive = false; };
  }, [dir]);
  const toggle = (f: FileEntry) => setChosen((m) => {
    const n = new Map(m);
    if (n.has(f.path)) n.delete(f.path);
    else n.set(f.path, f);
    return n;
  });
  const here = list ?? [];
  const allHere = here.length > 0 && here.every((f) => chosen.has(f.path));
  const up = parentPath(dir);
  return (
    <Modal
      title="Send to another server"
      subtitle={<>Step 1 of 2: choose what to send from <b>{s.profile.name}</b></>}
      onClose={onClose}
      footer={<>
        <span className="left faint">{chosen.size ? `${chosen.size} selected` : "Nothing selected"}</span>
        {chosen.size > 0 && <button className="btn ghost" onClick={() => setChosen(new Map())}>Clear</button>}
        <button className="btn" onClick={onClose}>Cancel</button>
        <button className="btn primary" disabled={chosen.size === 0} onClick={() => onPick([...chosen.values()])}>Next{chosen.size > 1 ? ` (${chosen.size})` : ""}</button>
      </>}
    >
      {err && <div className="alert">{err}</div>}
      <div className="pick-head">
        <div className="crumbs mono">
          {breadcrumbs(dir).map((b) => <button key={b.path} onClick={() => setDir(b.path)}>{b.name}</button>)}
        </div>
        {here.length > 0 && (
          <label className="check">
            <input type="checkbox" checked={allHere} onChange={() => setChosen((m) => {
              const n = new Map(m);
              for (const f of here) allHere ? n.delete(f.path) : n.set(f.path, f);
              return n;
            })} />
            <span>All here</span>
          </label>
        )}
      </div>
      <div className="folder-pick tall" role="listbox" aria-multiselectable="true" aria-label="Files and folders">
        {dir !== "/" && <button className="folder-row" onClick={() => setDir(up)}><FolderUp size={13} /> ..</button>}
        {!list && !err && <div className="faint" style={{ padding: "6px 8px" }}>Loading…</div>}
        {here.map((f) => (
          <div key={f.path} role="option" aria-selected={chosen.has(f.path)} className={`folder-row ${chosen.has(f.path) ? "on" : ""}`} onClick={() => toggle(f)} onDoubleClick={() => f.kind === "dir" && setDir(f.path)}>
            <input type="checkbox" checked={chosen.has(f.path)} onChange={() => toggle(f)} onClick={(e) => e.stopPropagation()} aria-label={`Send ${f.name}`} />
            {f.kind === "dir" ? <Folder size={13} /> : <FileText size={13} />}
            <span style={{ flex: 1, overflow: "hidden", textOverflow: "ellipsis" }}>{f.name}</span>
            <span className="faint">{f.kind === "dir" ? "folder" : bytes(f.size)}</span>
            {f.kind === "dir" && <button className="row-btn" title={`Open ${f.name}`} aria-label={`Open ${f.name}`} onClick={(e) => { e.stopPropagation(); setDir(f.path); }}><ChevronRight size={13} /></button>}
          </div>
        ))}
        {list?.length === 0 && <div className="faint" style={{ padding: "6px 8px" }}>This folder is empty</div>}
      </div>
      <p className="hint" style={{ marginTop: 8 }}>Tick one or more files and folders (a folder is sent with everything in it). Open a folder with › or a double-click; ticked items stay ticked when you move between folders.</p>
    </Modal>
  );
}

/**
 * Send a file or folder to another server: any saved SSH server, connected
 * here on the spot if it isn't yet. The copy is a job: it shows files and
 * bytes as they go, and keeps going if the dialog is closed.
 */
function CopyDialog({ s, entries, onClose }: { s: SessionState; entries: FileEntry[]; onClose: () => void }) {
  const what = entries.length === 1 ? entries[0].name : `${entries.length} items`;
  const { sessions, profiles, connecting } = useStore();
  const key = `copy:${s.info.id}`;
  const job = useRunningJob(key);
  // Saved SSH servers other than this one; a connected one is used as it is.
  const candidates = profiles.filter((p) => (p.protocol ?? "sftp") === "sftp" && p.id !== s.profile.id);
  const sessionOf = (pid: string) => Object.values(sessions).find((o) => o.profile.id === pid && !o.lost);
  const firstConnected = candidates.find((p) => sessionOf(p.id));
  const [pid, setPid] = useState(firstConnected?.id ?? candidates[0]?.id ?? "");
  const dst = pid ? sessionOf(pid) : undefined;
  const [dir, setDir] = useState(dst?.cwd ?? "");
  const [folders, setFolders] = useState<string[] | null>(null);
  const [overwrite, setOverwrite] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const valid = !!dst && dir.startsWith("/");

  // The destination's folders, to click through instead of typing a path.
  useEffect(() => {
    if (!dst || !dir.startsWith("/")) return setFolders(null);
    let alive = true;
    const t = setTimeout(() => {
      api.listDir(dst.info.id, dir).then(
        (l) => alive && setFolders(l.filter((f) => f.kind === "dir").map((f) => f.name).sort()),
        () => alive && setFolders(null),
      );
    }, 250);
    return () => {
      alive = false;
      clearTimeout(t);
    };
  }, [dst?.info.id, dir]);

  async function connectTarget() {
    setErr(null);
    const here = s.info.id;
    await useStore.getState().connect(pid);
    // Connecting switches to the new server; stay here, in this dialog.
    useStore.setState({ active: here, globalView: null });
    const o = sessionOf(pid) ?? Object.values(useStore.getState().sessions).find((x) => x.profile.id === pid && !x.lost);
    if (o) setDir(o.cwd);
  }

  async function go() {
    if (!valid || !dst) return;
    setErr(null);
    const target = dst.info.id;
    try {
      // One job for all of them: one progress, one Stop.
      await runJob(
        async (p) => {
          const total = { files: 0, dirs: 0, bytes: 0, skipped: 0 };
          for (const entry of entries) {
            const rep = await api.copyBetween({ srcSessionId: s.info.id, srcPath: entry.path, dstSessionId: target, dstPath: joinPath(dir, entry.name), overwrite }, p);
            total.files += rep.files;
            total.dirs += rep.dirs;
            total.bytes += rep.bytes;
            total.skipped += rep.skipped;
          }
          return total;
        },
        {
          key,
          title: `Sending ${what} to ${dst.profile.name}`,
          sid: s.info.id,
          quietHere: false,
          done: (rep) => `${what} sent to ${dst.profile.name}: ${rep.files} ${rep.files === 1 ? "file" : "files"}, ${bytes(rep.bytes)}${rep.skipped ? `, ${rep.skipped} links or special files skipped` : ""}`,
        },
      );
      // The other server's file list shows the new item the next time it's opened.
      useStore.getState().patch(target, { cwd: dir });
      onClose();
    } catch (e) {
      setErr(asApiError(e).message);
    }
  }

  const up = dir.replace(/\/[^/]+\/?$/, "") || "/";
  return (
    <Modal
      title={`Send ${what} to another server`}
      subtitle={<>From <b>{s.profile.name}</b> <span className="mono">{entries.length === 1 ? entries[0].path : entries.map((e) => e.name).join(", ")}</span></>}
      onClose={onClose}
      footer={<>
        <button className="btn" onClick={onClose}>{job ? "Continue in the background" : "Cancel"}</button>
        <button className="btn primary" disabled={!valid || !!job} onClick={go}>{job ? "Sending…" : "Send"}</button>
      </>}
    >
      {err && <div className="alert">{err}</div>}
      {job && <div style={{ marginBottom: 12 }}><JobProgress job={job} /></div>}
      {candidates.length === 0 ? (
        <p className="note">Add the other server first (the "+" in the sidebar), then come back here.</p>
      ) : (
        <div className="form">
          <label className="field">
            <span>To server</span>
            <select className="input" value={pid} disabled={!!job} onChange={(e) => { setPid(e.target.value); setDir(sessionOf(e.target.value)?.cwd ?? ""); }}>
              {candidates.map((p) => <option key={p.id} value={p.id}>{p.name}{sessionOf(p.id) ? "" : " (not connected)"}</option>)}
            </select>
          </label>
          {!dst ? (
            <div className="notice" style={{ margin: 0, display: "flex", alignItems: "center", gap: 10 }}>
              <span style={{ flex: 1 }}>Orbital connects to it first; you stay here.</span>
              <button className="btn primary" disabled={!!connecting[pid]} onClick={connectTarget}>{connecting[pid] ? "Connecting…" : "Connect"}</button>
            </div>
          ) : (
            <>
              <label className="field">
                <span>Into folder</span>
                <input className="input mono" value={dir} disabled={!!job} onChange={(e) => setDir(e.target.value)} />
              </label>
              {folders && !job && (
                <div className="folder-pick">
                  {dir !== "/" && <button className="folder-row" onClick={() => setDir(up)}><FolderUp size={13} /> ..</button>}
                  {folders.map((f) => (
                    <button key={f} className="folder-row" onClick={() => setDir(joinPath(dir, f))}><Folder size={13} /> {f}</button>
                  ))}
                  {folders.length === 0 && <div className="faint" style={{ padding: "6px 8px" }}>No folders here</div>}
                </div>
              )}
              <span className="hint">Arrives {entries.length === 1 ? "as" : "in"} <span className="mono">{valid ? (entries.length === 1 ? joinPath(dir, entries[0].name) : dir) : "…"}</span>. Files travel through this computer; the two servers don't connect to each other, and no keys or passwords are sent to either.</span>
              <label className="check">
                <input type="checkbox" checked={overwrite} disabled={!!job} onChange={(e) => setOverwrite(e.target.checked)} />
                <span>Replace what already exists there</span>
              </label>
            </>
          )}
        </div>
      )}
    </Modal>
  );
}

function ChmodDialog(p: { entry: FileEntry; onClose: () => void; onSubmit: (mode: number) => Promise<void> }) {
  const [mode, setMode] = useState(p.entry.modeOctal);
  const [err, setErr] = useState<string | null>(null);
  const bits: [string, number][] = [["Read", 4], ["Write", 2], ["Execute", 1]];
  const who: [string, number][] = [["Owner", 6], ["Group", 3], ["Others", 0]];
  return (
    <Modal
      title={`Permissions for ${p.entry.name}`}
      subtitle={<span className="mono">{p.entry.owner ?? p.entry.uid}:{p.entry.group ?? p.entry.gid}</span>}
      onClose={p.onClose}
      footer={<><span className="mono left faint">{octal(mode)}</span><button className="btn" onClick={p.onClose}>Cancel</button><button className="btn primary" onClick={() => p.onSubmit(mode).catch((e) => setErr(asApiError(e).message))}>Apply</button></>}
    >
      {err && <div className="alert">{err}</div>}
      <table className="table">
        <thead><tr><th /><th>Read</th><th>Write</th><th>Execute</th></tr></thead>
        <tbody>
          {who.map(([w, shift]) => (
            <tr key={w}>
              <td>{w}</td>
              {bits.map(([b, v]) => {
                const bit = v << shift;
                return <td key={b}><input type="checkbox" aria-label={`${w} ${b}`} style={{ accentColor: "var(--amber)" }} checked={(mode & bit) !== 0} onChange={(e) => setMode(e.target.checked ? mode | bit : mode & ~bit)} /></td>;
              })}
            </tr>
          ))}
        </tbody>
      </table>
    </Modal>
  );
}

// ---------------------------------------------------------------- editor
export function EditorPane({ s }: { s: SessionState }) {
  const { patch, toast } = useStore();
  const monacoTheme = useResolvedTheme() === "light" ? "orbital-light" : "orbital-dark";
  const editorPrefs = useSettings();
  const sid = s.info.id;
  const file = s.files.find((f) => f.path === s.activeFile) ?? null;
  const [review, setReview] = useState<OpenFile | null>(null);
  const [versions, setVersions] = useState<OpenFile | null>(null);
  const saveRef = useRef<() => void>(() => {});

  const update = (path: string, p: Partial<OpenFile>) =>
    patch(sid, (cur) => ({ files: cur.files.map((f) => (f.path === path ? { ...f, ...p } : f)) }));

  async function close(f: OpenFile) {
    if (f.content !== f.original) {
      const ok = await confirm({ title: `Discard changes to ${baseName(f.path)}?`, body: "Your unsaved edits will be lost.", confirmLabel: "Discard", danger: true });
      if (!ok) return;
    }
    patch(sid, (cur) => {
      const files = cur.files.filter((x) => x.path !== f.path);
      return { files, activeFile: cur.activeFile === f.path ? files[files.length - 1]?.path ?? null : cur.activeFile };
    });
  }

  async function reload(f: OpenFile) {
    const fc = await api.readFile(sid, f.path);
    update(f.path, { content: fc.content, original: fc.content, mtime: fc.mtime, writable: fc.writable });
  }

  /** ⌘S: plain files save straight away; configs and root-owned files go through review. */
  async function save(f: OpenFile, force = false) {
    if (f.content === f.original) return;
    if (f.validator || !f.writable) return setReview(f);
    try {
      const mtime = await api.writeFile(sid, f.path, f.content, force ? null : f.mtime);
      update(f.path, { original: f.content, mtime });
      toast({ kind: "success", title: `Saved ${baseName(f.path)}` });
    } catch (e) {
      const err = asApiError(e);
      if (err.kind === "Conflict") {
        const ok = await confirm({
          title: `${baseName(f.path)} changed on the server`,
          body: "Someone else modified this file after you opened it. Overwrite their changes with yours?",
          confirmLabel: "Overwrite",
          danger: true,
        });
        if (ok) save(f, true);
      } else if (err.kind === "PermissionDenied") {
        update(f.path, { writable: false });
        setReview({ ...f, writable: false });
      } else toast({ kind: "error", title: "Save failed", body: err.message });
    }
  }
  saveRef.current = () => file && save(file);

  const onMount: OnMount = (editor, monaco) => {
    editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyS, () => saveRef.current());
  };

  if (!file) {
    return (
      <section className="editor">
        <div className="empty">
          <div>
            <FileText size={28} />
            <h3>Open a file to edit it here</h3>
            {isSsh(s) ? <p>Double-click a file. Config files like nginx, sshd and sudoers are checked before they go live, and the previous version is kept as a backup.</p> : <p>Double-click a text file to edit it. Saving replaces the file; there are no server-side checks or backups on this kind of connection.</p>}
          </div>
        </div>
      </section>
    );
  }

  const dirty = file.content !== file.original;
  return (
    <section className="editor" aria-label="Editor">
      <div className="file-tabs" role="tablist">
        {s.files.map((f) => (
          <div key={f.path} role="tab" aria-selected={f.path === file.path} className={`file-tab ${f.path === file.path ? "on" : ""}`} onClick={() => patch(sid, { activeFile: f.path })} title={f.path}>
            {baseName(f.path)}
            {f.content !== f.original ? <span className="dirty" aria-label="Unsaved" /> : null}
            <button className="icon-btn" style={{ width: 20, height: 20 }} aria-label={`Close ${baseName(f.path)}`} onClick={(e) => { e.stopPropagation(); close(f); }}><X size={12} /></button>
          </div>
        ))}
      </div>
      <div className="editor-bar">
        <span className="mono faint selectable" style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{file.path}</span>
        {!file.writable && <span className="badge amber"><Lock size={11} /> Needs sudo to save</span>}
        {file.validator && <span className="badge ok"><ShieldCheck size={11} /> Checked by {file.validator.name}</span>}
        <span style={{ flex: 1 }} />
        {isSsh(s) && <button className="icon-btn" title="Earlier versions (backups made by safe save)" aria-label="Earlier versions" onClick={() => setVersions(file)}><History size={14} /></button>}
        <button className="icon-btn" title="Reload from server" aria-label="Reload from server" onClick={() => reload(file)}><RefreshCw size={14} /></button>
        <button className="btn primary" disabled={!dirty} onClick={() => save(file)}><Save size={14} /> {file.validator || !file.writable ? "Review & save" : "Save"}</button>
      </div>
      <div style={{ flex: 1, minHeight: 0 }}>
        <Editor
          key={file.path}
          theme={monacoTheme}
          path={`${sid}${file.path}`}
          language={languageFor(file.path)}
          value={file.content}
          onChange={(v) => update(file.path, { content: v ?? "" })}
          onMount={onMount}
          options={{
            fontFamily: "JetBrains Mono", fontSize: editorPrefs.editorFontSize, lineHeight: Math.round(editorPrefs.editorFontSize * 1.55),
            minimap: { enabled: editorPrefs.editorMinimap }, wordWrap: editorPrefs.editorWordWrap ? "on" : "off",
            renderWhitespace: "boundary", scrollBeyondLastLine: false, smoothScrolling: true, padding: { top: 10 },
            automaticLayout: true, bracketPairColorization: { enabled: true }, stickyScroll: { enabled: true },
          }}
        />
      </div>
      {versions && <VersionsDialog s={s} file={versions} onClose={() => setVersions(null)} onRestored={async () => { const f = versions; setVersions(null); await reload(f); }} />}
      {review && <SaveReview s={s} file={review} onClose={() => setReview(null)} onSaved={(rep) => {
        update(review.path, { original: review.content, mtime: rep.newMtime, writable: review.writable });
        setReview(null);
      }} />}
    </section>
  );
}

function SaveReview({ s, file, onClose, onSaved }: { s: SessionState; file: OpenFile; onClose: () => void; onSaved: (r: SafeWriteReport) => void }) {
  const toast = useStore((x) => x.toast);
  const monacoTheme = useResolvedTheme() === "light" ? "orbital-light" : "orbital-dark";
  const [validate, setValidate] = useState(!!file.validator);
  const [sudo, setSudo] = useState(!file.writable);
  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState<{ title: string; output: string } | null>(null);
  // Dispose diff models ourselves, after detaching them (avoids Monaco's
  // "TextModel got disposed before DiffEditorWidget model got reset").
  const diffRef = useRef<Parameters<DiffOnMount>[0] | null>(null);
  useEffect(() => () => {
    const ed = diffRef.current;
    const m = ed?.getModel();
    ed?.setModel(null);
    m?.original.dispose();
    m?.modified.dispose();
  }, []);

  async function go() {
    setBusy(true);
    setFailure(null);
    try {
      const rep = await api.safeWrite({ sessionId: s.info.id, path: file.path, content: file.content, validate, privileged: sudo, expectedMtime: file.mtime });
      onSaved(rep);
      toast({
        kind: "success",
        title: `Saved ${baseName(file.path)}`,
        body: [rep.validator && `${rep.validator} check passed`, rep.backupPath && `Backup: ${rep.backupPath}`].filter(Boolean).join("\n"),
        action: rep.suggestedReload
          ? {
              label: `Reload ${rep.suggestedReload.replace(".service", "")}`,
              run: () =>
                api.serviceAction(s.info.id, rep.suggestedReload!, "reload")
                  .then(() => toast({ kind: "success", title: `Reloaded ${rep.suggestedReload}` }))
                  .catch((e) => toast({ kind: "error", title: "Reload failed", body: asApiError(e).message })),
            }
          : undefined,
      });
    } catch (e) {
      const err = asApiError(e);
      if (err.kind === "ValidationFailed") setFailure({ title: `${err.data.validator} rejected the change — the server still has the old version`, output: err.data.output });
      else if (err.kind === "Conflict") setFailure({ title: "The file changed on the server after you opened it. Reload it and reapply your edits.", output: "" });
      else setFailure({ title: "Save failed", output: err.message });
    } finally {
      setBusy(false);
    }
  }

  return (
    <Modal
      wide
      title={`Review changes to ${baseName(file.path)}`}
      subtitle={<span className="mono">{file.path}</span>}
      onClose={onClose}
      footer={
        <>
          <div className="left" style={{ display: "flex", gap: 18 }}>
            {file.validator && (
              <label className="check"><input type="checkbox" checked={validate} onChange={(e) => setValidate(e.target.checked)} /><span>Check with <span className="mono">{file.validator.command.replace(/\{(file|path)\}/g, baseName(file.path)) || "JSON parser"}</span></span></label>
            )}
            <label className="check"><input type="checkbox" checked={sudo} disabled={!s.info.elevation} onChange={(e) => setSudo(e.target.checked)} /><span>Save as root (sudo)</span></label>
          </div>
          <button className="btn" onClick={onClose}>Keep editing</button>
          <button className="btn primary" disabled={busy} onClick={go}><Save size={14} /> {busy ? "Saving…" : "Save to server"}</button>
        </>
      }
    >
      {failure && (
        <div className="alert">
          <b>{failure.title}</b>
          {failure.output && <pre className="output" style={{ marginTop: 8 }}>{failure.output}</pre>}
        </div>
      )}
      <div style={{ height: "52vh", border: "1px solid var(--slate-4)", borderRadius: 6, overflow: "hidden" }}>
        <DiffEditor
          theme={monacoTheme}
          original={file.original}
          modified={file.content}
          language={languageFor(file.path)}
          keepCurrentOriginalModel
          keepCurrentModifiedModel
          onMount={(ed) => (diffRef.current = ed)}
          options={{ readOnly: true, renderSideBySide: true, minimap: { enabled: false }, fontFamily: "JetBrains Mono", fontSize: 12.5, automaticLayout: true, scrollBeyondLastLine: false }}
        />
      </div>
      <p className="faint" style={{ margin: 0 }}>
        The current file is backed up next to it as <span className="mono">{baseName(file.path)}.orbital-bak-…</span> (last 5 kept).
        {validate && file.validator?.stage === "postInstall" && " If the check fails, the backup is restored automatically."}
      </p>
    </Modal>
  );
}

/** "orbital-bak-20261117093000" → a local date (backups are named in UTC). */
function backupDate(name: string): Date | null {
  const m = name.match(/\.(?:orbital|outpost)-bak-(\d{4})(\d{2})(\d{2})(\d{2})(\d{2})(\d{2})$/);
  return m ? new Date(Date.UTC(+m[1], +m[2] - 1, +m[3], +m[4], +m[5], +m[6])) : null;
}

/** Earlier versions of a config file, kept by safe save: compare one with the editor and put it back. */
function VersionsDialog({ s, file, onClose, onRestored }: { s: SessionState; file: OpenFile; onClose: () => void; onRestored: () => void }) {
  const toast = useStore((x) => x.toast);
  const monacoTheme = useResolvedTheme() === "light" ? "orbital-light" : "orbital-dark";
  const sid = s.info.id;
  const [privileged, setPrivileged] = useState(!file.writable);
  const [list, setList] = useState<string[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [sel, setSel] = useState<string | null>(null);
  const [content, setContent] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const diffRef = useRef<Parameters<DiffOnMount>[0] | null>(null);
  useEffect(() => () => {
    const ed = diffRef.current;
    const m = ed?.getModel();
    ed?.setModel(null);
    m?.original.dispose();
    m?.modified.dispose();
  }, []);

  useEffect(() => {
    api.listBackups(sid, file.path, privileged)
      .then((l) => { setList(l); setSel(l[0] ?? null); })
      .catch((e) => setError(asApiError(e).message));
  }, [privileged]);
  useEffect(() => {
    if (!sel) return;
    setContent(null);
    api.readBackup(sid, file.path, sel, privileged).then(setContent).catch((e) => {
      const err = asApiError(e);
      // Backups keep the original's permissions; root-only ones need sudo to read.
      if (!privileged && err.kind !== "InvalidInput" && s.info.elevation) setPrivileged(true);
      else setError(err.message);
    });
  }, [sel, privileged]);

  async function restore() {
    if (!sel) return;
    const when = backupDate(sel);
    const dirty = file.content !== file.original;
    const ok = await confirm({
      title: `Restore the version from ${when ? when.toLocaleString() : baseName(sel)}?`,
      body: `It goes through safe save: ${file.validator ? `checked by ${file.validator.name}, and ` : ""}the current version is kept as a backup, so you can undo this.${dirty ? " Your unsaved edits in the editor are discarded." : ""}`,
      confirmLabel: "Restore",
    });
    if (!ok) return;
    setBusy(true);
    try {
      const rep = await api.restoreBackup(sid, file.path, sel, privileged);
      toast({ kind: "success", title: `Restored ${baseName(file.path)}`, body: rep.suggestedReload ? `Reload ${rep.suggestedReload} to apply it.` : rep.validatorOutput ? `${rep.validator}: ${rep.validatorOutput.split("\n")[0]}` : undefined });
      onRestored();
    } catch (e) {
      const err = asApiError(e);
      toast({ kind: "error", title: "The version wasn't restored", body: err.data?.output ?? err.message });
    } finally {
      setBusy(false);
    }
  }

  return (
    <Modal title={`Earlier versions of ${baseName(file.path)}`} wide onClose={onClose}
      subtitle={<span className="mono">{file.path}</span>}
      footer={<><button className="btn" onClick={onClose}>Close</button><button className="btn primary" disabled={!sel || content === null || busy} onClick={restore}><RotateCcw size={13} /> {busy ? "Restoring…" : "Restore this version"}</button></>}>
      {error && <div className="notice bad-text" style={{ margin: 0 }}>{error}</div>}
      {list && list.length === 0 && <p className="muted" style={{ margin: 0 }}>No earlier versions yet. Each save through "Review & save" keeps the previous version (the last 5).</p>}
      {list && list.length > 0 && (
        <div className="versions">
          <div className="versions-list">
            {list.map((b) => {
              const d = backupDate(b);
              return (
                <button key={b} className={`nav-item ${sel === b ? "on" : ""}`} onClick={() => setSel(b)} title={b}>
                  <History size={13} />
                  <span className="grow">{d ? d.toLocaleString([], { dateStyle: "medium", timeStyle: "short" }) : baseName(b)}<small>{d ? relativeTime(Math.floor(d.getTime() / 1000)) : ""}</small></span>
                </button>
              );
            })}
          </div>
          <div className="versions-diff">
            <div className="versions-labels"><span>That version</span><span>Now in the editor</span></div>
            {content === null
              ? <div className="empty"><p>Loading…</p></div>
              : <DiffEditor theme={monacoTheme} original={content} modified={file.content} language={languageFor(file.path)} keepCurrentOriginalModel keepCurrentModifiedModel
                  onMount={(ed) => (diffRef.current = ed)}
                  options={{ readOnly: true, renderSideBySide: true, useInlineViewWhenSpaceIsLimited: false, minimap: { enabled: false }, fontFamily: "JetBrains Mono", fontSize: 12.5, automaticLayout: true, scrollBeyondLastLine: false }} />}
          </div>
        </div>
      )}
    </Modal>
  );
}

/** Bookmarked folders for a server (per profile, on this computer). */
function useBookmarks(profileId: string): [string[], (m: string[]) => void] {
  const key = `orbital.bookmarks.${profileId}`;
  const [marks, setMarks] = useState<string[]>(() => {
    try {
      return JSON.parse(localStorage.getItem(key) ?? "[]");
    } catch {
      return [];
    }
  });
  const save = (m: string[]) => {
    setMarks(m);
    try {
      localStorage.setItem(key, JSON.stringify(m));
    } catch {
      /* private mode: bookmarks last until the app closes */
    }
  };
  return [marks, save];
}
