import { ArrowUp, File, Folder, FolderOpen, Link2, PanelRightClose, RefreshCw } from "lucide-react";
import { api, FileEntry } from "../lib/api";
import { bytes, parentPath, relativeTime } from "../lib/format";
import { useRemote } from "../lib/remote";
import { SessionState, useStore } from "../store";

/** A folder listed beside a tool tab: browse it, open files in the editor below, without leaving the tab. */
export function FolderPanel({ s, path }: { s: SessionState; path: string }) {
  const { patch, openFile } = useStore();
  const sid = s.info.id;
  const { data, error, loading, reload } = useRemote(() => api.listDir(sid, path), [sid, path]);
  const go = (p: string) => patch(sid, { folderDock: { tab: s.tab, path: p } });
  const open = (e: FileEntry) => (e.kind === "dir" ? go(e.path) : openFile(sid, e.path));
  const entries = data ?? [];

  return (
    <section className="folder-panel" aria-label={`Folder ${path}`}>
      <div className="dock-head">
        <button className="icon-btn" aria-label="Up one folder" title="Up one folder" disabled={path === "/"} onClick={() => go(parentPath(path))}><ArrowUp size={14} /></button>
        <span className="mono folder-path" title={path}>{path}</span>
        <span className="spacer" />
        <button className="icon-btn" aria-label="Refresh" onClick={reload}><RefreshCw size={13} className={loading ? "spin" : ""} /></button>
        <button className="btn ghost" title="Show this folder in the Files tab" onClick={() => patch(sid, { tab: "files", cwd: path, folderDock: null })}><FolderOpen size={13} /> Open in Files</button>
        <button className="icon-btn" aria-label="Close the folder panel" title="Close" onClick={() => patch(sid, { folderDock: null })}><PanelRightClose size={15} /></button>
      </div>
      <div className="scroll">
        {error && <div className="empty" style={{ flex: "none" }}><p>{error.message}</p></div>}
        <table className="table">
          <tbody>
            {entries.map((e) => (
              <tr key={e.path} className="click" onClick={() => open(e)} title={e.kind === "dir" ? "Open folder" : "Open in the editor"}>
                <td style={{ width: 24 }}>{e.kind === "dir" ? <Folder size={14} color="var(--amber)" /> : e.kind === "symlink" ? <Link2 size={14} className="faint" /> : <File size={14} className="faint" />}</td>
                <td className="mono"><div className="cell-clip">{e.name}</div></td>
                <td className="num faint">{e.kind === "dir" ? "" : bytes(e.size)}</td>
                <td className="faint nowrap" style={{ textAlign: "right" }}>{relativeTime(e.mtime)}</td>
              </tr>
            ))}
          </tbody>
        </table>
        {data && data.length === 0 && <div className="empty" style={{ flex: "none" }}><p>This folder is empty.</p></div>}
      </div>
    </section>
  );
}
