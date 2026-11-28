import { CornerDownLeft, Folder, History } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { api, asApiError } from "../lib/api";
import { remember, resolvePath, splitForSuggest, suggest } from "../lib/goto";
import { parentPath } from "../lib/format";
import { KEYS } from "../lib/platform";
import { Modal } from "./ui";

// Files → Go to Folder (⌘⇧G, as in the Finder): type or paste a path, with
// folders suggested as you type (Tab completes, ↑↓ pick). `~` is the home
// folder and a relative path starts at the current one. A file's path opens
// the file in its folder. Recent folders are kept per server on this computer.

const recentKey = (profileId: string) => `orbital.goto.${profileId}`;
const readRecent = (profileId: string): string[] => {
  try { const v = JSON.parse(localStorage.getItem(recentKey(profileId)) ?? "[]"); return Array.isArray(v) ? v.filter((x) => typeof x === "string") : []; } catch { return []; }
};

export function GoToFolder({ sid, profileId, cwd, home, onGo, onOpenFile, onClose }: {
  sid: string; profileId: string; cwd: string; home: string;
  onGo: (path: string) => void; onOpenFile: (path: string) => void; onClose: () => void;
}) {
  const [v, setV] = useState(cwd === "/" ? "/" : `${cwd}/`);
  const [names, setNames] = useState<{ dir: string; list: string[] } | null>(null);
  const [pick, setPick] = useState(-1);
  const [problem, setProblem] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const recent = readRecent(profileId).filter((p) => p !== cwd);
  const input = useRef<HTMLInputElement>(null);
  const { dir, prefix } = splitForSuggest(v, cwd, home);

  // The folders in the folder being typed into (listed once per folder).
  useEffect(() => {
    if (names?.dir === dir) return;
    const t = setTimeout(() => {
      api.listDir(sid, dir).then(
        (l) => setNames({ dir, list: l.filter((e) => e.kind === "dir" || e.kind === "symlink").map((e) => e.name) }),
        () => setNames({ dir, list: [] }),
      );
    }, 120);
    return () => clearTimeout(t);
  }, [sid, dir]);
  const hits = names?.dir === dir ? suggest(names.list, prefix) : [];
  useEffect(() => setPick(-1), [v]);

  const typedDir = v.trim().endsWith("/") ? v.trim() : v.trim().slice(0, v.trim().lastIndexOf("/") + 1);
  const complete = (name: string) => { setV(`${typedDir}${name}/`); input.current?.focus(); };

  async function go(raw: string) {
    const path = resolvePath(raw, cwd, home);
    setBusy(true);
    setProblem(null);
    try {
      await api.listDir(sid, path);
      try { localStorage.setItem(recentKey(profileId), JSON.stringify(remember(readRecent(profileId), path))); } catch { /* this run only */ }
      onGo(path);
    } catch (e) {
      // Not a folder: maybe a file, which opens in its folder.
      const parent = parentPath(path);
      const file = path.split("/").pop() ?? "";
      const there = await api.listDir(sid, parent).catch(() => null);
      const entry = there?.find((x) => x.name === file);
      if (entry && entry.kind !== "dir") {
        onGo(parent);
        onOpenFile(path);
      } else {
        const msg = asApiError(e).message;
        setProblem(there ? `There's no folder ${path}.` : /denied|permission/i.test(msg) ? `${path}: no permission to open it.` : `There's no folder ${path}.`);
      }
    } finally {
      setBusy(false);
    }
  }

  return (
    <Modal title="Go to folder" subtitle={<>Type or paste a path. <span className="mono">~</span> is your home folder; a path without <span className="mono">/</span> starts here. Tab completes.</>} onClose={onClose}
      footer={<><span className="faint" style={{ marginRight: "auto", fontSize: 12 }}>{KEYS.gotoFolder}</span><button className="btn" onClick={onClose}>Cancel</button><button className="btn primary" disabled={busy || !v.trim()} onClick={() => void go(v)}>{busy ? "Opening…" : "Go"}</button></>}>
      <div className="goto">
        <input ref={input} className="input mono" autoFocus onFocus={(e) => e.currentTarget.select()} spellCheck={false} value={v} aria-label="Folder path"
          onChange={(e) => { setV(e.target.value); setProblem(null); }}
          onKeyDown={(e) => {
            if (e.key === "Tab" && hits.length) { e.preventDefault(); complete(hits[Math.max(0, pick)]); }
            else if (e.key === "ArrowDown" && hits.length) { e.preventDefault(); setPick((p) => Math.min(hits.length - 1, p + 1)); }
            else if (e.key === "ArrowUp" && hits.length) { e.preventDefault(); setPick((p) => Math.max(-1, p - 1)); }
            else if (e.key === "Enter") { e.preventDefault(); if (pick >= 0) complete(hits[pick]); else void go(v); }
          }} />
        <span className="faint goto-resolved mono">{resolvePath(v, cwd, home)}</span>
        {problem && <div className="notice bad-text" style={{ margin: 0 }}>{problem}</div>}
        {hits.length > 0 && (
          <div className="goto-list" role="listbox" aria-label="Folders">
            {hits.map((n, i) => (
              <button key={n} role="option" aria-selected={i === pick} className={`goto-item ${i === pick ? "on" : ""}`} onClick={() => complete(n)} onDoubleClick={() => void go(`${typedDir}${n}`)}>
                <Folder size={13} /> <span className="mono">{n}</span>{i === pick && <CornerDownLeft size={11} className="faint" />}
              </button>
            ))}
          </div>
        )}
        {recent.length > 0 && (
          <div className="goto-list">
            <span className="faint goto-head"><History size={12} /> Recent on this server</span>
            {recent.map((p) => <button key={p} className="goto-item" onClick={() => void go(p)}><Folder size={13} /> <span className="mono">{p}</span></button>)}
          </div>
        )}
      </div>
    </Modal>
  );
}
