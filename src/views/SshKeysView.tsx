import { Copy, KeyRound, Laptop, Lock, Plus, RefreshCw, ShieldAlert, Trash2, Upload } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { confirm, Modal } from "../components/ui";
import { api, asApiError, AuthorizedKey, AuthorizedKeys, LocalKey } from "../lib/api";
import { useRemote, withSafetyOverride } from "../lib/remote";
import { SessionState, useStore } from "../store";

const short = (fp: string | null) => (fp ? `${fp.slice(0, 19)}…${fp.slice(-6)}` : "can't be read");
const typeName = (a: string) => (a === "ssh-ed25519" ? "Ed25519" : a === "ssh-rsa" ? "RSA" : a.startsWith("ecdsa") ? "ECDSA" : a.startsWith("sk-") ? "Security key" : a);

function copy(text: string, what: string) {
  navigator.clipboard?.writeText(text).catch(() => {});
  useStore.getState().toast({ kind: "success", title: `${what} copied` });
}

/** After a key from this computer was allowed in: offer to sign in with it instead of a password. */
async function offerKeyLogin(s: SessionState, key: LocalKey) {
  const p = s.profile;
  if (p.authMethod === "key" && p.keyPath === key.path) return;
  const st = useStore.getState();
  st.toast({
    kind: "success",
    title: `${key.name} can sign in as ${p.username}`,
    body: p.authMethod === "password" ? "Sign in with this key instead of the password from now on?" : "Use this key for this server's profile?",
    action: {
      label: "Use this key",
      run: async () => {
        try {
          const { id, createdAt: _c, lastConnectedAt: _l, ...rest } = p;
          const after = await api.updateProfile(id, { ...rest, authMethod: "key", keyPath: key.path });
          await st.loadProfiles();
          st.profileSaved(p, after, false);
          st.toast({ kind: "success", title: `${p.name} now signs in with ${key.name}`, body: key.encrypted ? "Orbital asks for the key's passphrase on the next connection." : undefined });
        } catch (e) {
          st.toast({ kind: "error", title: "Couldn't change the profile", body: asApiError(e).message });
        }
      },
    },
  });
}

function NewKeyDialog({ s, existing, onClose, onMade }: { s: SessionState; existing: string[]; onClose: () => void; onMade: (k: LocalKey) => void }) {
  const base = "id_ed25519";
  const free = !existing.includes(base) ? base : [`${base}_orbital`, ...Array.from({ length: 20 }, (_, i) => `${base}_${i + 2}`)].find((n) => !existing.includes(n))!;
  const [name, setName] = useState(free);
  const [comment, setComment] = useState(`${s.profile.username}@orbital`);
  const [pass, setPass] = useState("");
  const [pass2, setPass2] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const mismatch = pass !== pass2;
  async function make() {
    setBusy(true);
    setError(null);
    try {
      onMade(await api.generateSshKey(name.trim(), comment.trim(), pass || null));
    } catch (e) {
      setError(asApiError(e).message);
    } finally {
      setBusy(false);
    }
  }
  return (
    <Modal title="New SSH key" subtitle="An Ed25519 key pair, made on this computer and saved in ~/.ssh. The private key never leaves this computer." onClose={onClose}
      footer={<><button className="btn" onClick={onClose}>Cancel</button><button className="btn primary" disabled={busy || !name.trim() || mismatch} onClick={make}>{busy ? "Making the key…" : "Make key"}</button></>}>
      <div className="form">
        <label className="field"><span>File name</span>
          <div className="input-suffix"><span className="mono faint">~/.ssh/</span><input className="input mono" value={name} onChange={(e) => setName(e.target.value)} /></div>
          <small className="hint">The public half is saved beside it as <span className="mono">{name || "…"}.pub</span>.</small>
        </label>
        <label className="field"><span>Comment</span><input className="input mono" value={comment} onChange={(e) => setComment(e.target.value)} />
          <small className="hint">A label that shows up next to the key on servers, so you can tell keys apart.</small>
        </label>
        <div className="row-fields">
          <label className="field"><span>Passphrase (optional)</span><input className="input" type="password" value={pass} onChange={(e) => setPass(e.target.value)} /></label>
          <label className="field"><span>Repeat it</span><input className="input" type="password" value={pass2} onChange={(e) => setPass2(e.target.value)} /></label>
        </div>
        <small className="hint">{mismatch ? <span className="bad-text">The passphrases don't match.</span> : "A passphrase encrypts the key file, so a copy of it is useless without the passphrase. Orbital doesn't store it unless you save it for a server."}</small>
        {error && <div className="notice bad-text" style={{ margin: 0 }}>{error}</div>}
      </div>
    </Modal>
  );
}

function AddKeyDialog({ s, local, authorized, onClose, onAdded }: { s: SessionState; local: LocalKey[]; authorized: AuthorizedKeys; onClose: () => void; onAdded: (r: AuthorizedKeys, from?: LocalKey) => void }) {
  const [line, setLine] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const already = new Set(authorized.keys.map((k) => k.fingerprint));
  async function add(text: string, from?: LocalKey) {
    setBusy(true);
    setError(null);
    try {
      onAdded(await api.addAuthorizedKey(s.info.id, text), from);
    } catch (e) {
      setError(asApiError(e).message);
    } finally {
      setBusy(false);
    }
  }
  return (
    <Modal title={`Allow a key to sign in as ${authorized.user}`} subtitle={<span className="mono">{s.info.label} · {authorized.path}</span>} onClose={onClose}
      footer={<><button className="btn" onClick={onClose}>Cancel</button><button className="btn primary" disabled={busy || !line.trim()} onClick={() => add(line)}>{busy ? "Adding…" : "Allow key"}</button></>}>
      <div className="form">
        {local.length > 0 && (
          <div className="field"><span>A key on this computer</span>
            <div className="chips">
              {local.map((k) => (
                <button key={k.fingerprint} className="chip" disabled={busy || already.has(k.fingerprint)} title={already.has(k.fingerprint) ? "Already allowed" : k.fingerprint} onClick={() => add(k.publicLine, k)}>
                  {k.name}{already.has(k.fingerprint) ? " ✓" : ""}
                </button>
              ))}
            </div>
          </div>
        )}
        <label className="field"><span>Or paste a public key</span>
          <textarea className="input mono" rows={4} placeholder="ssh-ed25519 AAAAC3Nza… name@laptop" value={line} onChange={(e) => setLine(e.target.value)} />
          <small className="hint">The contents of someone's <span className="mono">.pub</span> file. Never paste a private key.</small>
        </label>
        {error && <div className="notice bad-text" style={{ margin: 0 }}>{error}</div>}
      </div>
    </Modal>
  );
}

/** Who can sign in to this account with a key, and this computer's own keys. */
export function SshKeysView({ s }: { s: SessionState }) {
  const sid = s.info.id;
  const toast = useStore((x) => x.toast);
  const { data, setData, error, loading, reload } = useRemote(() => api.authorizedKeys(sid), [sid]);
  const [local, setLocal] = useState<LocalKey[]>([]);
  const [adding, setAdding] = useState(false);
  const [making, setMaking] = useState(false);
  const loadLocal = () => api.localSshKeys().then(setLocal).catch(() => setLocal([]));
  useEffect(() => { loadLocal(); }, []);

  const allowed = useMemo(() => new Set((data?.keys ?? []).map((k) => k.fingerprint)), [data]);

  async function remove(k: AuthorizedKey) {
    const ok = await confirm({
      title: `Stop ${k.comment || "this key"} from signing in?`,
      body: `Whoever holds this key can no longer sign in as ${data?.user}. Sessions already open stay open.`,
      confirmLabel: "Remove key",
      danger: true,
    });
    if (!ok) return;
    try {
      const r = await withSafetyOverride((force) => api.removeAuthorizedKey(sid, k.line, force), "removing this key");
      if (r) setData(r);
    } catch (e) {
      toast({ kind: "error", title: "Couldn't remove the key", body: asApiError(e).message });
    }
  }

  async function allow(k: LocalKey) {
    try {
      setData(await api.addAuthorizedKey(sid, k.publicLine));
      offerKeyLogin(s, k);
    } catch (e) {
      toast({ kind: "error", title: "Couldn't add the key", body: asApiError(e).message });
    }
  }

  return (
    <div className="view">
      <div className="toolbar">
        <span className="muted">Keys that can sign in as <b className="mono">{data?.user ?? s.profile.username}</b></span>
        <span className="spacer" />
        <button className="btn primary" disabled={!data} onClick={() => setAdding(true)}><Plus size={13} /> Allow a key</button>
        <button className="icon-btn" aria-label="Refresh" onClick={() => { reload(); loadLocal(); }}><RefreshCw size={14} className={loading ? "spin" : ""} /></button>
      </div>
      <div className="scroll">
        {error && <div className="notice bad-text">{error.message}</div>}
        {data?.missing && <div className="notice">There is no <span className="mono">~/.ssh/authorized_keys</span> yet, so no key can sign in to this account. Allow one below; Orbital creates the file with the permissions sshd requires.</div>}
        {data && data.keys.length > 0 && (
          <table className="table">
            <thead><tr><th>Key</th><th>Type</th><th>Fingerprint</th><th /></tr></thead>
            <tbody>
              {data.keys.map((k) => (
                <tr key={k.line}>
                  <td>
                    <div className="key-name">
                      <b>{k.comment || <span className="faint">no comment</span>}</b>
                      {k.onThisComputer && <span className="badge amber" title="The same key is on this computer (in ~/.ssh, the profile's key, or the SSH agent)"><Laptop size={11} /> This computer</span>}
                      {k.options && <span className="badge" title={k.options}><Lock size={11} /> Restricted</span>}
                    </div>
                    {k.options && <div className="mono faint key-options" title={k.options}>{k.options}</div>}
                  </td>
                  <td className="faint nowrap">{typeName(k.algorithm)}</td>
                  <td className="mono faint" title={k.fingerprint ?? ""}>{short(k.fingerprint)}</td>
                  <td className="nowrap" style={{ textAlign: "right" }}>
                    {!k.fingerprint && <span className="badge warn" title="sshd ignores lines it can't read"><ShieldAlert size={11} /> Unreadable</span>}
                    <button className="icon-btn" aria-label={`Remove ${k.comment || "key"}`} onClick={() => remove(k)}><Trash2 size={14} /></button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}

        <div className="section-title keys-local-title">
          <span>Keys on this computer <span className="faint mono">~/.ssh</span></span>
          <button className="btn" onClick={() => setMaking(true)}><KeyRound size={13} /> New key</button>
        </div>
        {local.length === 0 && <p className="footnote">No key pairs in ~/.ssh yet. Make one to sign in without a password.</p>}
        {local.length > 0 && (
          <table className="table">
            <tbody>
              {local.map((k) => (
                <tr key={k.fingerprint}>
                  <td>
                    <div className="key-name"><b className="mono">{k.name}</b>{k.encrypted && <span className="badge" title="Protected by a passphrase"><Lock size={11} /> Passphrase</span>}</div>
                    <div className="faint key-options">{k.comment}</div>
                  </td>
                  <td className="faint nowrap">{typeName(k.algorithm)}</td>
                  <td className="mono faint" title={k.fingerprint}>{short(k.fingerprint)}</td>
                  <td className="nowrap" style={{ textAlign: "right" }}>
                    <button className="icon-btn" title="Copy the public key" aria-label={`Copy ${k.name}.pub`} onClick={() => copy(k.publicLine, `${k.name}.pub`)}><Copy size={14} /></button>
                    {allowed.has(k.fingerprint)
                      ? <span className="badge ok">Can sign in here</span>
                      : <button className="btn" disabled={!data} onClick={() => allow(k)}><Upload size={13} /> Allow on this server</button>}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
        <p className="footnote">Changes rewrite <span className="mono">~/.ssh/authorized_keys</span> in one step (written beside it, then moved over it) and go into the activity log with each key's fingerprint. Orbital won't remove the last key this computer signs in with unless you confirm you understand you may be locked out.</p>
      </div>
      {adding && data && (
        <AddKeyDialog s={s} local={local} authorized={data} onClose={() => setAdding(false)}
          onAdded={(r, from) => { setData(r); setAdding(false); if (from) offerKeyLogin(s, from); else toast({ kind: "success", title: "Key allowed" }); }} />
      )}
      {making && (
        <NewKeyDialog s={s} existing={local.map((k) => k.name)} onClose={() => setMaking(false)}
          onMade={(k) => { setMaking(false); loadLocal(); toast({ kind: "success", title: `Made ${k.path}`, body: "Click \"Allow on this server\" to sign in with it here." }); }} />
      )}
    </div>
  );
}
