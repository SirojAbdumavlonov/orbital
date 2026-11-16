import { ShieldAlert, ShieldQuestion } from "lucide-react";
import { useEffect, useState } from "react";
import { api } from "../lib/api";
import { useStore } from "../store";
import { Modal } from "./ui";

/** Host-key trust, host-key mismatch and credential prompts — Cyberduck-style connection security. */
export function Prompts() {
  const { prompt, setPrompt, connect, profiles, toast } = useStore();
  const [secret, setSecret] = useState("");
  const [remember, setRemember] = useState(true);
  useEffect(() => setSecret(""), [prompt]);
  if (!prompt) return null;
  const profile = profiles.find((p) => p.id === prompt.profileId);
  const jumpFor = prompt.type === "credentials" && prompt.forProfileId !== prompt.profileId ? profiles.find((p) => p.id === prompt.forProfileId) : null;
  const close = () => setPrompt(null);

  if (prompt.type === "hostkey") {
    return (
      <Modal
        title="Unknown host key"
        subtitle={<>Orbital hasn't connected to <b>{prompt.host}:{prompt.port}</b> before.</>}
        onClose={close}
        footer={
          <>
            <button className="btn" onClick={close}>Cancel</button>
            <button
              className="btn primary"
              // The confirmation names the endpoint it was shown for; the engine pins it for that endpoint only.
              onClick={() => connect(prompt.profileId, { ...prompt.opts, acceptFingerprint: prompt.fingerprint, acceptHost: prompt.host, acceptPort: prompt.port })}
            >
              Trust and connect
            </button>
          </>
        }
      >
        <div style={{ display: "flex", gap: 12 }}>
          <ShieldQuestion size={28} color="var(--amber)" style={{ flexShrink: 0 }} />
          <p style={{ margin: 0 }} className="muted">
            Compare this fingerprint with the one your hosting provider or server admin gave you (on the server:
            <span className="mono"> ssh-keygen -lf /etc/ssh/ssh_host_{prompt.algorithm.includes("ed25519") ? "ed25519" : "ecdsa"}_key.pub</span>).
            Once trusted, Orbital refuses to connect if it ever changes.
          </p>
        </div>
        <div className="fingerprint">
          <div className="faint" style={{ fontSize: 12 }}>{prompt.algorithm}</div>
          {prompt.fingerprint}
        </div>
      </Modal>
    );
  }

  if (prompt.type === "mismatch") {
    return (
      <Modal
        title="Host key has changed"
        onClose={close}
        footer={
          <>
            <button
              className="btn danger left"
              onClick={async () => {
                await api.forgetHost(prompt.host, prompt.port);
                close();
                toast({ kind: "info", title: "Old host key removed", body: "Connect again to review the new fingerprint." });
              }}
            >
              Remove old key
            </button>
            <button className="btn primary" autoFocus onClick={close}>Don't connect</button>
          </>
        }
      >
        <div className="alert" style={{ display: "flex", gap: 10 }}>
          <ShieldAlert size={20} style={{ flexShrink: 0 }} />
          <div>
            The server at <b>{prompt.host}:{prompt.port}</b> presented a different key than the one you trusted. This happens when a
            server is reinstalled — or when someone is intercepting the connection. Only remove the old key if you know the server was rebuilt.
          </div>
        </div>
        <div className="fingerprint">
          <div className="faint" style={{ fontSize: 12 }}>Trusted</div>
          {prompt.expected}
          <div className="faint" style={{ fontSize: 12, marginTop: 8 }}>Presented now</div>
          {prompt.actual}
        </div>
      </Modal>
    );
  }

  const submit = () =>
    connect(
      prompt.profileId,
      prompt.forProfileId === prompt.profileId
        ? { ...prompt.opts, secret, remember }
        : { ...prompt.opts, hopSecrets: { ...prompt.opts?.hopSecrets, [prompt.forProfileId]: secret }, remember },
    );
  return (
    <Modal
      title={`${prompt.what} for ${prompt.user}@${prompt.host}`}
      subtitle={jumpFor ? `${jumpFor.name} — jump host for ${profile?.name ?? "this server"}` : profile?.name}
      onClose={close}
      footer={
        <>
          <button className="btn" onClick={close}>Cancel</button>
          <button className="btn primary" onClick={submit}>Connect</button>
        </>
      }
    >
      {prompt.error && <div className="alert">{prompt.error}</div>}
      <input
        className="input"
        type="password"
        autoFocus
        aria-label={prompt.what}
        value={secret}
        onChange={(e) => setSecret(e.target.value)}
        onKeyDown={(e) => e.key === "Enter" && submit()}
      />
      <label className="check">
        <input type="checkbox" checked={remember} onChange={(e) => setRemember(e.target.checked)} />
        <span>Remember on this computer (encrypted)</span>
      </label>
    </Modal>
  );
}
