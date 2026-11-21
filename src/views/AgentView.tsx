import { ArrowUpCircle, Bell, BellOff, Bot, CheckCircle2, Cpu, Sun, Copy, KeyRound, Play, RefreshCw, Send, ShieldCheck, Square, Trash2 } from "lucide-react";
import { useEffect, useState } from "react";
import { JobProgress } from "../components/JobProgress";
import { confirm, Modal, StateDot } from "../components/ui";
import { AgentCheck, AgentSetup, AgentStatus, AiModel, AiProvider, api, asApiError, PairCode } from "../lib/api";
import { relativeTime } from "../lib/format";
import { runJob, useRunningJob } from "../lib/jobs";
import { useRemote } from "../lib/remote";
import { SessionState, useStore } from "../store";

/** Install this Orbital's agent on the server (keeps the bot, key, model and pairing). */
function updateAgent(sid: string) {
  return runJob((p) => api.agentUpdate(sid, p), { key: `agent:${sid}`, title: "Updating the AI agent", sid, quietHere: false, done: () => "The AI agent is up to date" });
}

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
      telegram_down: r.kind === "conflict" || r.kind === "token" ? `Telegram: ${r.error}` : `Telegram unreachable for ${r.minutes} min: ${r.error}. Messages sent meanwhile arrive when it's back.`,
      telegram_back: `Telegram reachable again after ${r.minutes} min; nothing was lost`,
      test_message: "Test message sent",
      daily_summary: "Daily summary sent",
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
  const [model, setModel] = useState(status.model ?? "");
  const [alerts, setAlerts] = useState(status.installed ? status.alerts : true);
  const [checked, setChecked] = useState<AgentCheck | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const job = useRunningJob(`agent:${sid}`);
  const setup: AgentSetup = { telegramToken: token.trim(), provider, apiKey: key.trim(), model: model.trim() || null, alerts };
  const ready = token.trim().length > 30 && key.trim().length > 20;
  // Typing a new token or key, or switching provider, needs a new check (and a model from its list).
  const changed = () => setChecked(null);

  async function check() {
    setBusy(true);
    setErr(null);
    try {
      const c = await api.agentCheck({ ...setup, model: null });
      setChecked(c);
      // Keep the chosen model if the key can use it; else the recommended one, else the newest.
      const ids = c.models.map((m) => m.id);
      const want = ids.includes(model) ? model : provider === "claude" && ids.includes("claude-opus-5-5") ? "claude-opus-5-5" : ids[0];
      setModel(want ?? "");
    } catch (e) {
      setChecked(null);
      setErr(asApiError(e).message.replace(/^Invalid input: /, ""));
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
      setErr(asApiError(e).message.replace(/^Invalid input: /, ""));
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
            <li><KeyRound size={13} /> The bot token and the AI key are kept on the server only, where only the administrator can read them.</li>
          </ul>
        </div>
      )}
      {err && <div className="alert">{err}</div>}
      <div className="form">
        <div className="agent-step"><span className="wizard-num">1</span> <b>Your Telegram bot</b></div>
        <p className="hint" style={{ marginTop: -4 }}>In Telegram, open <span className="mono">@BotFather</span>, send <span className="mono">/newbot</span>, pick a name, and copy the token it gives you.</p>
        <label className="field"><span>Bot token</span><input className="input mono" type="password" autoComplete="off" placeholder="123456789:AAH…" value={token} onChange={(e) => { setToken(e.target.value); changed(); }} /></label>

        <div className="agent-step"><span className="wizard-num">2</span> <b>The AI</b></div>
        <div className="segmented" role="tablist">
          <button className={provider === "claude" ? "on" : ""} onClick={() => { setProvider("claude"); setModel(""); changed(); }}>Claude</button>
          <button className={provider === "openai" ? "on" : ""} onClick={() => { setProvider("openai"); setModel(""); changed(); }}>ChatGPT</button>
        </div>
        <label className="field">
          <span>{provider === "claude" ? "Claude API key" : "OpenAI API key"}</span>
          <input className="input mono" type="password" autoComplete="off" placeholder={provider === "claude" ? "sk-ant-…" : "sk-…"} value={key} onChange={(e) => { setKey(e.target.value); changed(); }} />
          <small className="hint">{provider === "claude" ? "From console.anthropic.com → API keys. Billed by Anthropic per message." : "From platform.openai.com → API keys. Billed by OpenAI per message."}</small>
        </label>
        <label className="field">
          <span>Model</span>
          {checked ? (
            <select className="input" value={model} onChange={(e) => setModel(e.target.value)}>
              {checked.models.map((m) => (
                <option key={m.id} value={m.id}>{m.name}{m.name !== m.id ? ` (${m.id})` : ""}{m.id === "claude-opus-5-5" ? " · recommended" : ""}</option>
              ))}
            </select>
          ) : (
            <div className="input faint" style={{ display: "flex", alignItems: "center" }}>Check the key to choose from the models it can use</div>
          )}
          <small className="hint">{provider === "claude" ? "Claude Opus 5.5 is the most capable for this; Haiku is the cheapest." : "Only models that can chat are listed."}</small>
        </label>
        <label className="check"><input type="checkbox" checked={alerts} onChange={(e) => setAlerts(e.target.checked)} /><span>Message me when a service fails, a disk is over 90% full or a container keeps restarting (checked every 5 minutes)</span></label>

        <div className="agent-actions">
          {checked && <span className="badge ok"><CheckCircle2 size={12} /> @{checked.bot} and the key work · {checked.models.length} models</span>}
          <span className="spacer" />
          {onCancel && <button className="btn" onClick={onCancel}>Cancel</button>}
          <button className="btn" disabled={!ready || busy || !!job} onClick={check}>{busy ? "Checking…" : "Check"}</button>
          <button className="btn primary" disabled={!ready || !checked || !model || !!job} title={checked ? undefined : "Check the bot token and the key first"} onClick={install}><Send size={13} /> {status.installed ? "Save on the server" : `Install on ${s.profile.name}`}</button>
        </div>
        <p className="footnote">The agent installs in its own folder on the server and runs in the background. Remove it here any time; nothing else on the server changes.</p>
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
    if (!(await confirm({ title: `Remove the AI agent from ${s.profile.name}?`, body: "The bot stops answering, and its config (with the bot token and the AI key), its activity log and its folder are deleted from the server.", confirmLabel: "Remove agent", danger: true, requireText: "remove" }))) return;
    await act(() => api.agentRemove(sid), "The AI agent was removed");
  }

  const [choosing, setChoosing] = useState(false);
  const [testing, setTesting] = useState(false);
  async function sendTest() {
    setTesting(true);
    try {
      const to = await api.agentTest(sid);
      toast({ kind: "success", title: `Test message sent${to ? ` to ${to}` : ""}`, body: "Check Telegram: if it arrived, the bot works end to end." });
    } catch (e) {
      toast({ kind: "error", title: "The test message didn't go out", body: asApiError(e).message.replace(/^Invalid input: /, "") });
    } finally {
      setTesting(false);
    }
  }
  const setDaily = (t: string | null) => act(() => api.agentSetDaily(sid, t), t ? `Daily summary at ${t} (server time)` : "Daily summary is off");
  // Newest first; the same line repeated back to back shows once with a count (and its time span).
  const log: { time: string; text: string; n: number; first: string }[] = [];
  for (const l of (st.recent.map(activity).filter(Boolean) as { time: string; text: string }[]).reverse()) {
    const prev = log[log.length - 1];
    if (prev && prev.text === l.text) {
      prev.n += 1;
      prev.first = l.time;
    } else if (log.length < 25) {
      log.push({ ...l, n: 1, first: l.time });
    }
  }
  return (
    <>
      {st.updateAvailable && (
        <div className="agent-update">
          <ArrowUpCircle size={18} />
          <div>
            <b>An update for this agent is ready</b>
            <div className="faint">This server runs agent {st.version ?? "an older version"}; Orbital has {st.latestVersion}. It makes the bot answer reliably, adds changing the model, test messages and the daily summary. Your bot, key, model and pairing stay as they are.</div>
          </div>
          <button className="btn primary" disabled={!!job} onClick={() => updateAgent(sid).then(reload, () => {})}>Update agent</button>
        </div>
      )}
      <section className="panel agent-card">
        {st.owner ? (
          <div className="agent-paired">
            <CheckCircle2 size={18} className="ok-text" />
            <div>
              <b>Answering {st.owner}{bot ? <> through <span className="mono">@{bot}</span></> : ""}</b>
              <div className="faint">{st.pairedAt ? `Paired ${relativeTime(st.pairedAt)} · ` : ""}{st.provider === "openai" ? "ChatGPT" : "Claude"} · model <span className="mono">{st.model}</span></div>
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
        <button className="btn" disabled={!!job} onClick={() => setChoosing(true)}><Cpu size={13} /> Change model</button>
        {st.owner && <button className="btn" disabled={!!job || testing} onClick={sendTest}><Send size={13} /> {testing ? "Sending…" : "Send a test message"}</button>}
        <button className="btn" disabled={!!job} onClick={onEdit}>Change bot or AI key…</button>
        <span className="spacer" />
        <button className="btn danger" disabled={!!job} onClick={remove}><Trash2 size={13} /> Remove</button>
      </div>

      {choosing && <ModelDialog s={s} current={st.model} provider={st.provider} onClose={() => setChoosing(false)} onSaved={() => { setChoosing(false); reload(); }} />}

      <div className="agent-daily">
        <Sun size={14} className="faint" />
        <span>Daily summary in Telegram</span>
        <select className="input" style={{ width: 150, height: 28 }} value={st.dailyReport ?? ""} disabled={!!job} onChange={(e) => setDaily(e.target.value || null)}>
          <option value="">Off</option>
          {["07:00", "08:00", "09:00", "10:00", "12:00", "18:00", "21:00"].map((t) => <option key={t} value={t}>at {t}</option>)}
        </select>
        <span className="faint">Health, disks, memory, failed services and updates, once a day (server time). No AI, so it costs nothing.</span>
      </div>

      {st.alerting.length > 0 && <div className="notice">Told you about, not fixed yet: {st.alerting.map((a) => a.replace(":", " ")).join(", ")}</div>}

      <div className="section-title">Recent activity</div>
      {log.length === 0 ? <p className="faint" style={{ padding: "0 16px" }}>Nothing yet.</p> : (
        <table className="table">
          <tbody>{log.map((l, i) => <tr key={i}><td className="faint mono" style={{ width: 110, whiteSpace: "nowrap" }} title={l.n > 1 ? `${l.first} – ${l.time}` : undefined}>{l.time}</td><td>{l.text}{l.n > 1 && <span className="badge" style={{ marginLeft: 8 }}>×{l.n}</span>}</td></tr>)}</tbody>
        </table>
      )}
    </>
  );
}

/**
 * Switch the agent's model (Sonnet ↔ Opus, …) without typing the key again:
 * the agent lists what its key can use, and takes the new one from the next message.
 */
function ModelDialog({ s, current, provider, onClose, onSaved }: { s: SessionState; current: string | null; provider: string | null; onClose: () => void; onSaved: () => void }) {
  const sid = s.info.id;
  const [models, setModels] = useState<AiModel[] | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [pick, setPick] = useState(current ?? "");
  const [busy, setBusy] = useState(false);
  const [updating, setUpdating] = useState(false);
  useEffect(() => {
    api.agentModels(sid).then(
      (m) => {
        setModels(m);
        if (!m.some((x) => x.id === pick) && m[0]) setPick(m[0].id);
      },
      (e) => setErr(asApiError(e).message.replace(/^Invalid input: /, "")),
    );
  }, [sid]);
  async function save() {
    setBusy(true);
    setErr(null);
    try {
      await api.agentSetModel(sid, pick);
      useStore.getState().toast({ kind: "success", title: `The agent now uses ${models?.find((m) => m.id === pick)?.name ?? pick}`, body: "From your next message in Telegram; it starts a fresh conversation." });
      onSaved();
    } catch (e) {
      setErr(asApiError(e).message.replace(/^Invalid input: /, ""));
    } finally {
      setBusy(false);
    }
  }
  return (
    <Modal
      title="Change the AI model"
      subtitle={<>For the agent on <b>{s.profile.name}</b>. Your API key stays where it is.</>}
      onClose={onClose}
      footer={<><button className="btn" onClick={onClose}>Cancel</button><button className="btn primary" disabled={!models || !pick || pick === current || busy} onClick={save}>{busy ? "Saving…" : "Use this model"}</button></>}
    >
      {err && (
        <div className="alert">
          {err}
          {err.includes("Update agent") && (
            <div style={{ marginTop: 8 }}>
              <button className="btn primary" disabled={updating} onClick={async () => {
                setUpdating(true);
                try {
                  await updateAgent(sid);
                  setErr(null);
                  setModels(await api.agentModels(sid));
                } catch (e) {
                  setErr(asApiError(e).message);
                } finally {
                  setUpdating(false);
                }
              }}>{updating ? "Updating the agent…" : "Update agent now"}</button>
            </div>
          )}
        </div>
      )}
      {!models && !err && <p className="faint">Asking {provider === "openai" ? "OpenAI" : "Anthropic"} which models the key can use…</p>}
      {models && (
        <div className="model-list" role="radiogroup" aria-label="Model">
          {models.map((m) => (
            <label key={m.id} className={`model-row ${pick === m.id ? "on" : ""}`}>
              <input type="radio" name="model" checked={pick === m.id} onChange={() => setPick(m.id)} />
              <span className="model-name">{m.name}{m.name !== m.id && <span className="faint mono"> {m.id}</span>}</span>
              {m.id === current && <span className="badge">in use</span>}
              {m.id === "claude-opus-5-5" && m.id !== current && <span className="badge ok">recommended</span>}
            </label>
          ))}
        </div>
      )}
      <p className="hint" style={{ marginTop: 10 }}>{provider === "openai" ? "Bigger models answer better and cost more per message." : "Opus answers best; Sonnet is faster and cheaper; Haiku is the cheapest."} Each message is billed by the provider at that model's price.</p>
    </Modal>
  );
}
