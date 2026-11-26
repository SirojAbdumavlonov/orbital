import Editor, { OnMount } from "@monaco-editor/react";
import { Copy, History, Lock, Play, Unlock } from "lucide-react";
import { useMemo, useRef, useState } from "react";
import { api, asApiError, DbKind, SqlResult } from "../lib/api";
import "../lib/monaco";
import { useResolvedTheme, useSettings } from "../lib/settings";
import { SessionState, useStore } from "../store";
import { confirm, Modal } from "./ui";

const KEEP = 20;
const histKey = (profile: string, kind: DbKind, db: string) => `orbital.sql.${profile}.${kind}.${db}`;
function loadHistory(key: string): string[] {
  try {
    return JSON.parse(localStorage.getItem(key) ?? "[]");
  } catch {
    return [];
  }
}

/** Rows as tab-separated text, for pasting into a spreadsheet. */
function asTsv(r: SqlResult): string {
  const cell = (v: string | null) => (v ?? "").replace(/[\t\n]/g, " ");
  return [r.columns.join("\t"), ...r.rows.map((row) => row.map(cell).join("\t"))].join("\n");
}

/**
 * SQL console: run a query against one database on the server or in its
 * container. Read-only by default: the database itself refuses writes.
 */
export function SqlConsole({ s, kind, container, db, onClose }: { s: SessionState; kind: DbKind; container: string | null; db: string; onClose: () => void }) {
  const toast = useStore((x) => x.toast);
  const theme = useResolvedTheme() === "light" ? "orbital-light" : "orbital-dark";
  const fontSize = useSettings((x) => x.editorFontSize);
  const key = histKey(s.profile.id, kind, db);
  const [history, setHistory] = useState<string[]>(() => loadHistory(key));
  const [query, setQuery] = useState(history[0] ?? (kind === "postgres" ? "select table_name from information_schema.tables where table_schema = 'public' order by 1;" : "show tables;"));
  const [readOnly, setReadOnly] = useState(true);
  const [busy, setBusy] = useState(false);
  const [r, setR] = useState<SqlResult | null>(null);
  const [showHistory, setShowHistory] = useState(false);
  const runRef = useRef<() => void>(() => {});

  async function run() {
    const q = query.trim();
    if (!q || busy) return;
    setBusy(true);
    try {
      setR(await api.sqlQuery(s.info.id, kind, container, db, q, readOnly));
      const next = [q, ...history.filter((h) => h !== q)].slice(0, KEEP);
      setHistory(next);
      try { localStorage.setItem(key, JSON.stringify(next)); } catch { /* private mode */ }
    } catch (e) {
      toast({ kind: "error", title: "The query didn't run", body: asApiError(e).message });
    } finally {
      setBusy(false);
    }
  }
  runRef.current = () => void run();
  const onMount: OnMount = (editor, monaco) => {
    editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.Enter, () => runRef.current());
    editor.focus();
  };

  async function toggleWrite() {
    if (!readOnly) return setReadOnly(true);
    const ok = await confirm({
      title: "Allow changes to the database?",
      body: <>Queries can then insert, update and delete data in <span className="mono">{db}</span>, and drop tables. There is no undo; make a backup first if in doubt.</>,
      confirmLabel: "Allow changes",
      danger: true,
    });
    if (ok) setReadOnly(false);
  }

  const widths = useMemo(() => (r?.columns ?? []).map((c, i) => Math.min(360, Math.max(c.length, ...(r?.rows.slice(0, 50).map((row) => (row[i] ?? "NULL").length) ?? [0])) * 7.5 + 24)), [r]);
  const label = kind === "postgres" ? "PostgreSQL" : "MySQL";
  return (
    <Modal wide keepOnOutsideClick title={`${label}: ${db}`} subtitle={<span className="mono">{s.profile.name}{container ? ` · container ${container}` : ""}</span>} onClose={onClose}
      footer={<>
        <button className={`btn ${readOnly ? "" : "danger"}`} onClick={() => void toggleWrite()} title={readOnly ? "Writes are refused by the database" : "Writes are allowed"}>
          {readOnly ? <><Lock size={13} /> Read-only</> : <><Unlock size={13} /> Changes allowed</>}
        </button>
        <span className="spacer" />
        <button className="btn" onClick={onClose}>Close</button>
        <button className="btn primary" disabled={busy || !query.trim()} onClick={() => void run()}><Play size={13} /> {busy ? "Running…" : "Run"} <span className="kbd">⌘↵</span></button>
      </>}>
      <div className="sql-console">
        <div className="sql-editor">
          <Editor theme={theme} language="sql" value={query} onChange={(v) => setQuery(v ?? "")} onMount={onMount}
            options={{ fontFamily: "JetBrains Mono", fontSize, minimap: { enabled: false }, scrollBeyondLastLine: false, automaticLayout: true, wordWrap: "on", lineNumbers: "off", padding: { top: 8 } }} />
          {history.length > 0 && (
            <div className="sql-history">
              <button className="icon-btn" title="Recent queries" aria-label="Recent queries" onClick={() => setShowHistory(!showHistory)}><History size={14} /></button>
              {showHistory && (
                <div className="menu sql-history-menu" role="menu">
                  {history.map((h) => <button key={h} role="menuitem" className="menu-item mono" onClick={() => { setQuery(h); setShowHistory(false); }}>{h.length > 120 ? h.slice(0, 120) + "…" : h}</button>)}
                </div>
              )}
            </div>
          )}
        </div>
        <div className="sql-status">
          {r && (r.error ? <span className="bad-text">Error</span> : r.columns.length ? <span>{r.rows.length.toLocaleString()} rows{r.truncated ? " (cut off)" : ""}</span> : <span className="ok-text">Done</span>)}
          {r && <span className="faint">{r.durationMs} ms</span>}
          <span className="spacer" />
          {r && r.columns.length > 0 && <button className="btn" onClick={() => { navigator.clipboard?.writeText(asTsv(r)).catch(() => {}); toast({ kind: "success", title: "Rows copied", body: "Tab-separated, ready to paste into a spreadsheet." }); }}><Copy size={13} /> Copy rows</button>}
        </div>
        <div className="sql-result">
          {!r && <div className="empty"><p>Write a query and press Run (⌘↵). {readOnly ? "Read-only: the database refuses changes." : "Changes are allowed."}</p></div>}
          {r?.error && <pre className="output sql-error">{r.error}</pre>}
          {r?.text && <pre className="output">{r.text}</pre>}
          {r && r.columns.length > 0 && (
            <table className="table sql-table">
              <thead><tr>{r.columns.map((c, i) => <th key={i} style={{ minWidth: widths[i] }}>{c}</th>)}</tr></thead>
              <tbody>
                {r.rows.map((row, i) => (
                  <tr key={i}>{row.map((v, j) => <td key={j} className="mono" title={v ?? "NULL"}>{v == null ? <span className="faint">NULL</span> : v}</td>)}</tr>
                ))}
              </tbody>
            </table>
          )}
        </div>
        {r?.truncated && <p className="faint" style={{ margin: 0, fontSize: 12 }}>Only the first rows are shown. Add a LIMIT, or back the database up to get everything.</p>}
      </div>
    </Modal>
  );
}
