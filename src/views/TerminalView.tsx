import { FitAddon } from "@xterm/addon-fit";
import { WebLinksAddon } from "@xterm/addon-web-links";
import { Terminal } from "@xterm/xterm";
import { Plus, SquareTerminal, X } from "lucide-react";
import { useEffect, useRef } from "react";
import { api, asApiError } from "../lib/api";
import { SessionState, TermTab, useStore } from "../store";

export function TerminalView({ s }: { s: SessionState }) {
  const { openTerminal, closeTerminal, patch } = useStore();
  const sid = s.info.id;
  if (s.terminals.length === 0) {
    return (
      <div className="empty">
        <div>
          <SquareTerminal size={28} />
          <h3>Open a shell on {s.profile.name}</h3>
          <p>Runs over the connection you already have — no extra app or login. Use “Open terminal here” in Files to start in a specific folder.</p>
          <button className="btn primary" onClick={() => openTerminal(sid, null)}><Plus size={14} /> New terminal</button>
        </div>
      </div>
    );
  }
  return (
    <div className="term-wrap">
      <div className="term-tabs" role="tablist">
        {s.terminals.map((t) => (
          <div key={t.key} role="tab" aria-selected={t.key === s.activeTerminal} className={`term-tab ${t.key === s.activeTerminal ? "on" : ""}`} onClick={() => patch(sid, { activeTerminal: t.key })} title={t.cwd ?? "home"}>
            <SquareTerminal size={13} /> {t.title}
            <button className="icon-btn" style={{ width: 20, height: 20 }} aria-label="Close terminal" onClick={(e) => { e.stopPropagation(); closeTerminal(sid, t.key); }}><X size={12} /></button>
          </div>
        ))}
        <button className="icon-btn" aria-label="New terminal" title="New terminal" onClick={() => openTerminal(sid, null)}><Plus size={14} /></button>
      </div>
      <div className="term-host">
        {s.terminals.map((t) => (
          <TerminalPane key={t.key} sid={sid} tab={t} visible={s.tab === "terminal" && t.key === s.activeTerminal} />
        ))}
      </div>
    </div>
  );
}

function decode(b64: string): Uint8Array {
  const bin = atob(b64);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
}

function TerminalPane({ sid, tab, visible }: { sid: string; tab: TermTab; visible: boolean }) {
  const host = useRef<HTMLDivElement>(null);
  const fitRef = useRef<FitAddon>();
  const termRef = useRef<Terminal>();
  const tidRef = useRef<string>();
  const toast = useStore((x) => x.toast);

  useEffect(() => {
    const term = new Terminal({
      fontFamily: "JetBrains Mono, ui-monospace, Menlo, monospace",
      fontSize: 13,
      lineHeight: 1.2,
      cursorBlink: true,
      allowProposedApi: true,
      macOptionIsMeta: true,
      scrollback: 10000,
      theme: {
        background: "#141a25", foreground: "#e6eaf2", cursor: "#f2b544", cursorAccent: "#141a25",
        selectionBackground: "#f2b54440", black: "#1a2130", brightBlack: "#4a5670",
        red: "#e5645a", brightRed: "#ff8a80", green: "#4fbf8b", brightGreen: "#7fd1b9",
        yellow: "#f2b544", brightYellow: "#f6c362", blue: "#8fb4ff", brightBlue: "#b3ccff",
        magenta: "#c79bff", brightMagenta: "#dcc2ff", cyan: "#7fd1b9", brightCyan: "#a8e6d4",
        white: "#a9b3c7", brightWhite: "#ffffff",
      },
    });
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.loadAddon(new WebLinksAddon());
    term.open(host.current!);
    termRef.current = term;
    fitRef.current = fit;
    try { fit.fit(); } catch { /* hidden */ }

    let disposed = false;
    api
      .openTerminal(sid, term.cols || 100, term.rows || 30, tab.cwd, (m) => {
        if (m.type === "data") term.write(decode(m.data));
        else term.write(`\r\n\x1b[2m[session ended${m.code != null ? ` with code ${m.code}` : ""}]\x1b[0m\r\n`);
      })
      .then((tid) => {
        if (disposed) return api.terminalClose(tid);
        tidRef.current = tid;
        term.focus();
      })
      .catch((e) => {
        term.write(`\x1b[31mCouldn't open a shell: ${asApiError(e).message}\x1b[0m\r\n`);
        toast({ kind: "error", title: "Terminal failed", body: asApiError(e).message });
      });

    const input = term.onData((d) => tidRef.current && api.terminalWrite(tidRef.current, d));
    const resize = term.onResize(({ cols, rows }) => tidRef.current && api.terminalResize(tidRef.current, cols, rows));
    const ro = new ResizeObserver(() => {
      if (!disposed && host.current && host.current.offsetWidth > 0) try { fit.fit(); } catch { /* ignore */ }
    });
    ro.observe(host.current!);

    return () => {
      disposed = true;
      ro.disconnect();
      input.dispose();
      resize.dispose();
      if (tidRef.current) api.terminalClose(tidRef.current);
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

  return <div className="term-pane" ref={host} style={{ visibility: visible ? "visible" : "hidden" }} />;
}
