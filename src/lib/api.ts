// Typed bridge to the Rust engine. Types mirror orbital-core's serde output.
import { invoke, Channel } from "@tauri-apps/api/core";

export type AuthMethod = "password" | "key" | "agent";
export type SudoMode = "auto" | "never" | "noPassword" | "password";
export type SecretKind = "password" | "keyPassphrase" | "sudoPassword";

/** SFTP is the full SSH workbench; the others are file access only. */
export type Protocol = "sftp" | "ftp" | "ftps" | "webdav" | "webdavs" | "s3";

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
  /** Reach this server through another profile (ProxyJump / bastion). */
  jumpProfileId: string | null;
  protocol: Protocol;
  /** S3 region (e.g. us-east-1, auto for Cloudflare R2). */
  region: string | null;
  /** S3: address buckets as host/bucket instead of bucket.host. */
  pathStyle: boolean;
  createdAt: string;
  lastConnectedAt: string | null;
}

export type ProfileInput = Omit<Profile, "id" | "createdAt" | "lastConnectedAt" | "port" | "sudoMode" | "protocol" | "region" | "pathStyle"> & {
  port?: number;
  sudoMode?: SudoMode;
  protocol?: Protocol;
  region?: string | null;
  pathStyle?: boolean;
};

export interface SessionInfo {
  id: string;
  profileId: string;
  label: string;
  home: string;
  hasSystemd: boolean;
  dockerVersion: string | null;
  elevation: boolean;
  /** Jump hosts the session is tunnelled through, outermost first. */
  via: string[];
  connectedAt: string;
  protocol: Protocol;
}

export interface ConnectOpts {
  acceptFingerprint?: string;
  /** The endpoint the trust dialog showed that fingerprint for. */
  acceptHost?: string;
  acceptPort?: number;
  /** Secret typed for the server being connected to. */
  secret?: string;
  /** Secrets typed for jump hosts on the route, by profile id. */
  hopSecrets?: Record<string, string>;
  remember?: boolean;
}

export interface PresenceStatus {
  available: boolean;
  enabled: boolean;
}

export interface ApiError {
  kind:
    | "Ssh" | "Sftp" | "AuthFailed" | "UnknownHostKey" | "HostKeyMismatch" | "CredentialsRequired"
    | "PresenceRequired" | "Secret" | "Db" | "InvalidInput" | "NotFound" | "Remote" | "PermissionDenied" | "Conflict"
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

export type AuditCategory = "connection" | "file" | "config" | "service" | "docker" | "terminal" | "command" | "security" | "uptime";
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

export interface ToolPath {
  path: string;
  isDir: boolean;
}

/** Well-known server software found on a host. */
export interface DetectedTool {
  id: string;
  name: string;
  category: string;
  running: boolean;
  binary: string | null;
  unit: string | null;
  unitState: string | null;
  paths: ToolPath[];
}

export interface NginxLocation {
  path: string;
  proxyPass: string | null;
  root: string | null;
  line: number;
}

export interface NginxSite {
  file: string;
  line: number;
  serverNames: string[];
  listens: string[];
  ssl: boolean;
  sslCertificate: string | null;
  root: string | null;
  redirect: string | null;
  locations: NginxLocation[];
}

export interface NginxUpstream {
  name: string;
  servers: string[];
  file: string;
  line: number;
}

export interface NginxConfig {
  testOk: boolean;
  testOutput: string;
  files: { path: string; lines: number }[];
  sites: NginxSite[];
  upstreams: NginxUpstream[];
}

export interface TransferReport {
  files: number;
  dirs: number;
  bytes: number;
  skipped: number;
}

// ---------- uptime checks (run from this Mac) ----------

export type CheckKind = "http" | "tcp";
export type CheckState = "unknown" | "up" | "down";

export interface CheckInput {
  name: string;
  kind: CheckKind;
  /** A URL (http) or host:port (tcp). */
  target: string;
  expectedStatus?: number | null;
  contains?: string | null;
  intervalMinutes: 1 | 5 | 15;
  paused?: boolean;
}

export interface Check {
  id: string;
  name: string;
  kind: CheckKind;
  target: string;
  expectedStatus: number;
  contains: string | null;
  intervalMinutes: 1 | 5 | 15;
  paused: boolean;
  createdAt: string;
}

export interface CheckResult {
  at: string;
  up: boolean;
  latencyMs: number | null;
  statusCode: number | null;
  error: string | null;
  certExpires: string | null;
  /** Set when the TLS certificate expires within 14 days. */
  warning: string | null;
}

export interface StateChange {
  at: string;
  up: boolean;
  error: string | null;
}

export interface CheckStatus {
  check: Check;
  state: CheckState;
  last: CheckResult | null;
  /** Latest results, oldest first. */
  recent: CheckResult[];
  uptime24h: number | null;
  uptime7d: number | null;
  lastError: StateChange | null;
  since: string | null;
  /** Newest first. */
  changes: StateChange[];
}

export type UptimeEvent =
  | { type: "result"; checkId: string; result: CheckResult }
  | { type: "changed"; checkId: string; name: string; target: string; change: "wentDown" | "recovered"; error: string | null }
  | { type: "certWarning"; checkId: string; name: string; message: string };

// ---------- port forwarding, live logs, deploys ----------

export interface TunnelInfo {
  id: string;
  sessionId: string;
  localPort: number;
  remoteHost: string;
  remotePort: number;
  openConnections: number;
  totalConnections: number;
  bytes: number;
  label: string | null;
}

export type LogSource = { type: "file"; path: string } | { type: "unit"; unit: string } | { type: "container"; id: string };
export type LogMessage = { type: "data"; text: string } | { type: "end"; code: number | null };

export type RestartKind = "none" | "service" | "compose" | "command";
export interface AppInput { name: string; path: string; branch: string; build: string; restartKind: RestartKind; restartTarget: string | null }
export interface App extends AppInput { id: string; profileId: string; createdAt: string }
export interface Commit { id: string; short: string; author: string; when: string; subject: string }
export interface Checkout { head: string | null; branch: string | null; remote: string | null; dirty: string[]; recent: Commit[] }
export interface DeployRecord {
  id: string;
  appId: string;
  kind: "deploy" | "rollback";
  startedAt: string;
  finishedAt: string | null;
  fromCommit: string | null;
  toCommit: string | null;
  outcome: "running" | "success" | "failure";
  summary: string | null;
}
export type DeployEvent =
  | { type: "step"; name: string }
  | { type: "output"; text: string }
  | { type: "stepEnd"; ok: boolean; code: number | null }
  | { type: "done"; ok: boolean; record: DeployRecord };

// ---------- scheduled jobs ----------

export interface CronJob {
  source: "user" | "system";
  user: string;
  file: string | null;
  line: number;
  schedule: string;
  command: string;
  note: string | null;
  paused: boolean;
  description: string;
  nextRuns: string[];
  lastRun: string | null;
  raw: string;
}

export interface CronReport {
  jobs: CronJob[];
  timers: { unit: string; activates: string; calendar: string | null; next: string | null; last: string | null; description: string | null }[];
  periodic: { period: string; path: string }[];
  serverNow: string | null;
  timezone: string | null;
  complete: boolean;
}

export type CronChange =
  | { type: "add"; schedule: string; command: string; note: string | null }
  | { type: "update"; raw: string; schedule: string; command: string; note: string | null }
  | { type: "delete"; raw: string }
  | { type: "pause"; raw: string }
  | { type: "resume"; raw: string };

export interface CronPreview { description: string; nextRuns: string[]; error: string | null }

// ---------- resource usage ----------

export interface DiskUsage {
  path: string;
  total: number;
  entries: { name: string; path: string; bytes: number; isDir: boolean }[];
  fsSize: number | null;
  fsUsed: number | null;
  fsFree: number | null;
  mount: string | null;
  partial: boolean;
}

export interface UsageGroup { name: string; processes: number; cpu: number; rssBytes: number }
export interface ProcessReport {
  cores: number;
  method: "sampled" | "ps";
  processes: { pid: number; user: string; command: string; cpu: number; memPercent: number; rssBytes: number; unit: string | null }[];
  byProgram: UsageGroup[];
  byService: UsageGroup[];
  byUser: UsageGroup[];
}

// ---------- user accounts ----------

export interface UserAccount {
  name: string;
  uid: number;
  gid: number;
  primaryGroup: string | null;
  groups: string[];
  fullName: string;
  home: string;
  shell: string;
  system: boolean;
  admin: boolean;
  passwordStatus: string | null;
  passwordChanged: string | null;
  sshKeys: number | null;
  canLogin: boolean;
}

export interface UsersReport {
  users: UserAccount[];
  groups: { name: string; gid: number; members: string[] }[];
  shells: string[];
  uidMin: number;
  adminGroup: string | null;
  complete: boolean;
}

export interface NewUser {
  name: string;
  fullName: string | null;
  shell: string | null;
  createHome: boolean;
  home: string | null;
  primaryGroup: string | null;
  groups: string[];
  uid: number | null;
  expires: string | null;
  system: boolean;
  sshKeys: string[];
  mustChangePassword: boolean;
}

export type UserChange =
  | { type: "groups"; groups: string[] }
  | { type: "shell"; shell: string }
  | { type: "fullName"; fullName: string }
  | { type: "lock" }
  | { type: "unlock" }
  | { type: "password"; mustChange: boolean }
  | { type: "addKey"; key: string }
  | { type: "delete"; removeHome: boolean };

// ---------- Apache, HAProxy ----------

export interface ApacheStatus {
  testOk: boolean;
  testOutput: string;
  vhosts: { serverName: string; aliases: string[]; port: number; file: string; line: number }[];
  modules: string[];
  serverRoot: string | null;
  documentRoot: string | null;
  managesSites: boolean;
  sites: { name: string; file: string; enabled: boolean }[];
}

export interface HaServer { name: string; address: string; options: string; status: string | null; sessions: number | null; check: string | null }
export interface HaSection {
  kind: "frontend" | "backend" | "listen";
  name: string;
  line: number;
  binds: string[];
  mode: string | null;
  balance: string | null;
  defaultBackend: string | null;
  useBackends: string[];
  servers: HaServer[];
  status: string | null;
}
export interface HaproxyStatus { configFile: string; testOk: boolean; testOutput: string; sections: HaSection[]; socket: string | null }
export type HaServerState = "ready" | "drain" | "maint";

// ---------- databases ----------

export type DbKind = "postgres" | "mysql" | "mongo";
export type BackupFormat = "native" | "sqlGz";

export interface DbInfo {
  name: string;
  sizeBytes: number | null;
  connections: number | null;
  tables: number | null;
  system: boolean;
}

export interface DbServer {
  version: string | null;
  databases: DbInfo[];
  connections: number | null;
  maxConnections: number | null;
  error: string | null;
}

export interface RedisInfo {
  version: string | null;
  role: string | null;
  uptimeSecs: number | null;
  connectedClients: number | null;
  usedMemory: number | null;
  maxMemory: number | null;
  evictionPolicy: string | null;
  hits: number | null;
  misses: number | null;
  lastSave: number | null;
  lastSaveOk: boolean | null;
  aofEnabled: boolean | null;
  keyspace: { db: number; keys: number; expires: number }[];
  error: string | null;
}

export type RedisAction = "snapshot" | "flushAll" | { flushDb: number };

// ---------- server details ----------

export interface SystemInfo {
  hostname: string | null;
  fqdn: string | null;
  os: string | null;
  kernel: string | null;
  arch: string | null;
  virtualization: string | null;
  vendor: string | null;
  product: string | null;
  cpuThreads: number | null;
  bootTime: string | null;
  timezone: string | null;
  addresses: { interface: string; address: string }[];
  gateway: string | null;
  dns: string[];
  usersLoggedIn: number | null;
  processes: number | null;
  failedUnits: string[];
  updates: number | null;
  securityUpdates: number | null;
  rebootRequired: boolean;
  packageManager: string | null;
  securityModule: string | null;
}

// ---------- docker compose ----------

export type ComposeAction = "up" | "restart" | "pull";

export interface ComposeTarget {
  project: string;
  files: string[];
  workingDir: string | null;
  service: string | null;
}

// ---------- open ports ----------

export interface ListeningPort {
  protocol: "tcp" | "udp";
  address: string;
  port: number;
  /** all interfaces, this server only, or one specific address */
  scope: "all" | "local" | "address";
  process: string | null;
  pid: number | null;
  unit: string | null;
  /** What ufw does with traffic from the internet; null with no active firewall or for local ports. */
  firewall: "open" | "blocked" | "restricted" | "docker" | null;
}

export interface PortsReport {
  ports: ListeningPort[];
  source: string;
  complete: boolean;
  firewallActive: boolean;
}

// ---------- ufw ----------

export interface UfwRule {
  number: number;
  to: string;
  action: "ALLOW" | "DENY" | "REJECT" | "LIMIT";
  direction: "IN" | "OUT" | "FWD";
  from: string;
  v6: boolean;
  comment: string | null;
  raw: string;
}

export interface UfwStatus {
  active: boolean;
  defaultIncoming: string | null;
  defaultOutgoing: string | null;
  logging: string | null;
  rules: UfwRule[];
  added: string[];
  apps: string[];
}

export interface UfwRuleInput {
  action: "allow" | "deny" | "reject" | "limit";
  direction: "in" | "out";
  port: string | null;
  protocol: "any" | "tcp" | "udp";
  address: string | null;
  app: string | null;
  comment: string | null;
}

export type UfwRuleRef = Pick<UfwRule, "number" | "raw">;

// ---------- fail2ban ----------

export interface Jail {
  name: string;
  currentlyFailed: number;
  totalFailed: number;
  currentlyBanned: number;
  totalBanned: number;
  banned: string[];
  watching: string[];
  bantime: string | null;
  findtime: string | null;
  maxretry: string | null;
  ignoreIp: string[];
}

export interface Fail2banStatus {
  version: string | null;
  running: boolean;
  error: string | null;
  jails: Jail[];
  clientIp: string | null;
}

// ---------- certbot ----------

export interface Certificate {
  name: string;
  domains: string[];
  expires: string | null;
  daysLeft: number | null;
  valid: boolean;
  keyType: string | null;
  certificatePath: string | null;
  privateKeyPath: string | null;
  status: string;
}

export interface CertbotStatus {
  version: string | null;
  certificates: Certificate[];
  autoRenew: { via: string; active: boolean; nextRun: string | null } | null;
  plugins: string[];
  error: string | null;
}

export type CertMethod = "nginx" | "apache" | "webroot" | "standalone";

export interface IssueRequest {
  domains: string[];
  email: string | null;
  method: CertMethod;
  webroot: string | null;
  redirect: boolean;
  staging: boolean;
  agreeTos: boolean;
}

export type RenewMode = "dryRun" | "renew" | "force";

// ---------- sshd ----------

export interface SshdCheck {
  id: string;
  title: string;
  level: "bad" | "warn" | "ok" | "info";
  setting: string;
  value: string;
  advice: string;
}

export interface SshdReport {
  version: string | null;
  testOk: boolean;
  testOutput: string;
  readable: boolean;
  checks: SshdCheck[];
  settings: Record<string, string>;
  configFiles: string[];
  sessions: { user: string; tty: string; since: string; from: string | null }[];
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

  presenceStatus: () => invoke<PresenceStatus>("presence_status"),
  setPresenceGate: (enabled: boolean) => invoke<PresenceStatus>("set_presence_gate", { enabled }),

  connect: (profileId: string, opts?: ConnectOpts) => invoke<SessionInfo>("connect", { profileId, opts }),
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

  /** Copy a file or folder from one connected server to another (streamed through this Mac). */
  copyBetween: (a: { srcSessionId: string; srcPath: string; dstSessionId: string; dstPath: string; overwrite: boolean }) =>
    invoke<TransferReport>("copy_between", a),

  detectTools: (sessionId: string) => invoke<DetectedTool[]>("detect_tools", { sessionId }),
  nginxConfig: (sessionId: string) => invoke<NginxConfig>("nginx_config", { sessionId }),
  listeningPorts: (sessionId: string) => invoke<PortsReport>("listening_ports", { sessionId }),
  sshdReport: (sessionId: string) => invoke<SshdReport>("sshd_report", { sessionId }),

  /** Firewall changes are refused (PreflightFailed) when they would block this SSH connection, unless `force`. */
  ufwStatus: (sessionId: string) => invoke<UfwStatus>("ufw_status", { sessionId }),
  ufwAdd: (sessionId: string, rule: UfwRuleInput, force = false) => invoke<UfwStatus>("ufw_add", { sessionId, rule, force }),
  ufwUpdate: (sessionId: string, old: UfwRuleRef[], rule: UfwRuleInput, force = false) =>
    invoke<UfwStatus>("ufw_update", { sessionId, old, rule, force }),
  ufwDelete: (sessionId: string, rules: UfwRuleRef[], force = false) => invoke<UfwStatus>("ufw_delete", { sessionId, rules, force }),
  ufwSetEnabled: (sessionId: string, enabled: boolean, force = false) =>
    invoke<UfwStatus>("ufw_set_enabled", { sessionId, enabled, force }),

  fail2banStatus: (sessionId: string) => invoke<Fail2banStatus>("fail2ban_status", { sessionId }),
  fail2banUnban: (sessionId: string, jail: string | null, ip: string) => invoke<Fail2banStatus>("fail2ban_unban", { sessionId, jail, ip }),
  fail2banBan: (sessionId: string, jail: string, ip: string) => invoke<Fail2banStatus>("fail2ban_ban", { sessionId, jail, ip }),

  certbotStatus: (sessionId: string) => invoke<CertbotStatus>("certbot_status", { sessionId }),
  certbotIssue: (sessionId: string, request: IssueRequest) => invoke<ExecOutput>("certbot_issue", { sessionId, request }),
  certbotRenew: (sessionId: string, name: string | null, mode: RenewMode) => invoke<ExecOutput>("certbot_renew", { sessionId, name, mode }),
  certbotDelete: (sessionId: string, name: string, force = false) => invoke<ExecOutput>("certbot_delete", { sessionId, name, force }),

  /** Forward 127.0.0.1:localPort on this Mac to remoteHost:remotePort as the server sees it. */
  openTunnel: (sessionId: string, remoteHost: string, remotePort: number, localPort: number | null, label: string | null) =>
    invoke<TunnelInfo>("open_tunnel", { sessionId, remoteHost, remotePort, localPort, label }),
  closeTunnel: (id: string) => invoke<void>("close_tunnel", { id }),
  listTunnels: () => invoke<TunnelInfo[]>("list_tunnels"),
  logFiles: (sessionId: string) => invoke<{ path: string; size: number }[]>("log_files", { sessionId }),
  followLogs: (sessionId: string, source: LogSource, lines: number, onEvent: (m: LogMessage) => void) => {
    const ch = new Channel<LogMessage>();
    ch.onmessage = onEvent;
    return invoke<string>("follow_logs", { sessionId, source, lines, onEvent: ch });
  },
  stopLogs: (id: string) => invoke<void>("stop_logs", { id }),
  listApps: (profileId: string) => invoke<App[]>("list_apps", { profileId }),
  saveApp: (profileId: string, id: string | null, input: AppInput) => invoke<App>("save_app", { profileId, id, input }),
  deleteApp: (id: string) => invoke<void>("delete_app", { id }),
  deployHistory: (appId: string) => invoke<DeployRecord[]>("deploy_history", { appId }),
  deployLog: (runId: string) => invoke<string>("deploy_log", { runId }),
  appCheckout: (sessionId: string, appId: string) => invoke<Checkout>("app_checkout", { sessionId, appId }),
  runDeploy: (sessionId: string, appId: string, rollbackTo: string | null, onEvent: (e: DeployEvent) => void) => {
    const ch = new Channel<DeployEvent>();
    ch.onmessage = onEvent;
    return invoke<DeployRecord>("run_deploy", { sessionId, appId, rollbackTo, onEvent: ch });
  },
  cronReport: (sessionId: string) => invoke<CronReport>("cron_report", { sessionId }),
  /** Changes a user's crontab through `crontab -u USER -`; the change names the line it replaces. */
  cronChange: (sessionId: string, user: string, change: CronChange) => invoke<CronReport>("cron_change", { sessionId, user, change }),
  cronRun: (sessionId: string, user: string, command: string) => invoke<ExecOutput>("cron_run", { sessionId, user, command }),
  cronPreview: (schedule: string, serverNow: string) => invoke<CronPreview>("cron_preview", { schedule, serverNow }),
  diskUsage: (sessionId: string, path: string) => invoke<DiskUsage>("disk_usage", { sessionId, path }),
  processUsage: (sessionId: string) => invoke<ProcessReport>("process_usage", { sessionId }),
  listUsers: (sessionId: string) => invoke<UsersReport>("list_users", { sessionId }),
  /** The password, when given, goes to chpasswd over stdin on the server. */
  createUser: (sessionId: string, user: NewUser, password: string | null) => invoke<UsersReport>("create_user", { sessionId, user, password }),
  changeUser: (sessionId: string, name: string, change: UserChange, password: string | null = null, force = false) =>
    invoke<UsersReport>("change_user", { sessionId, name, change, password, force }),
  apacheStatus: (sessionId: string) => invoke<ApacheStatus>("apache_status", { sessionId }),
  /** a2ensite / a2dissite, undone if configtest fails. */
  apacheSite: (sessionId: string, name: string, enable: boolean) => invoke<ApacheStatus>("apache_site", { sessionId, name, enable }),
  haproxyStatus: (sessionId: string) => invoke<HaproxyStatus>("haproxy_status", { sessionId }),
  haproxyServerState: (sessionId: string, backend: string, server: string, state: HaServerState) =>
    invoke<HaproxyStatus>("haproxy_server_state", { sessionId, backend, server, state }),
  listDatabases: (sessionId: string, kind: DbKind) => invoke<DbServer>("list_databases", { sessionId, kind }),
  /** Dump on the server, download to `localPath`, remove the server copy. Returns bytes downloaded. */
  backupDatabase: (sessionId: string, kind: DbKind, name: string, format: BackupFormat, localPath: string) =>
    invoke<number>("backup_database", { sessionId, kind, name, format, localPath }),
  redisInfo: (sessionId: string) => invoke<RedisInfo>("redis_info", { sessionId }),
  redisAction: (sessionId: string, action: RedisAction) => invoke<RedisInfo>("redis_action", { sessionId, action }),
  systemInfo: (sessionId: string) => invoke<SystemInfo>("system_info", { sessionId }),
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
  /** docker compose up -d / restart / pull for a project or one service; returns Compose's output. */
  composeAction: (sessionId: string, target: ComposeTarget, action: ComposeAction) =>
    invoke<ExecOutput>("compose_action", { sessionId, target, action }),
  runCommand: (sessionId: string, command: string, privileged: boolean) =>
    invoke<ExecOutput>("run_command", { sessionId, command, privileged }),

  openTerminal: (sessionId: string, cols: number, rows: number, cwd: string | null, asRoot: boolean, onEvent: (m: TermMessage) => void) => {
    const ch = new Channel<TermMessage>();
    ch.onmessage = onEvent;
    return invoke<string>("open_terminal", { sessionId, cols, rows, cwd, asRoot, onEvent: ch });
  },
  terminalWrite: (terminalId: string, data: string) => invoke<void>("terminal_write", { terminalId, data }),
  terminalResize: (terminalId: string, cols: number, rows: number) => invoke<void>("terminal_resize", { terminalId, cols, rows }),
  terminalClose: (terminalId: string) => invoke<void>("terminal_close", { terminalId }),

  auditQuery: (filter: { profileId?: string; category?: AuditCategory; search?: string; limit?: number; beforeSeq?: number }) =>
    invoke<AuditEvent[]>("audit_query", { filter }),
  auditVerify: () => invoke<{ ok: boolean; checked: number; firstBrokenSeq: number | null }>("audit_verify"),
  auditExport: (format: "json" | "csv", path: string) => invoke<void>("audit_export", { format, path }),
  /** Results and alerts from the uptime checks, for as long as the page lives. */
  uptimeWatch: (onEvent: (e: UptimeEvent) => void) => {
    const ch = new Channel<UptimeEvent>();
    ch.onmessage = onEvent;
    return invoke<void>("uptime_watch", { onEvent: ch });
  },
  listUptime: () => invoke<CheckStatus[]>("list_uptime"),
  saveUptimeCheck: (id: string | null, input: CheckInput) => invoke<Check>("save_uptime_check", { id, input }),
  setUptimePaused: (id: string, paused: boolean) => invoke<Check>("set_uptime_paused", { id, paused }),
  deleteUptimeCheck: (id: string) => invoke<void>("delete_uptime_check", { id }),
  runUptimeCheck: (id: string) => invoke<CheckStatus>("run_uptime_check", { id }),
};
