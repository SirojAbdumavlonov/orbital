import { FitAddon } from "@xterm/addon-fit";
import { WebglAddon } from "@xterm/addon-webgl";
import { WebLinksAddon } from "@xterm/addon-web-links";
import { Terminal } from "@xterm/xterm";
import { Columns2, CornerDownLeft, Layers, Plus, Radio, Rows2, ShieldAlert, Sparkles, Square, SquareTerminal, User, X } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { api, asApiError, TmuxSession } from "../lib/api";
import { relativeTime } from "../lib/format";
import { useAccentColor, useResolvedTheme, useSettings } from "../lib/settings";
import { SessionState, TermTab, useStore } from "../store";
import { LineTracker, suggest } from "../lib/suggest";
import { broadcastFrom, registerTerminal, unregisterTerminal, useBroadcast, useLiveTerminals } from "../lib/broadcast";

export function TerminalView({ s }: { s: SessionState }) {
  const { openTerminal, closeTerminal, patch, renameTerminal } = useStore();
  const sid = s.info.id;
  const [renaming, setRenaming] = useState<string | null>(null);
  const asRoot = useSettings((x) => x.terminalAsRoot);
  const canRoot = s.info.elevation;
  const user = s.profile.username;
  // The second terminal on screen, while split (it must exist and differ from the active one).
  const split = s.termSplit && s.termSplit.key !== s.activeTerminal && s.terminals.some((t) => t.key === s.termSplit!.key) ? s.termSplit : null;
  const splitWith = (dir: "row" | "column") => {
    const other = s.terminals.find((t) => t.key !== s.activeTerminal);
    if (other) return patch(sid, { termSplit: { key: other.key, dir } });
    // Only one terminal: open another beside it.
    const before = s.activeTerminal;
    openTerminal(sid, null);
    const now = useStore.getState().sessions[sid];
    const added = now?.terminals[now.terminals.length - 1];
    if (added && before) patch(sid, { activeTerminal: before, termSplit: { key: added.key, dir } });
  };
  if (s.terminals.length === 0) {
    return (
      <div className="empty">
        <div>
          <SquareTerminal size={28} />
          <h3>Open a shell on {s.profile.name}</h3>
          <p>Runs over the connection you already have — no extra app or login. Use “Open terminal here” in Files to start in a specific folder.</p>
          <div style={{ display: "flex", gap: 8, justifyContent: "center" }}>
            {canRoot ? (
              <>
                <button className="btn primary" onClick={() => openTerminal(sid, null, asRoot)}><Plus size={14} /> New terminal as {asRoot ? "root" : user}</button>
                <button className="btn" onClick={() => openTerminal(sid, null, !asRoot)}>{asRoot ? <User size={14} /> : <ShieldAlert size={14} />} As {asRoot ? user : "root"}</button>
              </>
            ) : (
              <button className="btn primary" onClick={() => openTerminal(sid, null)}><Plus size={14} /> New terminal</button>
            )}
          </div>
          <div style={{ display: "flex", justifyContent: "center", marginTop: 10 }}><TmuxMenu s={s} label /></div>
          {canRoot && <p className="faint" style={{ marginTop: 12, fontSize: 12 }}>This server's settings allow sudo, so terminals can open as root. Change the default in Settings.</p>}
        </div>
      </div>
    );
  }
  return (
    <div className="term-wrap">
      <div className="term-tabs" role="tablist">
        {s.terminals.map((t) => (
          <div key={t.key} role="tab" aria-selected={t.key === s.activeTerminal} className={`term-tab ${t.key === s.activeTerminal ? "on" : split && t.key === split.key ? "on second" : ""}`} onClick={() => patch(sid, split && t.key === split.key && s.activeTerminal ? { activeTerminal: t.key, termSplit: { key: s.activeTerminal, dir: split.dir } } : { activeTerminal: t.key })} onDoubleClick={() => setRenaming(t.key)} title={`${t.cwd ?? "home"} · double-click to rename`}>
            {t.root ? <span className="root-mark" title="Runs as root">#</span> : <SquareTerminal size={13} />}
            {renaming === t.key ? (
              <input
                className="input"
                autoFocus
                defaultValue={t.title}
                aria-label="Terminal name"
                onFocus={(e) => e.target.select()}
                onClick={(e) => e.stopPropagation()}
                onBlur={(e) => { renameTerminal(sid, t.key, e.target.value); setRenaming(null); }}
                onKeyDown={(e) => {
                  if (e.key === "Enter") e.currentTarget.blur();
                  if (e.key === "Escape") { e.currentTarget.value = t.title; e.currentTarget.blur(); }
                }}
              />
            ) : t.title}
            <button className="icon-btn" style={{ width: 20, height: 20 }} aria-label="Close terminal" onClick={(e) => { e.stopPropagation(); closeTerminal(sid, t.key); }}><X size={12} /></button>
          </div>
        ))}
        <button className="icon-btn" aria-label="New terminal" title={canRoot ? `New terminal as ${asRoot ? "root" : user}` : "New terminal"} onClick={() => openTerminal(sid, null)}><Plus size={14} /></button>
        {canRoot && (
          <button className="icon-btn" aria-label={`New terminal as ${asRoot ? user : "root"}`} title={`New terminal as ${asRoot ? user : "root"}`} onClick={() => openTerminal(sid, null, !asRoot)}>
            {asRoot ? <User size={14} /> : <ShieldAlert size={14} />}
          </button>
        )}
        <TmuxMenu s={s} />
        <span className="term-tabs-end">
          {split ? (
            <button className="icon-btn" aria-label="Show one terminal" title="Show one terminal" onClick={() => patch(sid, { termSplit: null })}><Square size={14} /></button>
          ) : (
            <>
              <button className="icon-btn" aria-label="Split side by side" title="Show two terminals side by side" onClick={() => splitWith("row")}><Columns2 size={14} /></button>
              <button className="icon-btn" aria-label="Split one above the other" title="Show two terminals, one above the other" onClick={() => splitWith("column")}><Rows2 size={14} /></button>
            </>
          )}
          <BroadcastMenu />
        </span>
      </div>
      <div className="term-host">
        {s.terminals.map((t, i) => {
          const second = split && t.key === split.key;
          const same = s.terminals.filter((x) => x.title === t.title).length > 1;
          const first = t.key === s.activeTerminal;
          return (
            <TerminalPane
              key={t.key}
              sid={sid}
              tab={t}
              label={`${s.profile.name} · ${t.title}${same ? ` ${i + 1}` : ""}`}
              visible={s.tab === "terminal" && (first || !!second)}
              area={!split ? undefined : first ? (split.dir === "row" ? "left" : "top") : second ? (split.dir === "row" ? "right" : "bottom") : undefined}
            />
          );
        })}
      </div>
    </div>
  );
}

/**
 * tmux: sessions on the server (to reattach after a disconnect, or from
 * another computer) and a new named one. A tmux session keeps running when
 * Orbital disconnects.
 */
function TmuxMenu({ s, label }: { s: SessionState; label?: boolean }) {
  const openTerminal = useStore((x) => x.openTerminal);
  const asRoot = useSettings((x) => x.terminalAsRoot) && s.info.elevation;
  const [open, setOpen] = useState(false);
  const [list, setList] = useState<TmuxSession[] | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [name, setName] = useState("");
  const box = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    setList(null);
    setErr(null);
    api.tmuxSessions(s.info.id, asRoot).then(setList, (e) => setErr(asApiError(e).message));
    const away = (e: MouseEvent) => !box.current?.contains(e.target as Node) && setOpen(false);
    document.addEventListener("mousedown", away);
    return () => document.removeEventListener("mousedown", away);
  }, [open]);
  const valid = /^[A-Za-z0-9_.][A-Za-z0-9_.-]{0,39}$/.test(name);
  const go = (session: string) => {
    openTerminal(s.info.id, null, asRoot, { kind: "tmux", session });
    setOpen(false);
    setName("");
  };
  return (
    <div className="tmux-menu" ref={box}>
      <button className={label ? "btn" : "icon-btn"} aria-label="tmux sessions" title="tmux: sessions that keep running when you disconnect" onClick={() => setOpen(!open)}>
        <Layers size={14} />{label && " tmux sessions"}
      </button>
      {open && (
        <div className="tmux-pop" role="dialog" aria-label="tmux sessions">
          <b>tmux sessions{asRoot ? " (root)" : ""}</b>
          <p className="faint">Keep running when Orbital disconnects; open one again to pick up where you left off.</p>
          {err && <div className="alert">{err}</div>}
          {!list && !err && <div className="faint">Looking…</div>}
          {list?.length === 0 && <div className="faint">None running on this server.</div>}
          {list?.map((t) => (
            <button key={t.name} className="tmux-row" onClick={() => go(t.name)}>
              <span className="mono">{t.name}</span>
              <span className="faint">{t.windows} {t.windows === 1 ? "window" : "windows"}{t.created ? ` · ${relativeTime(t.created)}` : ""}{t.attached ? " · open elsewhere" : ""}</span>
            </button>
          ))}
          <form className="tmux-new" onSubmit={(e) => { e.preventDefault(); if (valid) go(name); }}>
            <input className="input mono" placeholder="new session name" value={name} onChange={(e) => setName(e.target.value)} />
            <button className="btn primary" disabled={!valid}>Open</button>
          </form>
        </div>
      )}
    </div>
  );
}

const TERM_THEMES = {
  dark: {
    background: "#141a25", foreground: "#e6eaf2", cursor: "#f2b544", cursorAccent: "#141a25",
    selectionBackground: "#f2b54440", black: "#1a2130", brightBlack: "#4a5670",
    red: "#e5645a", brightRed: "#ff8a80", green: "#4fbf8b", brightGreen: "#7fd1b9",
    yellow: "#f2b544", brightYellow: "#f6c362", blue: "#8fb4ff", brightBlue: "#b3ccff",
    magenta: "#c79bff", brightMagenta: "#dcc2ff", cyan: "#7fd1b9", brightCyan: "#a8e6d4",
    white: "#a9b3c7", brightWhite: "#ffffff",
  },
  light: {
    background: "#fbfcfd", foreground: "#1a2130", cursor: "#c98a12", cursorAccent: "#fbfcfd",
    selectionBackground: "#e3a32c40", black: "#1a2130", brightBlack: "#6b7689",
    red: "#c0392b", brightRed: "#d9534f", green: "#1e8a57", brightGreen: "#2fa36b",
    yellow: "#a8700a", brightYellow: "#c98a12", blue: "#2f5fb3", brightBlue: "#4a7bd0",
    magenta: "#8a4fc7", brightMagenta: "#a46ad9", cyan: "#1b8a7a", brightCyan: "#2aa391",
    white: "#c3cbd8", brightWhite: "#e8ecf2",
  },
};

/** The WebGL renderer, when this computer's webview has it; `refresh` redraws its letters. */
function attachGpuRenderer(term: Terminal): { refresh(): void; dispose(): void } {
  let addon: WebglAddon | null = null;
  try {
    addon = new WebglAddon();
    addon.onContextLoss(() => { addon?.dispose(); addon = null; });
    term.loadAddon(addon);
  } catch {
    addon = null; // no WebGL here: the DOM renderer stays
  }
  return {
    refresh() { try { addon?.clearTextureAtlas(); term.refresh(0, term.rows - 1); } catch { /* gone */ } },
    dispose() { try { addon?.dispose(); } catch { /* gone */ } },
  };
}

function decode(b64: string): Uint8Array {
  const bin = atob(b64);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
}

/** The server's shell history, read once per server and user (kept in memory only). */
const histories = new Map<string, Promise<string[]>>();
function historyFor(sid: string, root: boolean): Promise<string[]> {
  const key = `${sid}:${root}`;
  let h = histories.get(key);
  if (!h) {
    h = api.shellHistory(sid, root).catch(() => []);
    histories.set(key, h);
  }
  return h;
}

/** The text on the cursor's line before the cursor (with wrapped rows), and after it. */
function cursorText(term: Terminal): { before: string; after: string } {
  const b = term.buffer.active;
  let y = b.baseY + b.cursorY;
  let line = b.getLine(y);
  let before = (line?.translateToString(false) ?? "").slice(0, b.cursorX);
  const after = (line?.translateToString(true) ?? "").slice(b.cursorX);
  while (line?.isWrapped && y > 0) {
    line = b.getLine(--y);
    before = (line?.translateToString(false) ?? "") + before;
  }
  return { before, after };
}

/** The line ends like a shell prompt ($, #, %, >, ❯), so a command can be typed. */
const atPrompt = (before: string) => /[$#%>❯]\s*$/.test(before);

function TerminalPane({ sid, tab, visible, label, area }: { sid: string; tab: TermTab; visible: boolean; label: string; area?: "left" | "right" | "top" | "bottom" }) {
  const paneId = `${sid}:${tab.key}`;
  const broadcasting = useBroadcast((b) => b.on && b.targets.includes(paneId));
  const others = useBroadcast((b) => b.targets.length - 1);
  const host = useRef<HTMLDivElement>(null);
  const fitRef = useRef<FitAddon>();
  const termRef = useRef<Terminal>();
  const tidRef = useRef<string>();
  const [recording, setRecording] = useState(false);
  const toast = useStore((x) => x.toast);
  const theme = useResolvedTheme();
  const fontSize = useSettings((x) => x.terminalFontSize);
  const accent = useAccentColor();
  // The cursor and selection follow the accent colour.
  const termTheme = () => ({ ...TERM_THEMES[theme], cursor: accent, selectionBackground: `${accent}40` });
  const suggestOn = useSettings((x) => x.terminalSuggestions);
  const suggestOnRef = useRef(suggestOn);
  suggestOnRef.current = suggestOn;
  const [sugg, setSugg] = useState<string[]>([]);
  const suggRef = useRef<string[]>([]);
  const tracker = useRef(new LineTracker());
  const typed = useRef<string[]>([]);
  const history = useRef<string[]>([]);

  /** Work out what to suggest from what's typed and what's on screen. */
  const refresh = useRef(() => {});
  refresh.current = () => {
    const term = termRef.current;
    const t = tracker.current;
    let next: string[] = [];
    if (term && suggestOnRef.current && term.buffer.active.type === "normal" && t.known) {
      const { before, after } = cursorText(term);
      // Only at the end of a line the shell echoed: never while a password is
      // typed (no echo), in a full-screen program, or after edits we can't see.
      if (!after.trim() && before.endsWith(t.line) && (t.line || atPrompt(before))) {
        next = suggest(t.line, { session: typed.current, history: history.current });
      }
    }
    if (next.join("\n") !== suggRef.current.join("\n")) {
      suggRef.current = next;
      setSugg(next);
    }
  };

  /** Type the rest of a suggestion at the prompt. Enter stays the user's. */
  const accept = (cmd: string) => {
    const tid = tidRef.current;
    const t = tracker.current;
    if (!tid || !cmd.startsWith(t.line)) return;
    const rest = cmd.slice(t.line.length);
    if (rest) {
      api.terminalWrite(tid, rest);
      t.feed(rest);
    }
    termRef.current?.focus();
  };

  // Theme and font size follow the settings without restarting the shell.
  useEffect(() => {
    const term = termRef.current;
    if (!term) return;
    term.options.theme = termTheme();
    term.options.fontSize = fontSize;
    try { fitRef.current?.fit(); } catch { /* hidden */ }
  }, [theme, fontSize, accent]);

  useEffect(() => {
    let disposed = false;
    const term = new Terminal({
      fontFamily: "JetBrains Mono, ui-monospace, Menlo, monospace",
      fontSize,
      lineHeight: 1.2,
      cursorBlink: true,
      allowProposedApi: true,
      macOptionIsMeta: true,
      scrollback: 10000,
      theme: termTheme(),
    });
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.loadAddon(new WebLinksAddon());
    term.open(host.current!);
    termRef.current = term;
    fitRef.current = fit;
    // Draw with the GPU (WebGL), like Warp or iTerm: typing and fast output stay
    // smooth at the display's frame rate. Without WebGL, or if the GPU context is
    // lost, xterm falls back to its DOM renderer.
    const gpu = attachGpuRenderer(term);
    try { fit.fit(); } catch { /* hidden */ }
    // The atlas of drawn letters must use the real font: redraw once it has loaded.
    void document.fonts?.ready.then(() => { if (!disposed) { gpu.refresh(); try { fit.fit(); } catch { /* hidden */ } } });

    const recordThis = useSettings.getState().recordTerminals;
    api
      .openTerminal(sid, term.cols || 100, term.rows || 30, tab.cwd, tab.root, (m) => {
        if (m.type === "data") term.write(decode(m.data));
        else term.write(`\r\n\x1b[2m[session ended${m.code != null ? ` with code ${m.code}` : ""}]\x1b[0m\r\n`);
      }, tab.launch, recordThis)
      .then((tid) => {
        if (disposed) return api.terminalClose(tid);
        tidRef.current = tid;
        setRecording(recordThis);
        registerTerminal({ id: paneId, tid, label });
        term.focus();
      })
      .catch((e) => {
        term.write(`\x1b[31mCouldn't open a shell: ${asApiError(e).message}\x1b[0m\r\n`);
        toast({ kind: "error", title: "Terminal failed", body: asApiError(e).message });
      });

    void historyFor(sid, tab.root).then((h) => {
      history.current = h;
    });
    let frame = 0;
    const later = () => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => refresh.current());
    };
    const input = term.onData((d) => {
      if (!tidRef.current) return;
      // A command counts as typed only if the shell showed it (a password isn't echoed).
      const t = tracker.current;
      const shown = !!t.line && cursorText(term).before.endsWith(t.line);
      const done = t.feed(d);
      if (done && shown) typed.current = [...typed.current.filter((c) => c !== done), done].slice(-200);
      api.terminalWrite(tidRef.current, d);
      broadcastFrom(paneId, d);
      later();
    });
    const parsed = term.onWriteParsed(later);
    // → at the end of the line takes the first suggestion, as in fish.
    term.attachCustomKeyEventHandler((e) => {
      if (e.type === "keydown" && e.key === "ArrowRight" && !e.shiftKey && !e.altKey && !e.metaKey && !e.ctrlKey && suggRef.current.length > 0) {
        accept(suggRef.current[0]);
        return false;
      }
      return true;
    });
    const resize = term.onResize(({ cols, rows }) => tidRef.current && api.terminalResize(tidRef.current, cols, rows));
    const ro = new ResizeObserver(() => {
      if (!disposed && host.current && host.current.offsetWidth > 0) try { fit.fit(); } catch { /* ignore */ }
    });
    ro.observe(host.current!);

    return () => {
      disposed = true;
      ro.disconnect();
      input.dispose();
      parsed.dispose();
      cancelAnimationFrame(frame);
      resize.dispose();
      unregisterTerminal(paneId);
      if (tidRef.current) api.terminalClose(tidRef.current);
      gpu.dispose();
      term.dispose();
    };
  }, []);

  useEffect(() => {
    if (visible) {
      requestAnimationFrame(() => {
        try { fitRef.current?.fit(); } catch { /* ignore */ }
        termRef.current?.focus();
      });
    }
  }, [visible]);

  useEffect(() => refresh.current(), [suggestOn]);

  return (
    <div className={`term-pane ${area ? `area-${area}` : ""} ${broadcasting ? "broadcasting" : ""}`} style={{ visibility: visible ? "visible" : "hidden" }}>
      {(area || broadcasting) && (
        <div className="term-pane-head">
          <span className="mono">{label}</span>
          {broadcasting && <span className="term-broadcast-tag"><Radio size={11} /> typing also goes to {others} other terminal{others === 1 ? "" : "s"}</span>}
        </div>
      )}
      {recording && <button className="term-rec" title="This terminal is being recorded (output only). Recordings are under Activity log → Terminal recordings." onClick={() => useStore.getState().setGlobalView("recordings")}>● REC</button>}
      <div className="term-xterm" ref={host} />
      {suggestOn && (
        <div className="term-suggest" aria-label="Suggested commands">
          {sugg.length > 0 ? (
            <>
              <Sparkles size={12} className="faint" />
              {sugg.map((c, i) => (
                <button key={c} className={`term-chip mono ${i === 0 ? "first" : ""}`} title={i === 0 ? "→ or click to type it (Enter runs it)" : "Click to type it (Enter runs it)"} onMouseDown={(e) => e.preventDefault()} onClick={() => accept(c)}>
                  {c}
                </button>
              ))}
              <span className="faint term-suggest-hint"><kbd>→</kbd> types the first · <CornerDownLeft size={11} /> runs it</span>
            </>
          ) : (
            <span className="faint">Suggestions from this server's history and common next steps appear here as you type.</span>
          )}
        </div>
      )}
    </div>
  );
}

/** "Type in several": choose terminals (any server) that get the same keys; clearly marked while on. */
function BroadcastMenu() {
  const [open, setOpen] = useState(false);
  const live = useLiveTerminals((x) => x.list);
  const b = useBroadcast();
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const away = (e: MouseEvent) => !ref.current?.contains(e.target as Node) && setOpen(false);
    window.addEventListener("mousedown", away);
    return () => window.removeEventListener("mousedown", away);
  }, [open]);
  return (
    <div className="menu-anchor" ref={ref}>
      <button className={`icon-btn ${b.on ? "on broadcast-on" : ""}`} aria-label="Type in several terminals" title={b.on ? `Typing goes to ${b.targets.length} terminals` : "Type in several terminals at once"} onClick={() => setOpen(!open)}>
        <Radio size={14} />{b.on && <span className="count">{b.targets.length}</span>}
      </button>
      {open && (
        <div className="menu broadcast-menu" role="dialog" aria-label="Type in several terminals">
          <div className="broadcast-head">
            <b>Type in several terminals</b>
            <label className="check"><input type="checkbox" className="switch" checked={b.on} disabled={b.targets.length < 2} onChange={(e) => b.setOn(e.target.checked)} /> On</label>
          </div>
          <p className="faint">What you type in one checked terminal also goes to the others. Check two or more; each shows a stripe while it's on.</p>
          {live.length === 0 && <p className="faint">No terminals open.</p>}
          {live.map((t) => (
            <label key={t.id} className="check broadcast-item">
              <input type="checkbox" checked={b.targets.includes(t.id)} onChange={() => b.toggle(t.id)} /> <span className="mono">{t.label}</span>
            </label>
          ))}
        </div>
      )}
    </div>
  );
}
