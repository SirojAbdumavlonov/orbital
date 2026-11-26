import { FitAddon } from "@xterm/addon-fit";
import { Terminal } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";
import { Download, Pause, Play, RefreshCw, SkipBack, Trash2, Video } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { confirm } from "../components/ui";
import { api, asApiError, RecordingInfo } from "../lib/api";
import { bytes } from "../lib/format";
import { useSettings } from "../lib/settings";
import { useStore } from "../store";
import { parseCast, Cast } from "../lib/cast";

const SPEEDS = [1, 2, 4, 8];
/** 75 → "1:15", 3725 → "1:02:05". */
const clock = (secs: number) => {
  const s = Math.max(0, Math.floor(secs));
  const h = Math.floor(s / 3600), m = Math.floor((s % 3600) / 60), r = s % 60;
  return `${h ? `${h}:${String(m).padStart(2, "0")}` : m}:${String(r).padStart(2, "0")}`;
};
/** Long pauses (someone reading, or away) are shortened to this when playing. */
const IDLE_LIMIT = 2;

/** Plays an asciinema v2 recording in xterm: play/pause, speed, and a seek bar. */
function Player({ cast, onDone }: { cast: Cast; onDone?: () => void }) {
  const host = useRef<HTMLDivElement>(null);
  const term = useRef<Terminal | null>(null);
  const [playing, setPlaying] = useState(true);
  const [speed, setSpeed] = useState(1);
  const [pos, setPos] = useState(0);
  const posRef = useRef(0);
  const idx = useRef(0);
  const fontSize = useSettings((s) => s.terminalFontSize);

  useEffect(() => {
    const t = new Terminal({ cols: cast.width, rows: cast.height, fontFamily: "JetBrains Mono", fontSize, disableStdin: true, scrollback: 5000, convertEol: false });
    const fit = new FitAddon();
    t.loadAddon(fit);
    t.open(host.current!);
    term.current = t;
    return () => t.dispose();
  }, [cast]);

  /** Redraw from the start up to time `at` (seconds of the compressed timeline). */
  const seek = (at: number) => {
    const t = term.current;
    if (!t) return;
    t.reset();
    let i = 0;
    let out = "";
    while (i < cast.events.length && cast.events[i].t <= at) {
      const e = cast.events[i];
      if (e.kind === "o") out += e.data;
      else if (e.kind === "r") {
        if (out) { t.write(out); out = ""; }
        const [c, r] = e.data.split("x").map(Number);
        if (c && r) t.resize(c, r);
      }
      i++;
    }
    if (out) t.write(out);
    idx.current = i;
    posRef.current = at;
    setPos(at);
  };

  useEffect(() => {
    if (!playing) return;
    let last = performance.now();
    let raf = 0;
    const tick = (now: number) => {
      const dt = ((now - last) / 1000) * speed;
      last = now;
      const t = term.current;
      let at = posRef.current + dt;
      let out = "";
      while (idx.current < cast.events.length && cast.events[idx.current].t <= at) {
        const e = cast.events[idx.current];
        if (e.kind === "o") out += e.data;
        else if (e.kind === "r") {
          if (out) { t?.write(out); out = ""; }
          const [c, r] = e.data.split("x").map(Number);
          if (c && r) t?.resize(c, r);
        }
        idx.current++;
      }
      if (out) t?.write(out);
      if (idx.current >= cast.events.length) {
        at = cast.length;
        posRef.current = at;
        setPos(at);
        setPlaying(false);
        onDone?.();
        return;
      }
      posRef.current = at;
      setPos(at);
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, [playing, speed, cast]);

  return (
    <div className="player">
      <div className="player-screen" ref={host} />
      <div className="player-bar">
        <button className="icon-btn" aria-label={playing ? "Pause" : "Play"} onClick={() => { if (!playing && posRef.current >= cast.length) seek(0); setPlaying(!playing); }}>{playing ? <Pause size={15} /> : <Play size={15} />}</button>
        <button className="icon-btn" aria-label="From the start" onClick={() => { seek(0); setPlaying(true); }}><SkipBack size={14} /></button>
        <input type="range" className="player-seek" min={0} max={cast.length || 0.001} step={0.05} value={pos} aria-label="Position"
          onChange={(e) => { setPlaying(false); seek(Number(e.target.value)); }} />
        <span className="mono faint player-time">{clock(pos)} / {clock(Math.ceil(cast.length))}</span>
        <div className="segmented">{SPEEDS.map((x) => <button key={x} className={speed === x ? "on" : ""} onClick={() => setSpeed(x)}>{x}×</button>)}</div>
      </div>
    </div>
  );
}

/** Terminal recordings made on this computer: play, save as .cast, delete. */
export function RecordingsView() {
  const toast = useStore((x) => x.toast);
  const record = useSettings((s) => s.recordTerminals);
  const setSettings = useSettings((s) => s.set);
  const [list, setList] = useState<RecordingInfo[] | null>(null);
  const [sel, setSel] = useState<string | null>(null);
  const [text, setText] = useState<string | null>(null);
  const load = () => api.recordings().then(setList).catch((e) => { setList([]); toast({ kind: "error", title: "Couldn't list recordings", body: asApiError(e).message }); });
  useEffect(() => { void load(); }, []);
  useEffect(() => {
    if (!sel) return;
    setText(null);
    api.readRecording(sel).then(setText).catch((e) => toast({ kind: "error", title: "Couldn't open the recording", body: asApiError(e).message }));
  }, [sel]);
  const cast = useMemo(() => (text ? parseCast(text, IDLE_LIMIT) : null), [text]);
  const cur = list?.find((r) => r.name === sel);

  async function remove(r: RecordingInfo) {
    if (!(await confirm({ title: "Delete this recording?", body: `${r.title}, ${new Date(r.started * 1000).toLocaleString()}. It can't be recovered.`, confirmLabel: "Delete", danger: true }))) return;
    await api.deleteRecording(r.name);
    if (sel === r.name) { setSel(null); setText(null); }
    void load();
  }

  async function save(r: RecordingInfo) {
    try {
      const { save: saveDialog } = await import("@tauri-apps/plugin-dialog");
      const path = await saveDialog({ title: "Save recording", defaultPath: r.name });
      if (!path) return;
      await api.saveText(path, text ?? (await api.readRecording(r.name)));
      toast({ kind: "success", title: "Recording saved", body: "Play it with asciinema (asciinema play FILE) or on asciinema.org." });
    } catch (e) {
      toast({ kind: "error", title: "Couldn't save it", body: asApiError(e).message });
    }
  }

  return (
    <div className="view">
      <div className="toolbar view-toolbar">
        <label className="check" style={{ alignItems: "center" }}>
          <input type="checkbox" checked={record} onChange={(e) => setSettings({ recordTerminals: e.target.checked })} />
          <span>Record new terminals</span>
        </label>
        <span className="faint" style={{ fontSize: 12 }}>Output only, never keystrokes; saved on this computer.</span>
        <span className="spacer" />
        <button className="icon-btn" aria-label="Refresh" onClick={() => void load()}><RefreshCw size={14} /></button>
      </div>
      <div className="split">
        <div className="scroll">
          {list && list.length === 0 && (
            <div className="empty"><div>
              <Video size={28} className="faint" />
              <h3>No recordings yet</h3>
              <p>Turn on "Record new terminals" and open a terminal: everything it shows is saved, so you can replay what was done on a server, step by step. Each recording is also in the activity log.</p>
            </div></div>
          )}
          {list && list.length > 0 && (
            <table className="table">
              <thead><tr><th>Terminal</th><th>Started</th><th className="num">Length</th><th className="num">Size</th><th /></tr></thead>
              <tbody>
                {list.map((r) => (
                  <tr key={r.name} className={`click ${sel === r.name ? "sel" : ""}`} onClick={() => setSel(r.name)}>
                    <td className="mono"><div className="cell-clip" style={{ maxWidth: 340 }} title={r.title}>{r.title}</div></td>
                    <td className="muted nowrap">{new Date(r.started * 1000).toLocaleString()}</td>
                    <td className="num">{clock(r.seconds)}</td>
                    <td className="num faint nowrap">{bytes(r.bytes)}</td>
                    <td className="nowrap" style={{ textAlign: "right" }}>
                      <button className="icon-btn" aria-label="Save as .cast" onClick={(e) => { e.stopPropagation(); void save(r); }}><Download size={14} /></button>
                      <button className="icon-btn" aria-label="Delete recording" onClick={(e) => { e.stopPropagation(); void remove(r); }}><Trash2 size={14} /></button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </div>
        <aside className="drawer recordings-drawer">
          {cur && cast ? (
            <>
              <div className="drawer-head"><h3 className="mono" style={{ wordBreak: "break-all" }}>{cur.title}</h3><div className="muted">{new Date(cur.started * 1000).toLocaleString()} · pauses over {IDLE_LIMIT} s are shortened</div></div>
              <Player key={cur.name} cast={cast} />
            </>
          ) : <div className="empty"><p>{sel ? "Loading…" : "Pick a recording to play it."}</p></div>}
        </aside>
      </div>
    </div>
  );
}
