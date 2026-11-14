import { open } from "@tauri-apps/plugin-dialog";
import { FolderOpen } from "lucide-react";
import { useEffect, useState } from "react";
import { api, asApiError, AuthMethod, ProfileInput, SudoMode } from "../lib/api";
import { useStore } from "../store";
import { confirm, Modal } from "./ui";

const COLORS = ["#8fb4ff", "#4fbf8b", "#f2b544", "#e5645a", "#c79bff", "#7fd1b9"];

export function ProfileDialog() {
  const { editing, setEditing, loadProfiles, toast, connect } = useStore();
  const existing = editing && editing !== "new" ? editing : null;
  const [f, setF] = useState<ProfileInput>(blank());
  const [password, setPassword] = useState("");
  const [sudoPassword, setSudoPassword] = useState("");
  const [savedSecret, setSavedSecret] = useState(false);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (!editing) return;
    setPassword("");
    setSudoPassword("");
    if (existing) {
      setF({ ...existing });
      api.hasSecret(existing.id, existing.authMethod === "key" ? "keyPassphrase" : "password").then(setSavedSecret).catch(() => {});
    } else {
      setF(blank());
      setSavedSecret(false);
    }
  }, [editing]);

  if (!editing) return null;
  const set = <K extends keyof ProfileInput>(k: K, v: ProfileInput[K]) => setF((x) => ({ ...x, [k]: v }));
  const close = () => setEditing(null);

  async function save(andConnect: boolean) {
    setBusy(true);
    try {
      const input: ProfileInput = { ...f, name: f.name || f.host, port: Number(f.port) || 22 };
      const p = existing ? await api.updateProfile(existing.id, input) : await api.createProfile(input);
      if (password) await api.setSecret(p.id, f.authMethod === "key" ? "keyPassphrase" : "password", password);
      if (sudoPassword) await api.setSecret(p.id, "sudoPassword", sudoPassword);
      await loadProfiles();
      close();
      if (andConnect) connect(p.id);
    } catch (e) {
      toast({ kind: "error", title: "Couldn't save server", body: asApiError(e).message });
    } finally {
      setBusy(false);
    }
  }

  async function remove() {
    if (!existing) return;
    const ok = await confirm({
      title: `Delete “${existing.name}”?`,
      body: "The bookmark and any passwords saved for it in the Keychain will be removed. The activity log is kept.",
      confirmLabel: "Delete server",
      danger: true,
    });
    if (!ok) return;
    await api.deleteProfile(existing.id);
    await loadProfiles();
    close();
  }

  async function pickKey() {
    const path = await open({ multiple: false, directory: false, title: "Choose a private key", defaultPath: "~/.ssh" });
    if (typeof path === "string") set("keyPath", path);
  }

  const secretLabel = f.authMethod === "key" ? "Key passphrase" : "Password";
  return (
    <Modal
      title={existing ? "Edit server" : "Add server"}
      subtitle="Passwords and passphrases are stored only in your macOS Keychain."
      onClose={close}
      footer={
        <>
          {existing && <button className="btn danger left" onClick={remove}>Delete</button>}
          <button className="btn" onClick={close}>Cancel</button>
          <button className="btn" disabled={busy || !f.host || !f.username} onClick={() => save(false)}>Save</button>
          <button className="btn primary" disabled={busy || !f.host || !f.username} onClick={() => save(true)}>Save and connect</button>
        </>
      }
    >
      <div className="grid-host">
        <label className="field">
          <span>Host</span>
          <input className="input mono" autoFocus placeholder="203.0.113.10 or web-01.example.com" value={f.host} onChange={(e) => set("host", e.target.value)} />
        </label>
        <label className="field">
          <span>Port</span>
          <input className="input mono" inputMode="numeric" value={f.port ?? 22} onChange={(e) => set("port", Number(e.target.value.replace(/\D/g, "")) || 22)} />
        </label>
      </div>
      <div className="grid2">
        <label className="field">
          <span>Username</span>
          <input className="input mono" placeholder="deploy" value={f.username} onChange={(e) => set("username", e.target.value)} />
        </label>
        <label className="field">
          <span>Name</span>
          <input className="input" placeholder={f.host || "Production web"} value={f.name} onChange={(e) => set("name", e.target.value)} />
        </label>
      </div>

      <div className="field">
        <span>Sign in with</span>
        <div className="segmented">
          {(["password", "key", "agent"] as AuthMethod[]).map((m) => (
            <button key={m} className={f.authMethod === m ? "on" : ""} onClick={() => set("authMethod", m)}>
              {m === "password" ? "Password" : m === "key" ? "Private key" : "SSH agent"}
            </button>
          ))}
        </div>
      </div>

      {f.authMethod === "key" && (
        <label className="field">
          <span>Private key</span>
          <div style={{ display: "flex", gap: 6 }}>
            <input className="input mono" placeholder="~/.ssh/id_ed25519" value={f.keyPath ?? ""} onChange={(e) => set("keyPath", e.target.value)} />
            <button className="btn" onClick={pickKey}><FolderOpen size={14} /> Choose</button>
          </div>
        </label>
      )}
      {f.authMethod === "agent" && (
        <p className="note">Uses the keys loaded in your SSH agent (ssh-add, 1Password, Secretive). Nothing is stored by Outpost.</p>
      )}
      {f.authMethod !== "agent" && (
        <label className="field">
          <span>{secretLabel}</span>
          <input
            className="input"
            type="password"
            placeholder={savedSecret ? "Saved in Keychain — leave empty to keep" : f.authMethod === "key" ? "Leave empty if the key has none" : "Ask when connecting"}
            value={password}
            onChange={(e) => setPassword(e.target.value)}
          />
        </label>
      )}

      <div className="grid2">
        <label className="field">
          <span>Administrator access</span>
          <select className="input" value={f.sudoMode ?? "auto"} onChange={(e) => set("sudoMode", e.target.value as SudoMode)}>
            <option value="auto">Automatic (sudo -n unless root)</option>
            <option value="password">sudo with password</option>
            <option value="noPassword">sudo without password</option>
            <option value="never">Never elevate</option>
          </select>
          <span className="hint">Used to restart services and save root-owned files.</span>
        </label>
        {f.sudoMode === "password" ? (
          <label className="field">
            <span>sudo password</span>
            <input className="input" type="password" placeholder={f.authMethod === "password" ? "Same as login if empty" : "Required for sudo"} value={sudoPassword} onChange={(e) => setSudoPassword(e.target.value)} />
          </label>
        ) : (
          <label className="field">
            <span>Open folder</span>
            <input className="input mono" placeholder="Home directory" value={f.initialPath ?? ""} onChange={(e) => set("initialPath", e.target.value || null)} />
          </label>
        )}
      </div>

      <div className="grid2">
        <label className="field">
          <span>Group</span>
          <input className="input" placeholder="Production" value={f.groupName ?? ""} onChange={(e) => set("groupName", e.target.value || null)} />
        </label>
        <div className="field">
          <span>Label colour</span>
          <div style={{ display: "flex", gap: 6, height: 30, alignItems: "center" }}>
            {COLORS.map((c) => (
              <button
                key={c}
                aria-label={`Colour ${c}`}
                onClick={() => set("color", f.color === c ? null : c)}
                style={{ width: 20, height: 20, borderRadius: "50%", background: c, border: f.color === c ? "2px solid var(--ink)" : "2px solid transparent" }}
              />
            ))}
          </div>
        </div>
      </div>
    </Modal>
  );
}

function blank(): ProfileInput {
  return { name: "", host: "", port: 22, username: "", authMethod: "agent", keyPath: null, sudoMode: "auto", groupName: null, color: null, initialPath: null };
}
