import { open } from "@tauri-apps/plugin-dialog";
import { FolderOpen } from "lucide-react";
import { useEffect, useState } from "react";
import { api, asApiError, AuthMethod, Profile, ProfileInput, Protocol, SudoMode } from "../lib/api";
import { useStore } from "../store";
import { confirm, Modal } from "./ui";

const COLORS = ["#8fb4ff", "#4fbf8b", "#f2b544", "#e5645a", "#c79bff", "#7fd1b9"];

export const PROTOCOL_NAMES: Record<Protocol, string> = {
  sftp: "SFTP", ftp: "FTP", ftps: "FTPS", webdav: "WebDAV", webdavs: "WebDAV (HTTPS)", s3: "S3",
};

/** Kinds of connection, with the settings each service needs filled in. */
interface Preset {
  id: string;
  label: string;
  group: "Servers" | "File servers" | "Cloud storage";
  protocol: Protocol;
  port: number;
  host?: string;
  hostHint?: string;
  region?: string;
  pathStyle?: boolean;
  path?: string;
  note?: string;
}
const PRESETS: Preset[] = [
  { id: "sftp", label: "SSH server (SFTP)", group: "Servers", protocol: "sftp", port: 22 },
  { id: "ftp", label: "FTP", group: "File servers", protocol: "ftp", port: 21, note: "Plain FTP sends the password unencrypted. Use FTPS or SFTP when the server offers it." },
  { id: "ftps", label: "FTPS (FTP over TLS)", group: "File servers", protocol: "ftps", port: 21, note: "Explicit TLS (AUTH TLS). The server's certificate must be valid." },
  { id: "webdavs", label: "WebDAV (HTTPS)", group: "File servers", protocol: "webdavs", port: 443 },
  { id: "webdav", label: "WebDAV (HTTP)", group: "File servers", protocol: "webdav", port: 80, note: "Plain HTTP sends the password unencrypted." },
  { id: "nextcloud", label: "Nextcloud", group: "File servers", protocol: "webdavs", port: 443, hostHint: "cloud.example.com", path: "/remote.php/dav/files/{user}", note: "Use an app password (Settings → Security → Devices & sessions)." },
  { id: "owncloud", label: "ownCloud", group: "File servers", protocol: "webdavs", port: 443, hostHint: "cloud.example.com", path: "/remote.php/webdav" },
  { id: "aws", label: "Amazon S3", group: "Cloud storage", protocol: "s3", port: 443, host: "s3.amazonaws.com", region: "us-east-1", note: "Buckets in other regions are found automatically." },
  { id: "b2", label: "Backblaze B2", group: "Cloud storage", protocol: "s3", port: 443, host: "s3.us-west-004.backblazeb2.com", region: "us-west-004", note: "Use an application key; the endpoint is on the bucket's page." },
  { id: "r2", label: "Cloudflare R2", group: "Cloud storage", protocol: "s3", port: 443, hostHint: "ACCOUNT_ID.r2.cloudflarestorage.com", region: "auto", note: "Create an R2 API token with read and write access." },
  { id: "gcs", label: "Google Cloud Storage", group: "Cloud storage", protocol: "s3", port: 443, host: "storage.googleapis.com", region: "auto", note: "Uses HMAC keys (Cloud Storage → Settings → Interoperability)." },
  { id: "wasabi", label: "Wasabi", group: "Cloud storage", protocol: "s3", port: 443, host: "s3.wasabisys.com", region: "us-east-1" },
  { id: "spaces", label: "DigitalOcean Spaces", group: "Cloud storage", protocol: "s3", port: 443, host: "nyc3.digitaloceanspaces.com", region: "nyc3" },
  { id: "minio", label: "MinIO or other S3-compatible", group: "Cloud storage", protocol: "s3", port: 9000, hostHint: "minio.example.com", region: "us-east-1", pathStyle: true },
];

function presetFor(p: Pick<Profile, "protocol" | "host" | "initialPath">): Preset {
  const proto = p.protocol ?? "sftp";
  const byHost = PRESETS.find((x) => x.protocol === proto && ((x.host && p.host === x.host) || (x.hostHint?.startsWith("ACCOUNT") && p.host.endsWith(".r2.cloudflarestorage.com")) || (x.id === "b2" && p.host.endsWith(".backblazeb2.com")) || (x.id === "spaces" && p.host.endsWith(".digitaloceanspaces.com")) || (x.id === "wasabi" && p.host.endsWith(".wasabisys.com"))));
  const byPath = PRESETS.find((x) => x.protocol === proto && x.path && p.initialPath?.startsWith(x.path.replace("{user}", "")));
  return byHost ?? byPath ?? PRESETS.find((x) => x.protocol === proto) ?? PRESETS[0];
}

export function ProfileDialog() {
  const { editing, setEditing, loadProfiles, toast, connect, profiles, profileSaved, setReconnectAsk, reconnect, sessions } = useStore();
  const existing = editing && editing !== "new" ? editing : null;
  const [f, setF] = useState<ProfileInput>(blank());
  const [password, setPassword] = useState("");
  const [sudoPassword, setSudoPassword] = useState("");
  const [savedSecret, setSavedSecret] = useState(false);
  const [busy, setBusy] = useState(false);
  const [preset, setPreset] = useState<Preset>(PRESETS[0]);

  useEffect(() => {
    if (!editing) return;
    setPassword("");
    setSudoPassword("");
    if (existing) {
      setF({ ...existing });
      setPreset(presetFor(existing));
      api.hasSecret(existing.id, existing.protocol === "sftp" && existing.authMethod === "key" ? "keyPassphrase" : "password").then(setSavedSecret).catch(() => {});
    } else {
      setF(blank());
      setPreset(PRESETS[0]);
      setSavedSecret(false);
    }
  }, [editing]);

  if (!editing) return null;
  const set = <K extends keyof ProfileInput>(k: K, v: ProfileInput[K]) => setF((x) => ({ ...x, [k]: v }));
  const close = () => setEditing(null);
  const proto: Protocol = f.protocol ?? "sftp";
  const ssh = proto === "sftp";
  const s3 = proto === "s3";
  const dav = proto === "webdav" || proto === "webdavs";
  function pick(id: string) {
    const p = PRESETS.find((x) => x.id === id)!;
    setPreset(p);
    setF((x) => ({
      ...x,
      protocol: p.protocol,
      port: p.port,
      host: p.host ?? (PRESETS.some((q) => q.host && q.host === x.host) ? "" : x.host),
      region: p.region ?? null,
      pathStyle: !!p.pathStyle,
      initialPath: p.path ? p.path.replace("{user}", x.username || "{user}") : null,
      jumpProfileId: p.protocol === "sftp" ? x.jumpProfileId : null,
      authMethod: p.protocol === "sftp" ? x.authMethod : "password",
    }));
  }

  async function save(andConnect: boolean) {
    setBusy(true);
    try {
      const input: ProfileInput = {
        ...f,
        name: f.name || (s3 && !f.initialPath ? `${preset.label}` : f.initialPath && s3 ? f.initialPath : f.host),
        port: Number(f.port) || preset.port,
        ...(ssh ? {} : { authMethod: "password" as AuthMethod, sudoMode: "never" as SudoMode, jumpProfileId: null, keyPath: null }),
        initialPath: f.initialPath?.includes("{user}") ? f.initialPath.replace("{user}", f.username) : f.initialPath,
      };
      const p = existing ? await api.updateProfile(existing.id, input) : await api.createProfile(input);
      if (password) await api.setSecret(p.id, ssh && f.authMethod === "key" ? "keyPassphrase" : "password", password);
      if (sudoPassword) await api.setSecret(p.id, "sudoPassword", sudoPassword);
      await loadProfiles();
      close();
      // A connected server keeps its old settings until it reconnects.
      const live = Object.values(sessions).find((x) => x.profile.id === p.id && !x.lost);
      const needs = profileSaved(existing, p, !!password || !!sudoPassword) === "reconnect";
      if (live && needs) {
        if (andConnect) reconnect(live.info.id);
        else setReconnectAsk(live.info.id);
      } else if (andConnect) connect(p.id);
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
      body: "The bookmark and any passwords saved for it will be removed. The activity log is kept.",
      confirmLabel: "Delete server",
      danger: true,
    });
    if (!ok) return;
    try {
      await api.deleteProfile(existing.id);
    } catch (e) {
      toast({ kind: "error", title: "Couldn't delete server", body: asApiError(e).message });
      return;
    }
    await loadProfiles();
    close();
  }

  async function pickKey() {
    const path = await open({ multiple: false, directory: false, title: "Choose a private key", defaultPath: "~/.ssh" });
    if (typeof path === "string") set("keyPath", path);
  }

  const secretLabel = s3 ? "Secret access key" : ssh && f.authMethod === "key" ? "Key passphrase" : "Password";
  const groups = ["Servers", "File servers", "Cloud storage"] as const;
  return (
    <Modal
      title={existing ? "Edit server" : "Add server"}
      subtitle="Passwords, passphrases and keys are saved encrypted on this Mac, and only Orbital can read them."
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
      <label className="field">
        <span>Connection</span>
        <select className="input" value={preset.id} onChange={(e) => pick(e.target.value)}>
          {groups.map((g) => (
            <optgroup key={g} label={g}>
              {PRESETS.filter((p) => p.group === g).map((p) => <option key={p.id} value={p.id}>{p.label}</option>)}
            </optgroup>
          ))}
        </select>
        {ssh ? <span className="hint">The full workbench: files, services, terminal, monitoring and every tool.</span> : <span className="hint">{preset.note ? `${preset.note} ` : ""}Files only: browse, edit, upload and download.</span>}
      </label>
      <div className="grid-host">
        <label className="field">
          <span>{s3 ? "Endpoint" : "Host"}</span>
          <input className="input mono" autoFocus placeholder={preset.hostHint ?? preset.host ?? "203.0.113.10 or web-01.example.com"} value={f.host} onChange={(e) => set("host", e.target.value.trim())} />
        </label>
        <label className="field">
          <span>Port</span>
          <input className="input mono" inputMode="numeric" value={f.port ?? preset.port} onChange={(e) => set("port", Number(e.target.value.replace(/\D/g, "")) || preset.port)} />
        </label>
      </div>
      <div className="grid2">
        <label className="field">
          <span>{s3 ? "Access key ID" : "Username"}</span>
          <input className="input mono" placeholder={s3 ? "AKIA…" : proto === "ftp" || proto === "ftps" ? "anonymous" : "deploy"} value={f.username} onChange={(e) => set("username", e.target.value.trim())} />
        </label>
        <label className="field">
          <span>Name</span>
          <input className="input" placeholder={f.host || "Production web"} value={f.name} onChange={(e) => set("name", e.target.value)} />
        </label>
      </div>

      {!ssh && (
        <label className="field">
          <span>{secretLabel}</span>
          <input
            className="input"
            type="password"
            placeholder={savedSecret ? "Saved — leave empty to keep" : "Ask when connecting"}
            value={password}
            onChange={(e) => setPassword(e.target.value)}
          />
        </label>
      )}
      {s3 && (
        <div className="grid2">
          <label className="field">
            <span>Bucket</span>
            <input className="input mono" placeholder="All buckets" value={f.initialPath ?? ""} onChange={(e) => set("initialPath", e.target.value.trim() || null)} />
            <span className="hint">Needed when the keys may only see one bucket.</span>
          </label>
          <label className="field">
            <span>Region</span>
            <input className="input mono" placeholder="us-east-1" value={f.region ?? ""} onChange={(e) => set("region", e.target.value.trim() || null)} />
            <label className="check" style={{ marginTop: 6 }}><input type="checkbox" checked={!!f.pathStyle} onChange={(e) => set("pathStyle", e.target.checked)} /><span>Path-style addresses (host/bucket)</span></label>
          </label>
        </div>
      )}
      {(dav || proto === "ftp" || proto === "ftps") && (
        <label className="field">
          <span>{dav ? "WebDAV path" : "Open folder"}</span>
          <input className="input mono" placeholder={dav ? preset.path ?? "/ (the server's WebDAV folder)" : "Home folder"} value={f.initialPath ?? ""} onChange={(e) => set("initialPath", e.target.value.trim() || null)} />
          {dav && <span className="hint">Where the server's WebDAV share starts. It is shown as the top folder.</span>}
        </label>
      )}

      {ssh && <>
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
        <p className="note">Uses the keys loaded in your SSH agent (ssh-add, 1Password, Secretive). Nothing is stored by Orbital.</p>
      )}
      {f.authMethod !== "agent" && (
        <label className="field">
          <span>{secretLabel}</span>
          <input
            className="input"
            type="password"
            placeholder={savedSecret ? "Saved — leave empty to keep" : f.authMethod === "key" ? "Leave empty if the key has none" : "Ask when connecting"}
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

      <label className="field">
        <span>Connect through</span>
        <select className="input" value={f.jumpProfileId ?? ""} onChange={(e) => set("jumpProfileId", e.target.value || null)}>
          <option value="">Directly</option>
          {profiles.filter((p) => p.id !== existing?.id && (p.protocol ?? "sftp") === "sftp").map((p) => (
            <option key={p.id} value={p.id}>{p.name} ({p.username}@{p.host})</option>
          ))}
        </select>
        <span className="hint">Jump host (bastion). Each server on the route keeps its own host key check and credentials.</span>
      </label>
      </>}

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
  return { name: "", host: "", port: 22, username: "", authMethod: "agent", keyPath: null, sudoMode: "auto", groupName: null, color: null, initialPath: null, jumpProfileId: null, protocol: "sftp", region: null, pathStyle: false };
}
