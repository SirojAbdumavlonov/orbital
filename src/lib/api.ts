// Typed bridge to the Rust engine. Types mirror outpost-core's serde output.
import { invoke, Channel } from "@tauri-apps/api/core";

export type AuthMethod = "password" | "key" | "agent";
export type SudoMode = "auto" | "never" | "noPassword" | "password";
export type SecretKind = "password" | "keyPassphrase" | "sudoPassword";

export interface Profile {
  id: string;
  name: string;
  host: string;
  port: number;
  username: string;
  authMethod: AuthMethod;
  keyPath: string | null;
  sudoMode: SudoMode;
  groupName: string | null;
  color: string | null;
  initialPath: string | null;
  createdAt: string;
  lastConnectedAt: string | null;
}

export type ProfileInput = Omit<Profile, "id" | "createdAt" | "lastConnectedAt" | "port" | "sudoMode"> & {
  port?: number;
  sudoMode?: SudoMode;
};

export interface SessionInfo {
  id: string;
  profileId: string;
  label: string;
  home: string;
  hasSystemd: boolean;
  dockerVersion: string | null;
  elevation: boolean;
  connectedAt: string;
}

export interface ApiError {
  kind:
    | "Ssh" | "Sftp" | "AuthFailed" | "UnknownHostKey" | "HostKeyMismatch" | "CredentialsRequired"
    | "Secret" | "Db" | "InvalidInput" | "NotFound" | "Remote" | "PermissionDenied" | "Conflict"
    | "ValidationFailed" | "PreflightFailed" | "NotConnected" | "Io" | "Other";
  message: string;
  data: any;
}

export function asApiError(e: unknown): ApiError {
  if (e && typeof e === "object" && "kind" in e && "message" in e) return e as ApiError;
  return { kind: "Other", message: String(e), data: null };
}

export type EntryKind = "file" | "dir" | "symlink" | "other";
export interface FileEntry {
  name: string;
  path: string;
  kind: EntryKind;
  size: number;
  mode: string;
  modeOctal: number;
  mtime: number | null;
  owner: string | null;
  group: string | null;
  uid: number | null;
  gid: number | null;
  hidden: boolean;
}

export interface FileContent {
  path: string;
  content: string;
  size: number;
  mtime: number | null;
  modeOctal: number;
  writable: boolean;
}

export interface Validator {
  name: string;
  command: string;
  stage: "preInstall" | "postInstall" | "local";
}

export interface SafeWriteReport {
  path: string;
  bytes: number;
  privileged: boolean;
  backupPath: string | null;
  validator: string | null;
  validatorOutput: string | null;
  newMtime: number | null;
  suggestedReload: string | null;
}

export interface ExecOutput {
  stdout: string;
  stderr: string;
  exitCode: number;
  durationMs: number;
}

export interface Disk {
  filesystem: string;
  fsType: string;
  mount: string;
  totalBytes: number;
  usedBytes: number;
  availableBytes: number;
  usedPercent: number;
}

export interface Proc {
  pid: number;
  user: string;
  cpuPercent: number;
  memPercent: number;
  rssKb: number;
  command: string;
}

export interface Metrics {
  hostname: string;
  osName: string;
  kernel: string;
  cpuModel: string;
  cores: number;
  load: [number, number, number];
  memTotal: number;
  memAvailable: number;
  memUsed: number;
  memCached: number;
  swapTotal: number;
  swapUsed: number;
  uptimeSecs: number;
  disks: Disk[];
  net: { name: string; rxBytes: number; txBytes: number }[];
  processes: Proc[];
  cpuPercent: number | null;
  perCorePercent: number[];
  netRxPerSec: number | null;
  netTxPerSec: number | null;
  sampledAtMs: number;
}

export interface Service {
  unit: string;
  load: string;
  active: string;
  sub: string;
  description: string;
  unitFileState: string | null;
}

export interface ServiceDetail {
  unit: string;
  properties: Record<string, string>;
  logs: string;
}

export type ServiceAction = "start" | "stop" | "restart" | "reload" | "enable" | "disable";

export interface ActionReport {
  command: string;
  preflight: [string, string] | null;
  output: ExecOutput;
}

export interface Container {
  id: string;
  names: string;
  image: string;
  state: string;
  status: string;
  ports: string;
  createdAt: string;
  composeProject: string | null;
}

export interface ContainerStats {
  id: string;
  name: string;
  cpuPercent: string;
  memUsage: string;
  memPercent: string;
  netIo: string;
  blockIo: string;
  pids: string;
}

export type ContainerAction = "start" | "stop" | "restart" | "pause" | "unpause" | "remove" | "forceRemove";

export type AuditCategory = "connection" | "file" | "config" | "service" | "docker" | "terminal" | "command" | "security";
export interface AuditEvent {
  seq: number;
  id: string;
  ts: string;
  profileId: string | null;
  server: string | null;
  actor: string;
  category: AuditCategory;
  action: string;
  target: string | null;
  command: string | null;
  outcome: "success" | "failure" | "denied";
  exitCode: number | null;
  durationMs: number | null;
  details: any;
  prevHash: string;
  hash: string;
}

export interface KnownHost {
  host: string;
  port: number;
  algorithm: string;
  fingerprint: string;
  addedAt: string;
}

export type TermMessage = { type: "data"; data: string } | { type: "exit"; code: number | null };

export const api = {
  listProfiles: () => invoke<Profile[]>("list_profiles"),
  createProfile: (input: ProfileInput) => invoke<Profile>("create_profile", { input }),
  updateProfile: (id: string, input: ProfileInput) => invoke<Profile>("update_profile", { id, input }),
  deleteProfile: (id: string) => invoke<void>("delete_profile", { id }),
  setSecret: (profileId: string, kind: SecretKind, value: string | null) =>
    invoke<void>("set_secret", { profileId, kind, value }),
  hasSecret: (profileId: string, kind: SecretKind) => invoke<boolean>("has_secret", { profileId, kind }),
  knownHosts: () => invoke<KnownHost[]>("known_hosts"),
  forgetHost: (host: string, port: number) => invoke<void>("forget_host", { host, port }),

  connect: (profileId: string, opts?: { acceptFingerprint?: string; secret?: string; remember?: boolean }) =>
    invoke<SessionInfo>("connect", { profileId, opts }),
  disconnect: (sessionId: string) => invoke<void>("disconnect", { sessionId }),

  listDir: (sessionId: string, path: string) => invoke<FileEntry[]>("list_dir", { sessionId, path }),
  readFile: (sessionId: string, path: string) => invoke<FileContent>("read_file", { sessionId, path }),
  writeFile: (sessionId: string, path: string, content: string, expectedMtime?: number | null) =>
    invoke<number | null>("write_file", { sessionId, path, content, expectedMtime }),
  safeWrite: (a: { sessionId: string; path: string; content: string; validate: boolean; privileged: boolean; expectedMtime?: number | null }) =>
    invoke<SafeWriteReport>("safe_write", a),
  validatorFor: (path: string) => invoke<Validator | null>("validator_for", { path }),
  listBackups: (sessionId: string, path: string, privileged: boolean) =>
    invoke<string[]>("list_backups", { sessionId, path, privileged }),
  mkdir: (sessionId: string, path: string) => invoke<void>("mkdir", { sessionId, path }),
  createFile: (sessionId: string, path: string) => invoke<void>("create_file", { sessionId, path }),
  rename: (sessionId: string, from: string, to: string) => invoke<void>("rename", { sessionId, from, to }),
  remove: (sessionId: string, path: string, recursive: boolean) => invoke<void>("remove", { sessionId, path, recursive }),
  chmod: (sessionId: string, path: string, mode: number) => invoke<void>("chmod", { sessionId, path, mode }),
  upload: (sessionId: string, localPath: string, remotePath: string) => invoke<number>("upload", { sessionId, localPath, remotePath }),
  download: (sessionId: string, remotePath: string, localPath: string) => invoke<number>("download", { sessionId, remotePath, localPath }),

  metrics: (sessionId: string) => invoke<Metrics>("metrics", { sessionId }),
  listServices: (sessionId: string) => invoke<Service[]>("list_services", { sessionId }),
  serviceDetail: (sessionId: string, unit: string) => invoke<ServiceDetail>("service_detail", { sessionId, unit }),
  serviceAction: (sessionId: string, unit: string, action: ServiceAction) =>
    invoke<ActionReport>("service_action", { sessionId, unit, action }),
  listContainers: (sessionId: string) => invoke<Container[]>("list_containers", { sessionId }),
  containerStats: (sessionId: string) => invoke<ContainerStats[]>("container_stats", { sessionId }),
  inspectContainer: (sessionId: string, id: string) => invoke<any>("inspect_container", { sessionId, id }),
  containerLogs: (sessionId: string, id: string, tail = 500) => invoke<string>("container_logs", { sessionId, id, tail }),
  containerAction: (sessionId: string, id: string, action: ContainerAction) =>
    invoke<ExecOutput>("container_action", { sessionId, id, action }),
  runCommand: (sessionId: string, command: string, privileged: boolean) =>
    invoke<ExecOutput>("run_command", { sessionId, command, privileged }),

  openTerminal: (sessionId: string, cols: number, rows: number, cwd: string | null, onEvent: (m: TermMessage) => void) => {
    const ch = new Channel<TermMessage>();
    ch.onmessage = onEvent;
    return invoke<string>("open_terminal", { sessionId, cols, rows, cwd, onEvent: ch });
  },
  terminalWrite: (terminalId: string, data: string) => invoke<void>("terminal_write", { terminalId, data }),
  terminalResize: (terminalId: string, cols: number, rows: number) => invoke<void>("terminal_resize", { terminalId, cols, rows }),
  terminalClose: (terminalId: string) => invoke<void>("terminal_close", { terminalId }),

  auditQuery: (filter: { profileId?: string; category?: AuditCategory; search?: string; limit?: number; beforeSeq?: number }) =>
    invoke<AuditEvent[]>("audit_query", { filter }),
  auditVerify: () => invoke<{ ok: boolean; checked: number; firstBrokenSeq: number | null }>("audit_verify"),
  auditExport: (format: "json" | "csv", path: string) => invoke<void>("audit_export", { format, path }),
};
