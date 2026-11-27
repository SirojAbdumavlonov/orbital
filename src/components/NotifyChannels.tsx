import { Bot, Mail, MessageCircle, Pencil, Send, Trash2 } from "lucide-react";
import { useEffect, useState } from "react";
import { AlertKind, api, asApiError, ChannelConfig, NotifyChannel, NotifyChannelInput, SmtpSecurity, TelegramBot } from "../lib/api";
import { useStore } from "../store";
import { confirm, Modal } from "./ui";

// Settings → Notifications: where health and uptime alerts go besides
// Orbital's own window. Telegram (a bot the user makes), email over SMTP, or
// the AI agent's bot on one of their servers. Tokens and passwords go to the
// vault (notify.rs); nothing here keeps them.

const KIND_ICON = { telegram: MessageCircle, email: Mail, agent: Bot } as const;
const EVENT_LABEL: Record<AlertKind, string> = { health: "Health", uptime: "Uptime", jobs: "Jobs" };

function describe(c: ChannelConfig): string {
  if (c.kind === "telegram") return `Telegram · ${c.chatName || c.chatId}`;
  if (c.kind === "email") return `Email · ${c.to.join(", ")}`;
  return `AI agent on ${c.server}`;
}

export function NotifyChannels() {
  const toast = useStore((x) => x.toast);
  const [list, setList] = useState<NotifyChannel[] | null>(null);
  const [editing, setEditing] = useState<{ channel: NotifyChannel | null; kind: ChannelConfig["kind"] } | null>(null);
  const [testing, setTesting] = useState<string | null>(null);
  const load = () => api.notifyChannels().then(setList, () => setList([]));
  useEffect(() => { void load(); }, []);

  const save = async (c: NotifyChannel, patch: Partial<NotifyChannelInput>) => {
    try {
      await api.saveNotifyChannel(c.id, { name: c.name, enabled: c.enabled, events: c.events, config: c.config, quiet: c.quiet ?? null, secret: "", ...patch });
      void load();
    } catch (e) {
      toast({ kind: "error", title: "Couldn't change it", body: asApiError(e).message });
    }
  };
  const test = async (c: NotifyChannel) => {
    setTesting(c.id);
    try {
      await api.testNotifyChannel(c.id);
      toast({ kind: "success", title: `Test sent to ${c.name}`, body: "Check that it arrived." });
    } catch (e) {
      toast({ kind: "error", title: `${c.name}: the test didn't go through`, body: asApiError(e).message });
    } finally {
      setTesting(null);
    }
  };
  const remove = async (c: NotifyChannel) => {
    if (!(await confirm({ title: `Remove ${c.name}?`, body: "Alerts stop going there. Its saved token or password is deleted from this computer.", confirmLabel: "Remove", danger: true }))) return;
    await api.deleteNotifyChannel(c.id).catch(() => {});
    void load();
  };

  return (
    <div className="notify">
      {list?.length === 0 && <p className="faint notify-empty">Nothing yet: alerts only show in Orbital and as notifications on this computer.</p>}
      {list?.map((c) => {
        const Icon = KIND_ICON[c.config.kind];
        return (
          <div key={c.id} className={`notify-row ${c.enabled ? "" : "off"}`}>
            <Icon size={16} className="notify-icon" />
            <div className="notify-text">
              <b>{c.name}</b>
              <small className="faint">{describe(c.config)}{c.quiet ? ` · quiet ${c.quiet.from}–${c.quiet.to}` : ""}</small>
            </div>
            <div className="notify-events">
              {(["health", "uptime", "jobs"] as AlertKind[]).map((k) => (
                <label key={k} className="check" title={`Send ${EVENT_LABEL[k].toLowerCase()} alerts here`}>
                  <input type="checkbox" checked={c.events.includes(k)} disabled={c.events.length === 1 && c.events.includes(k)}
                    onChange={(e) => void save(c, { events: e.target.checked ? [...c.events, k] : c.events.filter((x) => x !== k) })} />
                  {EVENT_LABEL[k]}
                </label>
              ))}
            </div>
            <input type="checkbox" className="switch" aria-label={`Send to ${c.name}`} checked={c.enabled} onChange={(e) => void save(c, { enabled: e.target.checked })} />
            <button className="icon-btn" title="Send a test message" aria-label={`Test ${c.name}`} disabled={testing === c.id} onClick={() => void test(c)}><Send size={14} className={testing === c.id ? "spin" : ""} /></button>
            <button className="icon-btn" title="Edit" aria-label={`Edit ${c.name}`} onClick={() => setEditing({ channel: c, kind: c.config.kind })}><Pencil size={14} /></button>
            <button className="icon-btn" title="Remove" aria-label={`Remove ${c.name}`} onClick={() => void remove(c)}><Trash2 size={14} /></button>
          </div>
        );
      })}
      <div className="notify-add">
        <button className="btn" onClick={() => setEditing({ channel: null, kind: "telegram" })}><MessageCircle size={13} /> Telegram</button>
        <button className="btn" onClick={() => setEditing({ channel: null, kind: "email" })}><Mail size={13} /> Email (SMTP)</button>
        <button className="btn" onClick={() => setEditing({ channel: null, kind: "agent" })}><Bot size={13} /> AI agent's bot</button>
      </div>
      <p className="faint notify-note">
        Sent from this computer while Orbital is open. For alerts while it's closed, the AI agent on a server checks it every 5 minutes on its own.
      </p>
      {editing && <ChannelDialog channel={editing.channel} kind={editing.kind} onClose={() => setEditing(null)} onSaved={() => { setEditing(null); void load(); }} />}
    </div>
  );
}

function ChannelDialog({ channel, kind, onClose, onSaved }: { channel: NotifyChannel | null; kind: ChannelConfig["kind"]; onClose: () => void; onSaved: () => void }) {
  const profiles = useStore((x) => x.profiles).filter((p) => (p.protocol ?? "sftp") === "sftp");
  const c = channel?.config;
  const [name, setName] = useState(channel?.name ?? (kind === "telegram" ? "My Telegram" : kind === "email" ? "Email" : "AI agent"));
  const [events, setEvents] = useState<AlertKind[]>(channel?.events ?? ["health", "uptime"]);
  const [quietOn, setQuietOn] = useState(!!channel?.quiet);
  const [quietFrom, setQuietFrom] = useState(channel?.quiet?.from ?? "22:00");
  const [quietTo, setQuietTo] = useState(channel?.quiet?.to ?? "07:00");
  const [secret, setSecret] = useState("");
  // Telegram
  const [chatId, setChatId] = useState(c?.kind === "telegram" ? c.chatId : "");
  const [chatName, setChatName] = useState(c?.kind === "telegram" ? c.chatName : "");
  const [bot, setBot] = useState<TelegramBot | null>(null);
  const [finding, setFinding] = useState(false);
  // Email
  const [host, setHost] = useState(c?.kind === "email" ? c.host : "");
  const [port, setPort] = useState(String(c?.kind === "email" ? c.port : 587));
  const [security, setSecurity] = useState<SmtpSecurity>(c?.kind === "email" ? c.security : "startTls");
  const [username, setUsername] = useState(c?.kind === "email" ? c.username : "");
  const [from, setFrom] = useState(c?.kind === "email" ? c.from : "");
  const [to, setTo] = useState(c?.kind === "email" ? c.to.join(", ") : "");
  // Agent
  const [profileId, setProfileId] = useState(c?.kind === "agent" ? c.profileId : profiles[0]?.id ?? "");
  const [busy, setBusy] = useState(false);
  const [problem, setProblem] = useState<string | null>(null);

  const findChats = async () => {
    setFinding(true);
    setProblem(null);
    try {
      const b = await api.telegramLookup(secret);
      setBot(b);
      if (b.chats.length === 1) { setChatId(b.chats[0].id); setChatName(b.chats[0].name); }
      if (b.chats.length === 0) setProblem(`No chats yet: open @${b.username} in Telegram, press Start (or send it any message), then Find chats again.`);
    } catch (e) {
      setProblem(asApiError(e).message.replace(/^Invalid input: /, ""));
    } finally {
      setFinding(false);
    }
  };

  const config = (): ChannelConfig => {
    if (kind === "telegram") return { kind, chatId: chatId.trim(), chatName };
    if (kind === "email") return { kind, host: host.trim(), port: Number(port) || 0, security, username: username.trim(), from: from.trim(), to: to.split(/[\s,;]+/).filter(Boolean) };
    const p = profiles.find((x) => x.id === profileId);
    return { kind, profileId, server: p?.name ?? "" };
  };
  const go = async () => {
    setBusy(true);
    setProblem(null);
    try {
      const saved = await api.saveNotifyChannel(channel?.id ?? null, { name, enabled: channel?.enabled ?? true, events, config: config(), secret, quiet: quietOn ? { from: quietFrom, to: quietTo } : null });
      // A first message proves it works; a failure here still keeps the channel.
      if (!channel) {
        try { await api.testNotifyChannel(saved.id); } catch (e) {
          useStore.getState().toast({ kind: "error", title: `${saved.name} is saved, but the test didn't arrive`, body: asApiError(e).message });
        }
      }
      onSaved();
    } catch (e) {
      setProblem(asApiError(e).message.replace(/^Invalid input: /, ""));
    } finally {
      setBusy(false);
    }
  };
  const toggle = (k: AlertKind) => setEvents((e) => (e.includes(k) ? e.filter((x) => x !== k) : [...e, k]));
  const title = kind === "telegram" ? "Telegram" : kind === "email" ? "Email (SMTP)" : "AI agent's bot";

  return (
    <Modal title={channel ? `Edit ${channel.name}` : `Send alerts by ${title}`} keepOnOutsideClick onClose={onClose}
      subtitle={kind === "telegram" ? "A bot of your own sends them to you or a group." : kind === "email" ? "Through any SMTP account (Gmail, Fastmail, your company's mail server…)." : "The Telegram bot of the AI agent on one of your servers sends them to the account paired with it."}
      footer={<><button className="btn" onClick={onClose}>Cancel</button><button className="btn primary" disabled={busy || events.length === 0} onClick={go}>{busy ? "Saving…" : channel ? "Save" : "Save and send a test"}</button></>}>
      <div className="route-form">
        <label className="field"><span>Name</span><input className="input" value={name} onChange={(e) => setName(e.target.value)} /></label>

        {kind === "telegram" && (
          <>
            <ol className="notify-steps">
              <li>In Telegram, open <b>@BotFather</b>, send <span className="mono">/newbot</span> and follow it. It gives you a token.</li>
              <li>Open your new bot and press <b>Start</b> (for a group: add the bot and say something there).</li>
              <li>Paste the token, then Find chats.</li>
            </ol>
            <label className="field">
              <span>Bot token</span>
              <div className="row-input">
                <input className="input mono" type="password" value={secret} onChange={(e) => setSecret(e.target.value)} placeholder={channel ? "Saved (leave empty to keep it)" : "123456789:AA…"} autoComplete="off" />
                <button className="btn" disabled={!secret.trim() || finding} onClick={findChats}>{finding ? "Looking…" : "Find chats"}</button>
              </div>
              <span className="hint">Kept encrypted on this computer, used only to send these messages.</span>
            </label>
            {bot && bot.chats.length > 0 && (
              <label className="field">
                <span>Send to</span>
                <select className="input" value={chatId} onChange={(e) => { setChatId(e.target.value); setChatName(bot.chats.find((x) => x.id === e.target.value)?.name ?? ""); }}>
                  <option value="">Choose a chat…</option>
                  {bot.chats.map((ch) => <option key={ch.id} value={ch.id}>{ch.name} ({ch.kind})</option>)}
                </select>
                <span className="hint">Chats that wrote to @{bot.username} recently.</span>
              </label>
            )}
            {channel && !bot && <span className="hint">Sends to {chatName || chatId}. Paste the token and Find chats to pick another.</span>}
          </>
        )}

        {kind === "email" && (
          <>
            <div className="field-row">
              <label className="field grow"><span>SMTP server</span><input className="input mono" value={host} onChange={(e) => setHost(e.target.value)} placeholder="smtp.example.com" /></label>
              <label className="field" style={{ width: 90 }}><span>Port</span><input className="input mono" value={port} inputMode="numeric" onChange={(e) => setPort(e.target.value.replace(/\D/g, ""))} /></label>
            </div>
            <div className="segmented">
              {([["startTls", "STARTTLS (587)"], ["tls", "TLS (465)"], ["none", "No encryption"]] as [SmtpSecurity, string][]).map(([id, label]) => (
                <button key={id} className={security === id ? "on" : ""} onClick={() => { setSecurity(id); if (id === "tls" && port === "587") setPort("465"); if (id === "startTls" && port === "465") setPort("587"); }}>{label}</button>
              ))}
            </div>
            <div className="field-row">
              <label className="field grow"><span>User name</span><input className="input" value={username} onChange={(e) => setUsername(e.target.value)} placeholder="Usually your email address" autoComplete="off" /></label>
              <label className="field grow"><span>Password</span><input className="input" type="password" value={secret} onChange={(e) => setSecret(e.target.value)} placeholder={channel ? "Saved (leave empty to keep it)" : "An app password works best"} autoComplete="off" /></label>
            </div>
            <label className="field"><span>From</span><input className="input" value={from} onChange={(e) => setFrom(e.target.value)} placeholder="alerts@example.com" /></label>
            <label className="field"><span>To</span><input className="input" value={to} onChange={(e) => setTo(e.target.value)} placeholder="you@example.com, team@example.com" /><span className="hint">Up to 10 addresses, separated by commas.</span></label>
            <span className="hint">Gmail and Outlook need an app password (a normal one is refused). The password is kept encrypted on this computer.</span>
          </>
        )}

        {kind === "agent" && (
          <>
            <label className="field">
              <span>Server</span>
              <select className="input" value={profileId} onChange={(e) => setProfileId(e.target.value)}>
                {profiles.map((p) => <option key={p.id} value={p.id}>{p.name}</option>)}
              </select>
              <span className="hint">Its AI agent must be installed and paired (the AI agent tab). Orbital must be connected to this server when an alert goes out.</span>
            </label>
          </>
        )}

        <div className="field">
          <span>Send</span>
          <div className="notify-events">
            <label className="check"><input type="checkbox" checked={events.includes("health")} onChange={() => toggle("health")} /> Health alerts <span className="faint">(server needs attention, a service or container failed, back to normal)</span></label>
            <label className="check"><input type="checkbox" checked={events.includes("uptime")} onChange={() => toggle("uptime")} /> Uptime alerts <span className="faint">(a check went down or came back, certificate expiring)</span></label>
            <label className="check"><input type="checkbox" checked={events.includes("jobs")} onChange={() => toggle("jobs")} /> Long jobs <span className="faint">(an install, backup, upload or update that ran 2 minutes or more finished or failed)</span></label>
          </div>
        </div>
        <div className="field">
          <label className="check"><input type="checkbox" checked={quietOn} onChange={(e) => setQuietOn(e.target.checked)} /> Quiet hours</label>
          {quietOn && (
            <div className="row-input" style={{ alignItems: "center" }}>
              <input className="input mono" type="time" value={quietFrom} onChange={(e) => setQuietFrom(e.target.value)} aria-label="Quiet from" style={{ width: 120 }} />
              <span className="faint">to</span>
              <input className="input mono" type="time" value={quietTo} onChange={(e) => setQuietTo(e.target.value)} aria-label="Quiet until" style={{ width: 120 }} />
            </div>
          )}
          <span className="hint">{quietOn ? "In these hours (this computer's time) only problems are sent; recoveries, warnings and finished jobs are left out." : "Send at any hour."}</span>
        </div>
        {problem && <div className="alert" style={{ whiteSpace: "pre-wrap" }}>{problem}</div>}
      </div>
    </Modal>
  );
}
