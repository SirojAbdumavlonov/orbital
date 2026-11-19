import { Bell, BellOff, Bot, CheckCircle2, Copy, KeyRound, Play, RefreshCw, Send, ShieldCheck, Square, Trash2 } from "lucide-react";
import { useEffect, useState } from "react";
import { JobProgress } from "../components/JobProgress";
import { confirm, StateDot } from "../components/ui";
import { AgentSetup, AgentStatus, AiProvider, api, asApiError, PairCode } from "../lib/api";
import { relativeTime } from "../lib/format";
import { runJob, useRunningJob } from "../lib/jobs";
import { useRemote } from "../lib/remote";
import { SessionState, useStore } from "../store";

const copy = (t: string) => navigator.clipboard?.writeText(t).catch(() => {});

/** Friendly lines from the agent's activity log (JSON lines, no secrets). */
function activity(line: string): { time: string; text: string } | null {
  try {
    const r = JSON.parse(line);
    const t = String(r.time ?? "").replace("T", " ").slice(5, 16);
    const text: Record<string, string> = {
      start: `Started (version ${r.version})`,
      config: `Settings loaded (${r.provider ?? ""} ${r.model ?? ""})`.trim(),
      paired: `Paired with ${r.name}`,
      pair_failed: "Someone sent a wrong pairing code",
      question: "You asked something",
      tool: `Looked: ${String(r.tool ?? "").replace(/_/g, " ")}`,
      approval_asked: `Asked to approve: ${r.action}`,
      approved: `Approved: ${r.action}`,
      action_done: `Done: ${r.action}`,
      rejected: `Rejected: ${r.action}`,
      expired: `Not approved in time: ${r.action}`,
      alert: `Alert sent: ${String(r.key ?? "").replace(":", " ")}`,
      alert_cleared: `All clear sent: ${String(r.key ?? "").replace(":", " ")}`,
      error: `Problem: ${r.error}`,
    };
    return { time: t, text: text[r.event] ?? r.event };
  } catch {
    return null;
  }
}

/**
 * The AI agent: a Telegram bot on this server, backed by Claude or ChatGPT,
 * that answers questions about it 24/7 and asks before changing anything.
 */
export function AgentView({ s }: { s: SessionState }) {
  const sid = s.info.id;
  const { data: st, error, loading, reload } = useRemote(() => api.agentStatus(sid), [sid]);
  const job = useRunningJob(`agent:${sid}`);
  const [editing, setEditing] = useState(false);
  const [code, setCode] = useState<PairCode | null>(null);

  // While a pairing code waits, look for the pairing every few seconds.
  useEffect(() => {
    if (!st?.installed || st.owner || !(code || st.pairCodeActive)) return;
    const t = setInterval(reload, 4000);
    return () => clearInterval(t);
  }, [st, code]);
  useEffect(() => {
    if (st?.owner) setCode(null);
  }, [st?.owner]);

  if (error) return <div className="empty"><div><h3>Couldn't read the agent's state</h3><p>{error.message}</p><button className="btn" onClick={reload}>Try again</button></div></div>;
  if (!st) return <div className="empty"><div><RefreshCw size={22} className="spin faint" /></div></div>;

  return (
    <div className="view">
      <div className="toolbar">
        <Bot size={15} className="faint" />
        <b>AI agent in Telegram</b>
        {st.installed && <span className="faint"><StateDot state={st.running ? "running" : "inactive"} label={st.running ? "running" : "stopped"} />{st.version ? ` · v${st.version}` : ""}</span>}
        <span className="spacer" />
        <button className="icon-btn" aria-label="Refresh" onClick={reload}><RefreshCw size={14} className={loading ? "spin" : ""} /></button>
      </div>
      {job && <div className="job-bar"><JobProgress job={job} /></div>}
      <div className="scroll agent">
        {!st.installed || editing ? (
          <Setup s={s} status={st} onDone={(c) => { setCode(c); setEditing(false); reload(); }} onCancel={st.installed ? () => setEditing(false) : undefined} />
        ) : (
          <Installed s={s} st={st} code={code} setCode={setCode} reload={reload} onEdit={() => setEditing(true)} />
        )}
      </div>
    </div>
  );
}

function Setup({ s, status, onDone, onCancel }: { s: SessionState; status: AgentStatus; onDone: (c: PairCode) => void; onCancel?: () => void }) {
  const sid = s.info.id;
  const [token, setToken] = useState("");
  const [provider, setProvider] = useState<AiProvider>((status.provider as AiProvider) || "claude");
  const [key, setKey] = useState("");
  const [model, setModel] = useState("");
  const [alerts, setAlerts] = useState(status.installed ? status.alerts : true);
  const [checked, setChecked] = useState<string | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const job = useRunningJob(`agent:${sid}`);
  const setup: AgentSetup = { telegramToken: token.trim(), provider, apiKey: key.trim(), model: model.trim() || null, alerts };
  const ready = token.trim().length > 30 && key.trim().length > 20;

  async function check() {
    setBusy(true);
    setErr(null);
    try {
      setChecked(await api.agentCheck(setup));
    } catch (e) {
      setChecked(null);
      setErr(asApiError(e).message);
    } finally {
      setBusy(false);
    }
  }

  async function install() {
    setErr(null);
    try {
      const c = await runJob((p) => api.agentInstall(sid, setup, p), { key: `agent:${sid}`, title: status.installed ? "Updating the AI agent's settings" : "Installing the AI agent", sid, done: (r) => `AI agent ready: send /start ${r.code} to @${r.bot}` });
      // The keys are on the server now; forget them here.
      setToken("");
      setKey("");
      onDone(c);
    } catch (e) {
      setErr(asApiError(e).message);
    }
  }

  return (
    <div className="agent-setup">
      {!status.installed && (
        <div className="agent-intro">
          <p>Ask about <b>{s.profile.name}</b> from Telegram, any time: "is everything OK?", "why is the site slow?", "show nginx errors from the last hour". The agent runs on the server, so it answers even when this computer is off, and messages you when a service fails, a disk fills up or a container keeps restarting.</p>
          <ul className="agent-safety">
            <li><ShieldCheck size={13} /> It only answers your Telegram account (paired with a one-time code).</li>
            <li><ShieldCheck size={13} /> It looks freely; restarting, stopping or running anything waits for your <b>Approve</b> in the chat.</li>
            <li><ShieldCheck size={13} /> It never reads keys, passwords or its own config. Everything it does is in its activity log.</li>
            <li><KeyRound size={13} /> The bot token and the AI key are stored on the server only, in a file only root can read.</li>
          </ul>
        </div>
      )}
      {err && <div className="alert">{err}</div>}
      <div className="form">
        <div className="agent-step"><span className="wizard-num">1</span> <b>Your Telegram bot</b></div>
        <p className="hint" style={{ marginTop: -4 }}>In Telegram, open <span className="mono">@BotFather</span>, send <span className="mono">/newbot</span>, pick a name, and copy the token it gives you.</p>
        <label className="field"><span>Bot token</span><input className="input mono" type="password" autoComplete="off" placeholder="123456789:AAH…" value={token} onChange={(e) => { setToken(e.target.value); setChecked(null); }} /></label>

        <div className="agent-step"><span className="wizard-num">2</span> <b>The AI</b></div>
        <div className="segmented" role="tablist">
          <button className={provider === "claude" ? "on" : ""} onClick={() => { setProvider("claude"); setChecked(null); }}>Claude</button>
          <button className={provider === "openai" ? "on" : ""} onClick={() => { setProvider("openai"); setChecked(null); }}>ChatGPT</button>
        </div>
        <label className="field">
          <span>{provider === "claude" ? "Claude API key" : "OpenAI API key"}</span>
          <input className="input mono" type="password" autoComplete="off" placeholder={provider === "claude" ? "sk-ant-…" : "sk-…"} value={key} onChange={(e) => { setKey(e.target.value); setChecked(null); }} />
          <small className="hint">{provider === "claude" ? "From console.anthropic.com → API keys. Billed by Anthropic per message." : "From platform.openai.com → API keys. Billed by OpenAI per message."}</small>
        </label>
        <label className="field">
          <span>Model <span className="faint">(optional)</span></span>
          <input className="input mono" placeholder={provider === "claude" ? "claude-opus-5-5" : "gpt-5"} value={model} onChange={(e) => setModel(e.target.value)} />
        </label>
        <label className="check"><input type="checkbox" checked={alerts} onChange={(e) => setAlerts(e.target.checked)} /><span>Message me when a service fails, a disk is over 90% full or a container keeps restarting (checked every 5 minutes)</span></label>

        <div className="agent-actions">
          {checked && <span className="badge ok"><CheckCircle2 size={12} /> @{checked} and the key work</span>}
          <span className="spacer" />
          {onCancel && <button className="btn" onClick={onCancel}>Cancel</button>}
          <button className="btn" disabled={!ready || busy || !!job} onClick={check}>{busy ? "Checking…" : "Check"}</button>
          <button className="btn primary" disabled={!ready || !!job} onClick={install}><Send size={13} /> {status.installed ? "Save on the server" : `Install on ${s.profile.name}`}</button>
        </div>
        <p className="footnote">Installing adds Python's AI library ({provider === "claude" ? "anthropic" : "openai"}) in its own folder, /opt/orbital-agent, and a service, orbital-agent. Remove it here any time; nothing else on the server changes.</p>
      </div>
    </div>
  );
}

function Installed({ s, st, code, setCode, reload, onEdit }: { s: SessionState; st: AgentStatus; code: PairCode | null; setCode: (c: PairCode | null) => void; reload: () => void; onEdit: () => void }) {
  const sid = s.info.id;
  const { toast } = useStore.getState();
  const job = useRunningJob(`agent:${sid}`);
  const [, tick] = useState(0);
  useEffect(() => {
    if (!code) return;
    const t = setInterval(() => tick((n) => n + 1), 1000);
    return () => clearInterval(t);
  }, [code]);
  const left = code ? Math.max(0, code.expires - Math.floor(Date.now() / 1000)) : 0;
  const bot = st.bot ?? code?.bot;

  const act = async (f: () => Promise<unknown>, ok: string) => {
    try {
      await f();
      toast({ kind: "success", title: ok });
    } catch (e) {
      toast({ kind: "error", title: "That didn't work", body: asApiError(e).message });
    }
    reload();
  };
  async function newCode() {
    if (st.owner && !(await confirm({ title: "Pair another Telegram account?", body: `${st.owner} stops being answered until the new code is used.`, confirmLabel: "Make a new code" }))) return;
    try {
      setCode(await api.agentPairCode(sid));
      reload();
    } catch (e) {
      toast({ kind: "error", title: "Couldn't make a code", body: asApiError(e).message });
    }
  }
  async function remove() {
    if (!(await confirm({ title: `Remove the AI agent from ${s.profile.name}?`, body: "The bot stops answering, and its config (with the bot token and the AI key), its activity log and its Python folder are deleted from the server.", confirmLabel: "Remove agent", danger: true, requireText: "remove" }))) return;
    await act(() => api.agentRemove(sid), "The AI agent was removed");
  }

  const log = st.recent.map(activity).filter(Boolean).reverse().slice(0, 25) as { time: string; text: string }[];
  return (
    <>
      <section className="panel agent-card">
        {st.owner ? (
          <div className="agent-paired">
            <CheckCircle2 size={18} className="ok-text" />
            <div>
              <b>Answering {st.owner}{bot ? <> through <span className="mono">@{bot}</span></> : ""}</b>
              <div className="faint">{st.pairedAt ? `Paired ${relativeTime(st.pairedAt)} · ` : ""}{st.provider === "openai" ? "ChatGPT" : "Claude"} ({st.model})</div>
            </div>
          </div>
        ) : code && left > 0 ? (
          <div className="agent-pair">
            <div className="faint">In Telegram, open <span className="mono">@{bot ?? "your bot"}</span> and send:</div>
            <div className="agent-code mono">/start {code.code}</div>
            <div className="agent-pair-row">
              <button className="btn" onClick={() => { copy(`/start ${code.code}`); toast({ kind: "info", title: "Copied" }); }}><Copy size={13} /> Copy</button>
              <span className="faint">valid for {Math.floor(left / 60)}:{String(left % 60).padStart(2, "0")} · waiting for you…</span>
            </div>
          </div>
        ) : (
          <div className="agent-pair">
            <b>Not paired with a Telegram account yet</b>
            <div className="faint">Make a pairing code, then send it to <span className="mono">@{bot ?? "your bot"}</span>.</div>
          </div>
        )}
      </section>

      <div className="agent-actions">
        <button className="btn" disabled={!!job} onClick={newCode}><KeyRound size={13} /> {st.owner ? "Pair another account" : "Make a pairing code"}</button>
        <button className="btn" disabled={!!job} onClick={() => act(() => api.agentSetAlerts(sid, !st.alerts), st.alerts ? "Alerts are off" : "Alerts are on")}>{st.alerts ? <><BellOff size={13} /> Turn alerts off</> : <><Bell size={13} /> Turn alerts on</>}</button>
        {st.running
          ? <button className="btn" disabled={!!job} onClick={() => act(() => api.agentSetRunning(sid, false), "The agent is stopped")}><Square size={13} /> Stop</button>
          : <button className="btn" disabled={!!job} onClick={() => act(() => api.agentSetRunning(sid, true), "The agent is running")}><Play size={13} /> Start</button>}
        {st.updateAvailable && <button className="btn primary" disabled={!!job} onClick={() => runJob((p) => api.agentUpdate(sid, p), { key: `agent:${sid}`, title: "Updating the AI agent", sid, quietHere: false, done: () => "The AI agent is up to date" }).then(reload, () => {})}>Update agent</button>}
        <button className="btn" disabled={!!job} onClick={onEdit}>Change bot or AI key…</button>
        <span className="spacer" />
        <button className="btn danger" disabled={!!job} onClick={remove}><Trash2 size={13} /> Remove</button>
      </div>

      {st.alerting.length > 0 && <div className="notice">Told you about, not fixed yet: {st.alerting.map((a) => a.replace(":", " ")).join(", ")}</div>}

      <div className="section-title">Recent activity</div>
      {log.length === 0 ? <p className="faint" style={{ padding: "0 16px" }}>Nothing yet.</p> : (
        <table className="table">
          <tbody>{log.map((l, i) => <tr key={i}><td className="faint mono" style={{ width: 110, whiteSpace: "nowrap" }}>{l.time}</td><td>{l.text}</td></tr>)}</tbody>
        </table>
      )}
    </>
  );
}
