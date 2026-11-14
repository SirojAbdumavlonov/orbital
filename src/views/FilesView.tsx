import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import {
  ArrowUp, Download, Eye, EyeOff, FilePlus, FileText, Folder, FolderPlus, Link2, Lock, Pencil, RefreshCw, Save,
  ShieldCheck, SquareTerminal, Trash2, Upload, X,
} from "lucide-react";
import Editor, { DiffEditor, DiffOnMount, OnMount } from "@monaco-editor/react";
import { useCallback, useEffect, useRef, useState } from "react";
import { confirm, Modal } from "../components/ui";
import { api, asApiError, FileEntry, SafeWriteReport } from "../lib/api";
import { baseName, breadcrumbs, bytes, joinPath, octal, parentPath, relativeTime } from "../lib/format";
import { languageFor } from "../lib/language";
import "../lib/monaco";
import { OpenFile, SessionState, useStore } from "../store";

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
  const { patch, toast, openTerminal } = useStore();
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

  useEffect(() => { load(s.cwd); setSel(null); setFilter(""); }, [s.cwd]);
  const go = (path: string) => patch(sid, { cwd: path });

  async function openEntry(e: FileEntry) {
    if (e.kind === "dir") return go(e.path);
    const existing = s.files.find((f) => f.path === e.path);
    if (existing) return patch(sid, { activeFile: e.path });
    try {
      const [fc, validator] = await Promise.all([api.readFile(sid, e.path), api.validatorFor(e.path)]);
      const file: OpenFile = { path: fc.path, content: fc.content, original: fc.content, mtime: fc.mtime, writable: fc.writable, validator };
      patch(sid, (cur) => ({ files: [...cur.files, file], activeFile: file.path }));
    } catch (err) {
      toast({ kind: "error", title: `Can't open ${e.name}`, body: asApiError(err).message });
    }
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
      try {
        const n = await api.upload(sid, p, joinPath(s.cwd, name));
        toast({ kind: "success", title: `Uploaded ${name}`, body: bytes(n) });
      } catch (err) {
        toast({ kind: "error", title: `Upload of ${name} failed`, body: asApiError(err).message });
      }
    }
    load();
  }

  async function pickUpload() {
    const picked = await openDialog({ multiple: true, title: `Upload to ${s.cwd}` });
    if (picked) uploadPaths(Array.isArray(picked) ? picked : [picked]);
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
        <button className="icon-btn" aria-label="Refresh" title="Refresh" onClick={() => load()}><RefreshCw size={14} className={loading ? "spin" : ""} /></button>
      </div>
      <div className="toolbar">
        <input className="input" style={{ height: 26 }} placeholder="Filter this folder" value={filter} onChange={(e) => setFilter(e.target.value)} />
        <button className="icon-btn" title={showHidden ? "Hide dotfiles" : "Show dotfiles"} aria-label="Toggle hidden files" onClick={() => setShowHidden(!showHidden)}>{showHidden ? <Eye size={15} /> : <EyeOff size={15} />}</button>
        <button className="icon-btn" title="New file" aria-label="New file" onClick={() => setNewItem("file")}><FilePlus size={15} /></button>
        <button className="icon-btn" title="New folder" aria-label="New folder" onClick={() => setNewItem("folder")}><FolderPlus size={15} /></button>
        <button className="icon-btn" title="Upload files (or drop them here)" aria-label="Upload" onClick={pickUpload}><Upload size={15} /></button>
        <button className="icon-btn" title="Open terminal in this folder" aria-label="Open terminal here" onClick={() => openTerminal(sid, s.cwd)}><SquareTerminal size={15} /></button>
      </div>
      <div className="list" role="listbox" tabIndex={0} onKeyDown={(e) => {
        if (e.key === "Enter" && selected) openEntry(selected);
        if ((e.key === "Backspace" && e.metaKey) && selected) remove(selected);
      }}>
        {error && <div className="empty"><div><h3>Can't read this folder</h3><p>{error}</p><button className="btn" onClick={() => go(s.info.home)}>Go to home folder</button></div></div>}
        {!error && visible.length === 0 && !loading && <div className="empty"><p>{filter ? "No matches in this folder." : "This folder is empty. Drop files here to upload."}</p></div>}
        {visible.map((e) => (
          <div
            key={e.path}
            role="option"
            aria-selected={sel === e.path}
            className={`entry ${e.kind === "dir" ? "dir" : ""} ${e.hidden ? "hidden" : ""} ${sel === e.path ? "sel" : ""}`}
            onClick={() => setSel(e.path)}
            onDoubleClick={() => openEntry(e)}
            title={`${e.mode}  ${e.owner ?? e.uid ?? ""}:${e.group ?? e.gid ?? ""}`}
          >
            {e.kind === "dir" ? <Folder size={15} color="var(--amber)" /> : e.kind === "symlink" ? <Link2 size={15} className="faint" /> : <FileText size={15} className="faint" />}
            <span className="n">{e.name}</span>
            <span className="s">{e.kind === "dir" ? "" : bytes(e.size)}</span>
            <span className="t">{relativeTime(e.mtime)}</span>
          </div>
        ))}
      </div>
      {selected && (
        <div className="toolbar" style={{ borderTop: "1px solid var(--slate-4)", borderBottom: 0 }}>
          <span className="mono faint" title="Permissions">{selected.mode}</span>
          <span className="spacer" />
          {selected.kind !== "dir" && <button className="icon-btn" title="Download" aria-label="Download" onClick={() => download(selected)}><Download size={14} /></button>}
          <button className="icon-btn" title="Rename" aria-label="Rename" onClick={() => setRename(selected)}><Pencil size={14} /></button>
          <button className="icon-btn" title="Permissions" aria-label="Permissions" onClick={() => setChmodOf(selected)}><Lock size={14} /></button>
          <button className="icon-btn" title="Delete" aria-label="Delete" onClick={() => remove(selected)}><Trash2 size={14} /></button>
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
function EditorPane({ s }: { s: SessionState }) {
  const { patch, toast } = useStore();
  const sid = s.info.id;
  const file = s.files.find((f) => f.path === s.activeFile) ?? null;
  const [review, setReview] = useState<OpenFile | null>(null);
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
            <p>Double-click a file. Config files like nginx, sshd and sudoers are checked before they go live, and the previous version is kept as a backup.</p>
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
        <button className="icon-btn" title="Reload from server" aria-label="Reload from server" onClick={() => reload(file)}><RefreshCw size={14} /></button>
        <button className="btn primary" disabled={!dirty} onClick={() => save(file)}><Save size={14} /> {file.validator || !file.writable ? "Review & save" : "Save"}</button>
      </div>
      <div style={{ flex: 1, minHeight: 0 }}>
        <Editor
          key={file.path}
          theme="outpost-dark"
          path={`${sid}${file.path}`}
          language={languageFor(file.path)}
          value={file.content}
          onChange={(v) => update(file.path, { content: v ?? "" })}
          onMount={onMount}
          options={{
            fontFamily: "JetBrains Mono", fontSize: 13, lineHeight: 20, minimap: { enabled: false },
            renderWhitespace: "boundary", scrollBeyondLastLine: false, smoothScrolling: true, padding: { top: 10 },
            automaticLayout: true, bracketPairColorization: { enabled: true }, stickyScroll: { enabled: true },
          }}
        />
      </div>
      {review && <SaveReview s={s} file={review} onClose={() => setReview(null)} onSaved={(rep) => {
        update(review.path, { original: review.content, mtime: rep.newMtime, writable: review.writable });
        setReview(null);
      }} />}
    </section>
  );
}

function SaveReview({ s, file, onClose, onSaved }: { s: SessionState; file: OpenFile; onClose: () => void; onSaved: (r: SafeWriteReport) => void }) {
  const toast = useStore((x) => x.toast);
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
          theme="outpost-dark"
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
        The current file is backed up next to it as <span className="mono">{baseName(file.path)}.outpost-bak-…</span> (last 5 kept).
        {validate && file.validator?.stage === "postInstall" && " If the check fails, the backup is restored automatically."}
      </p>
    </Modal>
  );
}
