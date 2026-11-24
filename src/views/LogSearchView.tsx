import { Search } from "lucide-react";
import { useState } from "react";
import { api, asApiError, LogSearch, SearchSource } from "../lib/api";
import { useKept } from "../lib/kept";
import { SessionState, useStore } from "../store";

// Search logs: a word or an error message across the journal or /var/log,
// over a time window. Fixed-string and case-insensitive; the newest 500 lines.

const WINDOWS: [number, string][] = [[1, "1 hour"], [6, "6 hours"], [24, "1 day"], [72, "3 days"], [168, "7 days"]];

/** Splits a line around the matches of `term` (case-insensitive) for highlighting. */
export function marks(line: string, term: string): { text: string; hit: boolean }[] {
  const t = term.trim().toLowerCase();
  if (!t) return [{ text: line, hit: false }];
  const out: { text: string; hit: boolean }[] = [];
  const low = line.toLowerCase();
  let i = 0;
  for (let j = low.indexOf(t); j >= 0; j = low.indexOf(t, i)) {
    if (j > i) out.push({ text: line.slice(i, j), hit: false });
    out.push({ text: line.slice(j, j + t.length), hit: true });
    i = j + t.length;
  }
  if (i < line.length) out.push({ text: line.slice(i), hit: false });
  return out;
}

export function LogSearchView({ s }: { s: SessionState }) {
  const sid = s.info.id;
  const [term, setTerm] = useKept(`${sid}:logsearch:term`, "");
  const [hours, setHours] = useKept(`${sid}:logsearch:hours`, 24);
  const [source, setSource] = useKept<SearchSource>(`${sid}:logsearch:source`, "journal");
  const [result, setResult] = useKept<{ r: LogSearch; term: string } | null>(`${sid}:logsearch:result`, null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const openFile = useStore((x) => x.openFile);
  const go = async () => {
    setBusy(true);
    setError(null);
    try {
      setResult({ r: await api.searchLogs(sid, term, hours, source), term });
    } catch (e) {
      setError(asApiError(e).message.replace(/^Invalid input: /, ""));
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="view">
      <form className="net-form" onSubmit={(e) => { e.preventDefault(); void go(); }}>
        <div className="net-inputs">
          <input className="input mono" value={term} onChange={(e) => setTerm(e.target.value)} placeholder="timeout, error 502, OOM, a request id…" autoFocus spellCheck={false} />
          <select className="input" style={{ maxWidth: 130 }} value={hours} onChange={(e) => setHours(Number(e.target.value))}>{WINDOWS.map(([h, l]) => <option key={h} value={h}>Last {l}</option>)}</select>
          <button className="btn primary" disabled={busy || term.trim().length < 2}><Search size={13} /> {busy ? "Searching…" : "Search"}</button>
        </div>
        <div className="segmented" style={{ alignSelf: "flex-start" }}>
          <button type="button" className={source === "journal" ? "on" : ""} onClick={() => setSource("journal")}>System journal</button>
          <button type="button" className={source === "files" ? "on" : ""} onClick={() => setSource("files")}>Files in /var/log</button>
        </div>
        <p className="faint net-hint">Finds the text as typed (not a pattern), in any case. {source === "files" ? "Searches .log files, syslog and messages changed in that time." : "Includes every service that logs to systemd."}</p>
      </form>
      <div className="scroll">
        {error && <div className="alert" style={{ margin: 16 }}>{error}</div>}
        {result && (
          <>
            <div className="net-history-head">
              <span className="faint">{result.r.total === 0 ? `Nothing matched “${result.term}”.` : `${result.r.total} matching line${result.r.total === 1 ? "" : "s"}${result.r.truncated ? `; the newest ${result.r.hits.length} shown` : ""}`}</span>
            </div>
            <div className="log-hits mono">
              {result.r.hits.map((h, i) => (
                <div key={i} className="log-hit">
                  {h.file && <button className="log-file" title="Open this file" onClick={() => void openFile(sid, h.file)}>{h.file.replace(/^\/var\/log\//, "")}</button>}
                  <span>{marks(h.line, result.term).map((m, j) => (m.hit ? <mark key={j}>{m.text}</mark> : <span key={j}>{m.text}</span>))}</span>
                </div>
              ))}
            </div>
          </>
        )}
      </div>
    </div>
  );
}
