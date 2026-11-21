import { KeyRound, Lock, LockOpen, Plus, RefreshCw, Search, ShieldCheck, Trash2, UserPlus } from "lucide-react";
import { useMemo, useState } from "react";
import { confirm, DrawerClose, Modal, StateDot } from "../components/ui";
import { api, asApiError, NewUser, UserAccount, UserChange, UsersReport } from "../lib/api";
import { useRemote, withSafetyOverride } from "../lib/remote";
import { SessionState, useStore } from "../store";
import { useKept } from "../lib/kept";

/** Groups people are usually added to, with what they grant. */
const COMMON_GROUPS: Record<string, string> = {
  sudo: "Administrator: can run commands as root",
  wheel: "Administrator: can run commands as root",
  docker: "Can run Docker containers (effectively root)",
  "www-data": "Can write the web server's files",
  adm: "Can read system logs",
  "systemd-journal": "Can read the full journal",
};

function status(u: UserAccount): { state: string; label: string } {
  if (u.passwordStatus === "L") return { state: "inactive", label: u.sshKeys ? "Keys only" : "Locked" };
  if (u.passwordStatus === "NP") return { state: "failed", label: "No password" };
  if (!u.canLogin) return { state: "inactive", label: "No sign-in" };
  return { state: "running", label: "Active" };
}

function GroupPicker({ report, value, onChange }: { report: UsersReport; value: string[]; onChange: (g: string[]) => void }) {
  const [more, setMore] = useState(false);
  const common = Object.keys(COMMON_GROUPS).filter((g) => report.groups.some((x) => x.name === g));
  const others = report.groups.map((g) => g.name).filter((g) => !common.includes(g) && !report.users.some((u) => u.name === g)).sort();
  const toggle = (g: string) => onChange(value.includes(g) ? value.filter((x) => x !== g) : [...value, g]);
  return (
    <div className="group-picker">
      {common.map((g) => (
        <label key={g} className="check"><input type="checkbox" checked={value.includes(g)} onChange={() => toggle(g)} /><span><b className="mono">{g}</b> <span className="faint">{COMMON_GROUPS[g]}</span></span></label>
      ))}
      {more ? (
        <div className="chips">{others.map((g) => <button key={g} className={`chip ${value.includes(g) ? "on" : ""}`} onClick={() => toggle(g)}>{g}</button>)}</div>
      ) : (
        others.length > 0 && <button className="btn ghost" style={{ alignSelf: "flex-start" }} onClick={() => setMore(true)}>Other groups ({others.length})…</button>
      )}
    </div>
  );
}

function AddUserDialog({ s, report, onClose, onDone }: { s: SessionState; report: UsersReport; onClose: () => void; onDone: (r: UsersReport) => void }) {
  const toast = useStore((x) => x.toast);
  const [u, setU] = useState<NewUser>({
    name: "", fullName: "", shell: report.shells.find((x) => x.endsWith("/bash")) ?? null, createHome: true, home: null,
    primaryGroup: null, groups: [], uid: null, expires: null, system: false, sshKeys: [], mustChangePassword: true,
  });
  const [signIn, setSignIn] = useState<"password" | "keys" | "both">("both");
  const [pw, setPw] = useState("");
  const [pw2, setPw2] = useState("");
  const [keys, setKeys] = useState("");
  const [advanced, setAdvanced] = useState(false);
  const [busy, setBusy] = useState(false);
  const set = (p: Partial<NewUser>) => setU((x) => ({ ...x, ...p }));
  const usePw = signIn !== "keys";
  const useKeys = signIn !== "password";
  const keyList = keys.split("\n").map((k) => k.trim()).filter(Boolean);
  const nameOk = /^[a-z_][a-z0-9_-]{0,31}$/.test(u.name);
  const exists = report.users.some((x) => x.name === u.name);
  const pwProblem = usePw ? (pw.length < 8 ? "At least 8 characters" : pw !== pw2 ? "The two passwords differ" : null) : null;
  const ready = nameOk && !exists && !pwProblem && (!useKeys || keyList.length > 0 || signIn === "both");

  async function create() {
    setBusy(true);
    try {
      const r = await api.createUser(s.info.id, { ...u, sshKeys: useKeys ? keyList : [], mustChangePassword: usePw && u.mustChangePassword }, usePw ? pw : null);
      toast({ kind: "success", title: `${u.name} created`, body: [usePw && "password set", useKeys && keyList.length && `${keyList.length} SSH key${keyList.length > 1 ? "s" : ""}`, u.groups.length && `groups: ${u.groups.join(", ")}`].filter(Boolean).join(" · ") });
      onDone(r);
      onClose();
    } catch (e) {
      toast({ kind: "error", title: `Couldn't create ${u.name}`, body: asApiError(e).message });
    } finally {
      setBusy(false);
    }
  }

  return (
    <Modal title="Add a user" wide subtitle={`A new account on ${s.profile.name}.`} onClose={busy ? undefined : onClose}
      footer={<><button className="btn" disabled={busy} onClick={onClose}>Cancel</button><button className="btn primary" disabled={!ready || busy} onClick={create}><UserPlus size={13} /> {busy ? "Creating…" : "Create user"}</button></>}>
      <div className="form">
        <div className="row2" style={{ gridTemplateColumns: "1fr 1fr" }}>
          <label className="field"><span>User name</span>
            <input className="input mono" autoFocus value={u.name} onChange={(e) => set({ name: e.target.value.toLowerCase() })} placeholder="anna" />
            <small className="hint">{exists ? <span className="bad-text">That user already exists</span> : u.name && !nameOk ? <span className="bad-text">Lowercase letters, digits, - and _, starting with a letter</span> : "Used to sign in"}</small>
          </label>
          <label className="field"><span>Full name (optional)</span>
            <input className="input" value={u.fullName ?? ""} onChange={(e) => set({ fullName: e.target.value })} placeholder="Anna Karimova" />
          </label>
        </div>

        <div className="field"><span>Signs in with</span>
          <div className="segmented">
            <button className={signIn === "both" ? "on" : ""} onClick={() => setSignIn("both")}>Password and SSH key</button>
            <button className={signIn === "keys" ? "on" : ""} onClick={() => setSignIn("keys")}>SSH key only</button>
            <button className={signIn === "password" ? "on" : ""} onClick={() => setSignIn("password")}>Password only</button>
          </div>
          <small className="hint">{signIn === "keys" ? "The password is locked, so it can't be guessed. Safest for servers." : "The password also works for sudo."}</small>
        </div>
        {usePw && (
          <>
            <div className="row2" style={{ gridTemplateColumns: "1fr 1fr" }}>
              <label className="field"><span>Password</span><input className="input" type="password" autoComplete="new-password" value={pw} onChange={(e) => setPw(e.target.value)} /></label>
              <label className="field"><span>Repeat password</span><input className="input" type="password" autoComplete="new-password" value={pw2} onChange={(e) => setPw2(e.target.value)} /></label>
            </div>
            {pw && pwProblem && <small className="bad-text">{pwProblem}</small>}
            <label className="check"><input type="checkbox" checked={u.mustChangePassword} onChange={(e) => set({ mustChangePassword: e.target.checked })} /><span>Must choose a new password at first sign-in</span></label>
          </>
        )}
        {useKeys && (
          <label className="field"><span>SSH public keys {signIn === "both" && "(optional)"}</span>
            <textarea className="input mono" rows={3} value={keys} onChange={(e) => setKeys(e.target.value)} placeholder="ssh-ed25519 AAAAC3Nza… anna@laptop" />
            <small className="hint">One per line: the contents of the person's ~/.ssh/id_ed25519.pub. Orbital writes them to ~/.ssh/authorized_keys with the right permissions.</small>
          </label>
        )}

        <div className="field"><span>Groups</span><GroupPicker report={report} value={u.groups} onChange={(groups) => set({ groups })} /></div>

        <button className="btn ghost" style={{ alignSelf: "flex-start" }} onClick={() => setAdvanced(!advanced)}>{advanced ? "Hide" : "Show"} advanced options</button>
        {advanced && (
          <div className="form" style={{ gap: 12 }}>
            <div className="row2" style={{ gridTemplateColumns: "1fr 1fr" }}>
              <label className="field"><span>Shell</span>
                <select className="input mono" value={u.shell ?? ""} onChange={(e) => set({ shell: e.target.value || null })}>
                  <option value="">System default</option>
                  {report.shells.map((x) => <option key={x}>{x}</option>)}
                  <option value="/usr/sbin/nologin">/usr/sbin/nologin (no shell: services, SFTP-only)</option>
                </select>
              </label>
              <label className="field"><span>Home folder</span>
                <input className="input mono" value={u.home ?? ""} onChange={(e) => set({ home: e.target.value || null })} placeholder={`/home/${u.name || "name"}`} />
              </label>
            </div>
            <label className="check"><input type="checkbox" checked={u.createHome} onChange={(e) => set({ createHome: e.target.checked })} /><span>Create the home folder (from /etc/skel)</span></label>
            <div className="row2" style={{ gridTemplateColumns: "1fr 1fr 1fr" }}>
              <label className="field"><span>Primary group</span>
                <select className="input mono" value={u.primaryGroup ?? ""} onChange={(e) => set({ primaryGroup: e.target.value || null })}>
                  <option value="">New group named after the user</option>
                  {report.groups.map((g) => <option key={g.name}>{g.name}</option>)}
                </select>
              </label>
              <label className="field"><span>User ID (optional)</span>
                <input className="input mono" inputMode="numeric" value={u.uid ?? ""} onChange={(e) => set({ uid: e.target.value ? Number(e.target.value) || null : null })} placeholder={`next free from ${report.uidMin}`} />
              </label>
              <label className="field"><span>Account expires</span>
                <input className="input" type="date" value={u.expires ?? ""} onChange={(e) => set({ expires: e.target.value || null })} />
              </label>
            </div>
            <label className="check"><input type="checkbox" checked={u.system} onChange={(e) => set({ system: e.target.checked })} /><span>System account: for a service, not a person (low user ID, no password ageing)</span></label>
          </div>
        )}
      </div>
    </Modal>
  );
}

/** Accounts on the server: who can sign in, how, and with which rights. */
export function UsersView({ s }: { s: SessionState }) {
  const toast = useStore((x) => x.toast);
  const sid = s.info.id;
  const me = s.profile.username;
  const { data: r, setData, error, loading, reload } = useRemote(() => api.listUsers(sid), [sid]);
  const [q, setQ] = useKept(`${s.info.id}:users:q`, "");
  const [showSystem, setShowSystem] = useState(false);
  const [sel, setSel] = useKept<string | null>(`${s.info.id}:users:sel`, null);
  const [adding, setAdding] = useState(false);
  const [groups, setGroups] = useState<string[] | null>(null);
  const [pwFor, setPwFor] = useState(false);
  const [newPw, setNewPw] = useState("");
  const [keyFor, setKeyFor] = useState(false);
  const [newKey, setNewKey] = useState("");
  const [busy, setBusy] = useState(false);

  const list = useMemo(() => (r?.users ?? []).filter((u) => (showSystem || !u.system) && (!q || `${u.name} ${u.fullName} ${u.groups.join(" ")}`.toLowerCase().includes(q.toLowerCase()))), [r, q, showSystem]);
  const cur = r?.users.find((u) => u.name === sel) ?? null;

  async function change(c: UserChange, done: string, password: string | null = null) {
    if (!cur) return false;
    setBusy(true);
    try {
      const next = await withSafetyOverride((force) => api.changeUser(sid, cur.name, c, password, force), "this would affect your access");
      if (next) {
        setData(next);
        toast({ kind: "success", title: done });
      }
      return !!next;
    } catch (e) {
      const err = asApiError(e);
      toast({ kind: "error", title: `Couldn't change ${cur.name}`, body: err.data?.output ?? err.message });
      return false;
    } finally {
      setBusy(false);
    }
  }

  async function remove() {
    if (!cur) return;
    let removeHome = false;
    const ok = await confirm({
      title: `Delete ${cur.name}?`,
      body: <>
        The account can no longer sign in, and its group memberships are removed.
        <label className="check" style={{ marginTop: 10 }}><input type="checkbox" onChange={(e) => (removeHome = e.target.checked)} /><span>Also delete {cur.home} and the user's mail</span></label>
      </>,
      confirmLabel: "Delete user",
      danger: true,
      requireText: cur.name,
    });
    if (ok && (await change({ type: "delete", removeHome }, `${cur.name} deleted`))) setSel(null);
  }

  return (
    <div className="view">
      <div className="toolbar">
        <div className="search"><Search size={14} /><input className="input" placeholder="Find a user or group" value={q} onChange={(e) => setQ(e.target.value)} /></div>
        <label className="check" style={{ alignItems: "center" }}><input type="checkbox" checked={showSystem} onChange={(e) => setShowSystem(e.target.checked)} /><span>Show system accounts</span></label>
        <span className="spacer" />
        {r && <button className="btn primary" onClick={() => setAdding(true)}><UserPlus size={13} /> Add user</button>}
        <button className="icon-btn" aria-label="Refresh" onClick={reload}><RefreshCw size={14} className={loading ? "spin" : ""} /></button>
      </div>
      {error && <div className="empty"><div><h3>Couldn't read the accounts</h3><p>{error.message}</p></div></div>}
      {r && !r.complete && <div className="notice">Sign-in status and SSH keys need root to read. Turn on sudo for this server, then reconnect.</div>}
      {r && (
        <div className="split">
          <div className="scroll">
            <table className="table">
              <thead><tr><th>User</th><th>Groups</th><th>Shell</th><th>Sign-in</th><th className="num">Keys</th></tr></thead>
              <tbody>
                {list.map((u) => {
                  const st = status(u);
                  return (
                    <tr key={u.name} className={`click ${sel === u.name ? "sel" : ""}`} onClick={() => { setSel(u.name); setGroups(null); setPwFor(false); setKeyFor(false); }}>
                      <td>
                        <b className="mono">{u.name}</b>{u.name === me && <span className="badge amber" style={{ marginLeft: 6 }}>you</span>}{u.admin && <span className="badge" style={{ marginLeft: 6 }} title="Can run commands as root"><ShieldCheck size={11} /> admin</span>}
                        {u.fullName && <div className="faint" style={{ fontSize: 12 }}>{u.fullName}</div>}
                      </td>
                      <td className="muted"><div className="cell-clip" style={{ maxWidth: 220 }}>{u.groups.join(", ") || "—"}</div></td>
                      <td className="mono faint">{u.shell.split("/").pop()}</td>
                      <td><StateDot state={st.state} label={st.label} /></td>
                      <td className="num muted">{u.sshKeys ?? "—"}</td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
            {list.length === 0 && <div className="empty" style={{ flex: "none" }}><p>No accounts match.</p></div>}
          </div>
          <aside className="drawer">
            {!cur ? <div className="empty"><p>Select a user to change groups, password, SSH keys or access.</p></div> : (
              <>
                <div className="drawer-head">
                  <DrawerClose onClose={() => setSel(null)} />
                  <h3 className="mono">{cur.name}</h3>
                  <div className="muted">{cur.fullName || "—"} · UID {cur.uid}</div>
                </div>
                <div className="actions">
                  <button className="btn" disabled={busy} onClick={() => { setPwFor(!pwFor); setNewPw(""); }}><KeyRound size={13} /> {cur.passwordStatus === "P" ? "Change password" : "Set password"}</button>
                  <button className="btn" disabled={busy} onClick={() => { setKeyFor(!keyFor); setNewKey(""); }}><Plus size={13} /> Add SSH key</button>
                  {cur.passwordStatus === "L" && !cur.sshKeys
                    ? <button className="btn" disabled={busy} onClick={() => change({ type: "unlock" }, `${cur.name} unlocked`)}><LockOpen size={13} /> Unlock</button>
                    : <button className="btn" disabled={busy || cur.name === me || cur.name === "root"} title={cur.name === me ? "That's the account you are connected as" : "Blocks every sign-in, password and keys"} onClick={async () => { if (await confirm({ title: `Lock ${cur.name}?`, body: "Every sign-in is blocked, by password and by SSH key. Nothing is deleted; unlock to allow it again.", confirmLabel: "Lock account", danger: true })) change({ type: "lock" }, `${cur.name} locked`); }}><Lock size={13} /> Lock</button>}
                  <button className="btn danger" disabled={busy || cur.name === me || cur.name === "root"} onClick={remove}><Trash2 size={13} /> Delete</button>
                </div>
                {pwFor && (
                  <div className="inline-form">
                    <input className="input" type="password" autoFocus autoComplete="new-password" placeholder="New password (at least 8 characters)" value={newPw} onChange={(e) => setNewPw(e.target.value)} />
                    <button className="btn primary" disabled={busy || newPw.length < 8} onClick={async () => { if (await change({ type: "password", mustChange: false }, `Password changed for ${cur.name}`, newPw)) { setPwFor(false); setNewPw(""); } }}>Save</button>
                  </div>
                )}
                {keyFor && (
                  <div className="inline-form">
                    <input className="input mono" autoFocus placeholder="ssh-ed25519 AAAA… name@laptop" value={newKey} onChange={(e) => setNewKey(e.target.value)} />
                    <button className="btn primary" disabled={busy || !newKey.trim()} onClick={async () => { if (await change({ type: "addKey", key: newKey.trim() }, `Key added for ${cur.name}`)) { setKeyFor(false); setNewKey(""); } }}>Add</button>
                  </div>
                )}
                <dl className="kv">
                  <dt>Home</dt><dd className="mono">{cur.home}</dd>
                  <dt>Shell</dt>
                  <dd>
                    <select className="input mono" style={{ height: 26 }} value={cur.shell} disabled={busy} onChange={(e) => change({ type: "shell", shell: e.target.value }, `Shell changed for ${cur.name}`)}>
                      {[...new Set([cur.shell, ...(r.shells ?? []), "/usr/sbin/nologin"])].map((x) => <option key={x}>{x}</option>)}
                    </select>
                  </dd>
                  <dt>Primary group</dt><dd className="mono">{cur.primaryGroup ?? cur.gid}</dd>
                  <dt>Password</dt><dd>{{ P: "Set", L: "Locked", NP: "None (no password needed!)" }[cur.passwordStatus ?? ""] ?? "—"}{cur.passwordChanged && <span className="faint"> · changed {cur.passwordChanged}</span>}</dd>
                  <dt>SSH keys</dt><dd>{cur.sshKeys ?? "—"}</dd>
                </dl>
                <div className="section-title" style={{ display: "flex", alignItems: "center" }}>Groups<span className="spacer" style={{ flex: 1 }} />
                  {groups === null
                    ? <button className="btn ghost" onClick={() => setGroups(cur.groups)}>Edit</button>
                    : <><button className="btn ghost" onClick={() => setGroups(null)}>Cancel</button><button className="btn primary" disabled={busy} onClick={async () => { if (await change({ type: "groups", groups }, `Groups updated for ${cur.name}`)) setGroups(null); }}>Save</button></>}
                </div>
                <div style={{ padding: "0 14px 14px", overflow: "auto" }}>
                  {groups === null
                    ? <div className="chips">{cur.groups.length ? cur.groups.map((g) => <span key={g} className="badge mono">{g}</span>) : <span className="faint">No extra groups</span>}</div>
                    : <GroupPicker report={r} value={groups} onChange={setGroups} />}
                </div>
              </>
            )}
          </aside>
        </div>
      )}
      {adding && r && <AddUserDialog s={s} report={r} onClose={() => setAdding(false)} onDone={setData} />}
    </div>
  );
}
