// Typed bridge to the Rust engine. Types mirror orbital-core's serde output.
import { invoke as tauriInvoke, Channel } from "@tauri-apps/api/core";

/** Told about every call that found its server's connection gone and couldn't reconnect (the store marks it lost). */
let onNotConnected: ((sessionId: string) => void) | null = null;
export function setNotConnectedHandler(f: (sessionId: string) => void) {
  onNotConnected = f;
}

/**
 * Reconnects a server quietly (the store's `reconnect` with `quiet`): the new
 * session id, or null when it can't (no network, a password nobody saved, …).
 */
let reconnector: ((sessionId: string) => Promise<string | null>) | null = null;
export function setReconnector(f: (sessionId: string) => Promise<string | null>) {
  reconnector = f;
}

/** One reconnect per lost session, shared by every call that finds it gone at the same time. */
const reconnecting = new Map<string, Promise<string | null>>();
/** Old session id → its replacement, for calls that still use the old one. */
const replacedBy = new Map<string, string>();

/** Calls that are never repeated on a new connection: connecting itself, and terminals (they died with it). */
export const NO_RETRY = /^(connect|disconnect|terminal_|open_terminal|close_terminal|resize_terminal|write_terminal|tmux_)/;

/** The same arguments, pointing at the new session. */
export function withSession(args: Record<string, unknown>, from: string, to: string): Record<string, unknown> {
  const out: Record<string, unknown> = { ...args };
  for (const k of ["sessionId", "srcSessionId", "dstSessionId"]) if (out[k] === from) out[k] = to;
  return out;
}

const sessionOf = (args?: Record<string, unknown>) => {
  const sid = args && (args.sessionId ?? args.srcSessionId ?? args.dstSessionId);
  return typeof sid === "string" ? sid : null;
};
const isNotConnected = (e: unknown) => !!e && typeof e === "object" && (e as { kind?: string }).kind === "NotConnected";

/**
 * Every backend call. When it finds its server's connection gone, Orbital
 * reconnects in the background (a few tries) and makes the call once more on
 * the new connection, so a dropped connection goes unnoticed. Only when that
 * fails is the server marked as disconnected (its header offers Reconnect).
 */
async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  // A call made with the id of a session that was already replaced goes to its replacement.
  let sid = sessionOf(args);
  if (sid && replacedBy.has(sid) && args) {
    let to = sid;
    while (replacedBy.has(to)) to = replacedBy.get(to)!;
    args = withSession(args, sid, to);
    sid = to;
  }
  try {
    return await tauriInvoke<T>(cmd, args);
  } catch (e) {
    if (!isNotConnected(e) || !sid || !args) throw e;
    if (!reconnector || NO_RETRY.test(cmd)) {
      onNotConnected?.(sid);
      throw e;
    }
    let job = reconnecting.get(sid);
    if (!job) {
      job = reconnector(sid).finally(() => reconnecting.delete(sid!));
      reconnecting.set(sid, job);
    }
    const fresh = await job.catch(() => null);
    if (!fresh) {
      onNotConnected?.(sid);
      throw e;
    }
    replacedBy.set(sid, fresh);
    return await tauriInvoke<T>(cmd, withSession(args, sid, fresh));
  }
}

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

/** One host from ~/.ssh/config, as it would be saved (ssh_config::Candidate). */
export interface SshImportCandidate {
  key: string;
  name: string;
  aliases: string[];
  host: string;
  port: number;
  username: string;
  authMethod: AuthMethod;
  keyPath: string | null;
  /** Key of the candidate this one connects through. */
  jumpKey: string | null;
  /** Only here because another host jumps through it. */
  jumpOnly: boolean;
  importable: boolean;
  source: string | null;
  notes: SshImportNote[];
  /** A saved profile with the same host, port and user. */
  existingId: string | null;
  existingName: string | null;
}
export interface SshImportNote { level: "info" | "warn"; text: string }
export interface SshImportPreview { path: string; found: boolean; candidates: SshImportCandidate[]; notes: SshImportNote[] }
export interface SshImportReport { created: Profile[]; failed: { name: string; error: string }[] }

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
    | "ValidationFailed" | "PreflightFailed" | "NotConnected" | "Io" | "Cancelled" | "Other";
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

/** Where the memory goes (bytes); used + buffCache + free = memTotal, like `free`. */
export interface MemoryDetail {
  /** RAM in the machine, when the server's /sys says. */
  installed: number | null;
  used: number;
  buffCache: number;
  free: number;
  buffers: number;
  pageCache: number;
  anon: number;
  shared: number;
  slabReclaimable: number;
  slabUnreclaimable: number;
  kernelStack: number;
  pageTables: number;
  dirty: number;
  committed: number;
  commitLimit: number;
  active: number;
  inactive: number;
  swapCached: number;
  hugePages: number;
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
  memory: MemoryDetail;
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

export type AuditCategory = "connection" | "file" | "config" | "service" | "docker" | "terminal" | "command" | "security" | "dns" | "uptime";
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
  /** Installed on the server itself (binary, unit or a host process). */
  onHost: boolean;
  /** Running containers whose image is this program. */
  containers: string[];
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
  /** Why the server refused the last connection (null while it works). */
  lastError: string | null;
}

export interface SiteProbe { url: string; status: number | null; error: string | null }
export interface SiteTest { configOk: boolean; configOutput: string; server: SiteProbe; internet: SiteProbe }

export interface InstallOption { id: string; name: string; category: string; hint: string; apt: string[]; dnf: string[] }

export type MoveMode = "copy" | "move";
export interface MountCopy { source: string; volume: string | null; dest: string }
export interface PortMap { hostIp: string; hostPort: number | null; container: string }
export interface EnvVar { key: string; value: string | null; secret: boolean }
export interface MovePlan { name: string; image: string; imageId: string; repoDigest: string | null; running: boolean; runArgs: string[]; envCount: number; hostPorts: number[]; copies: MountCopy[]; network: string | null; notes: string[]; ports: PortMap[]; args: string[]; envVars: EnvVar[]; hints: string[] }
/** Changes made in the Move dialog. */
export interface MoveOptions { ports?: PortMap[]; args?: string[]; envSet?: Record<string, string>; envRemove?: string[] }
export interface MovePreview { plan: MovePlan; clashes: { noDocker: boolean; name: boolean; ports: number[]; paths: string[] }; target: string }

// ---------- sign-ins and log search ----------
export interface LoginSession { user: string; tty: string; from: string; when: string }
export interface Count { key: string; count: number }
export interface LoginsReport { now: LoginSession[]; recent: LoginSession[]; failed: number; topAddresses: Count[]; topUsers: Count[]; source: string }
export type SearchSource = "journal" | "files";
export interface LogSearch { hits: { file: string; line: string }[]; truncated: boolean; total: number }

// ---------- Docker Compose stacks ----------
export interface Stack { name: string; status: string; files: string[]; running: boolean; ours: boolean }
export type StackAction = "up" | "pullUp" | "restart" | "down" | "logs";
/** A ready-made app (apps.rs): installed as a stack on 127.0.0.1:port. */
export interface AppOption { id: string; name: string; category: string; hint: string; port: number; firstSteps: string; memoryMb: number; images: string[] }
export interface AppInstalled { name: string; app: string; port: number; started: boolean; output: string; shown: { label: string; value: string }[]; firstSteps: string }

// ---------- services made by Orbital ----------
export type RestartPolicy = "always" | "onFailure" | "no";
export interface ServiceSpec { name: string; description: string; command: string; workdir: string | null; user: string | null; restart: RestartPolicy; env: [string, string][] }

// ---------- scheduled backups ----------
export type BackupSource = { kind: "folder"; path: string } | { kind: "database"; engine: DbKind; name: string; container?: string | null };
export interface BackupSchedule { every: "day" | "week"; weekday: number; hour: number; minute: number }
export interface BackupJob { id: string; name: string; source: BackupSource; schedule: BackupSchedule; keep: number }
export interface BackupFile { path: string; bytes: number; at: number }
export interface BackupStatus { job: BackupJob; files: BackupFile[]; lastRun: number | null; lastError: string | null; scheduled: boolean }
export type RestoreTo = "copy" | "original";

// ---------- nginx routes ----------
export type RouteTarget =
  | { kind: "app"; host: string; port: number; keepPrefix: boolean; websockets: boolean }
  | { kind: "folder"; dir: string }
  | { kind: "redirect"; to: string; code: number };
export interface Route { path: string; exact: boolean; target: RouteTarget }
export type RouteChange = { op: "add"; route: Route } | { op: "replace"; spec: string; route: Route } | { op: "remove"; spec: string };
export interface RouteInfo { spec: string; line: number; route: Route | null; summary: string; text: string }
export interface RoutesView { file: string; line: number; routes: RouteInfo[] }
export interface RouteChangeReport { file: string; before: string; after: string; applied: boolean; backupPath: string | null; reloaded: boolean; reloadOutput: string }

export type FeedbackKind = "problem" | "idea" | "praise" | "other";
export interface FeedbackInput { kind: FeedbackKind; about: string; message: string; contact: string }

export interface StatsState { available: boolean; enabled: boolean | null }

export type NetCheck = "ping" | "dns" | "port" | "http" | "trace";
export interface NetResult { ok: boolean; summary: string; output: string; command: string }

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

// ---------- new website ----------

export type SiteKind = { kind: "static"; root: string } | { kind: "proxy"; port: number };
export interface WebsiteInput { domain: string; includeWww: boolean; site: SiteKind }
export type SiteLayout = "sitesAvailable" | "confD";
export interface WebsitePlan {
  domain: string;
  names: string[];
  layout: SiteLayout;
  file: string;
  enabledLink: string | null;
  root: string | null;
  rootExists: boolean;
  content: string;
  /** Why it can't be created as it stands. Empty: good to go. */
  blockers: string[];
  /** nginx is broken by its default site pointing at deleted certificates; turning it off repairs it. */
  defaultOffFixes: boolean;
  defaultMissing: string[];
  /** Enabled files that break nginx (deleted certificate, dangling link). */
  brokenSites: BrokenSite[];
}
export interface BrokenSite { file: string; link: boolean; missing: string[]; dangling: boolean; stock: boolean }
export interface WebsiteReport { plan: WebsitePlan; write: SafeWriteReport; rootCreated: boolean; reloaded: boolean; reloadOutput: string }

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
  /** Another certbot run holds the lock; the list is unavailable for now. */
  busy: boolean;
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

// ---------- processes, disks, packages, TLS inspector, timers ----------

export interface Proc { pid: number; ppid: number; user: string; nice: number; state: string; cpu: number; mem: number; rssBytes: number; elapsedSecs: number; name: string; args: string }
export interface ProcDetail { pid: number; threads: number | null; swapBytes: number | null; openFiles: number | null; cwd: string | null; exe: string | null; unit: string | null; listening: string[]; maxFiles: string | null }
export type ProcSignal = "TERM" | "KILL" | "HUP" | "INT" | "STOP" | "CONT" | "USR1" | "USR2";
export interface BlockDevice { name: string; kind: string; size: number; fstype: string | null; mountpoint: string | null; model: string | null; rotational: boolean; parent: string | null; healthy: boolean | null }
export interface DiskFilesystem { source: string; fstype: string; mountpoint: string; size: number; used: number; inodes: number | null; inodesUsed: number | null }
export interface FstabEntry { source: string; mountpoint: string; fstype: string; options: string; mounted: boolean }
export interface DiskReport { devices: BlockDevice[]; filesystems: DiskFilesystem[]; fstab: FstabEntry[]; smartMissing: boolean; warnings: string[] }
export interface Package { name: string; version: string; size: number | null; summary: string }
export interface FoundPackage { name: string; summary: string; installed: boolean }
export type PkgAction = "install" | "remove";
export interface PkgPlan { install: string[]; remove: string[]; text: string; essential: string[] }
export interface CertInfo { subject: string; commonName: string | null; issuer: string; names: string[]; notBefore: number; notAfter: number; daysLeft: number; serial: string; key: string; selfSigned: boolean }
export interface TlsReport { host: string; port: number; address: string | null; trusted: boolean; problem: string | null; protocol: string | null; cipher: string | null; chain: CertInfo[] }
export interface TimerSpec { name: string; description: string; command: string; user: string | null; schedule: string; persistent: boolean }
export interface ScheduleCheck { normalized: string; next: string }

// ---------- kernel tuning, SSH hardening, security audit, SQL console, recordings ----------

export interface KernelSetting {
  key: string; group: string; title: string; why: string; recommended: string;
  cmp: "eq" | "atLeast" | "atMost"; onlyWith: string | null;
  /** What the kernel uses now; null when this kernel doesn't have it. */
  current: string | null;
  /** The value in Orbital's file, if it sets this key. */
  ours: string | null;
  ok: boolean;
}
export interface KernelReport { settings: KernelSetting[]; custom: Record<string, string>; customCurrent: Record<string, string>; fileExists: boolean; file: string }
export interface HardenState { includesDropins: boolean; current: Record<string, string>; dropin: string }
export interface HardenResult { state: HardenState; notInEffect: [string, string][] }
export type AuditLevel = "bad" | "warn" | "info" | "ok";
export interface AuditFinding { id: string; level: AuditLevel; title: string; detail: string; items: string[] }
export interface AuditReport { findings: AuditFinding[]; partial: boolean }
export interface SqlResult { columns: string[]; rows: (string | null)[][]; text: string | null; error: string | null; truncated: boolean; durationMs: number }
export interface RecordingInfo { name: string; title: string; started: number; seconds: number; bytes: number; width: number; height: number }

// ---------- snippets ----------

export interface Snippet { id: string; name: string; command: string; description: string | null; sudo: boolean; params: string[]; createdAt: string; updatedAt: string }
export interface SnippetInput { name: string; command: string; description: string | null; sudo: boolean }
export interface SnippetRun { sessionId: string; server: string; command: string; output: ExecOutput | null; error: string | null; /** Refused as the login user; sudo is allowed there. */ needsSudo: boolean }

// ---------- SSH keys ----------

export interface LocalKey { name: string; path: string; algorithm: string; comment: string; fingerprint: string; publicLine: string; encrypted: boolean }
export interface AuthorizedKey { line: string; options: string | null; algorithm: string; comment: string; fingerprint: string | null; onThisComputer: boolean }
export interface AuthorizedKeys { user: string; path: string; keys: AuthorizedKey[]; missing: boolean }

// ---------- DNS ----------

export type DnsProvider = "cloudflare";
export type DnsType = "A" | "AAAA" | "CNAME" | "TXT" | "MX";

export interface DnsAccount { id: string; name: string; kind: DnsProvider; createdAt: string }
export interface DnsZone { id: string; name: string; status: string; nameServers: string[] }
export interface DnsRecord {
  id: string;
  name: string;
  /** May be a type Orbital doesn't create (NS, SRV, CAA, …). */
  type: string;
  content: string;
  /** Seconds; 1 = automatic. */
  ttl: number;
  proxied: boolean;
  proxiable: boolean;
  priority: number | null;
  comment: string | null;
}
export interface DnsRecordInput { type: DnsType; name: string; content: string; ttl: number; proxied: boolean; priority: number | null }
export interface ResolverAnswer { resolver: string; status: string; values: string[]; ok: boolean; error: string | null }
export interface DnsCheck { name: string; type: DnsType; expected: string[]; answers: ResolverAnswer[]; verified: boolean; checkedAt: string }

export type TermMessage = { type: "data"; data: string } | { type: "exit"; code: number | null };

// ---------- system updates ----------

export interface Upgradable {
  name: string;
  current: string | null;
  available: string;
  security: boolean;
}

export interface UpdatesReport {
  manager: "apt" | "dnf" | "yum" | null;
  packages: Upgradable[];
  rebootRequired: boolean;
  /** Unix seconds of the last package-list refresh, when known. */
  refreshedAt: number | null;
}

// ---------- AI agent (Telegram bot on the server) ----------

export type AiProvider = "claude" | "openai";

/** Typed once; the token and key go to the server only (root-only file), never kept in Orbital. */
export interface AgentSetup {
  telegramToken: string;
  provider: AiProvider;
  apiKey: string;
  model: string | null;
  alerts: boolean;
}

export interface AgentStatus {
  installed: boolean;
  running: boolean;
  version: string | null;
  provider: string | null;
  model: string | null;
  bot: string | null;
  alerts: boolean;
  pairCodeActive: boolean;
  owner: string | null;
  pairedAt: number | null;
  alerting: string[];
  /** JSON lines from the agent's activity log. */
  recent: string[];
  updateAvailable: boolean;
  /** Time of the daily summary ("09:00", server time), or null when off. */
  dailyReport: string | null;
  /** The agent version this Orbital installs. */
  latestVersion: string;
}

export interface AiModel {
  id: string;
  name: string;
}

/** What Check found: the bot's @name and the models the key can use (newest first). */
export interface AgentCheck {
  bot: string;
  models: AiModel[];
}

export interface PairCode {
  code: string;
  expires: number;
  bot: string | null;
}

// ---------- find files ----------

export interface FoundFile {
  path: string;
  dir: boolean;
}

export interface FindReport {
  files: FoundFile[];
  /** More matched than were returned (200). */
  truncated: boolean;
}

// ---------- disk cleanup ----------

export interface CleanupItem {
  id: string;
  title: string;
  detail: string;
  bytes: number | null;
}

// ---------- terminals: what they run ----------

/** A login shell, a tmux session (created or reattached by name), or a shell inside a Docker container. */
export type TermLaunch = { kind: "shell" } | { kind: "tmux"; session: string } | { kind: "container"; id: string };

export interface TmuxSession {
  name: string;
  windows: number;
  attached: boolean;
  created: number | null;
}

// ---------- job progress (orbital_core::progress) ----------

/** What a long operation reports while it runs. */
export type ProgressEvent =
  | { type: "step"; text: string }
  | { type: "line"; text: string }
  | { type: "transfer"; files: number; bytes: number; current: string };
/** A progress listener. The channel's id is written onto it: it is the job's id, which `cancelJob` stops. */
export type OnProgress = ((e: ProgressEvent) => void) & { jobId?: number };

/** Long commands always take a progress channel; it is only listened to when someone wants it. */
function progressChannel(onProgress?: OnProgress) {
  const ch = new Channel<ProgressEvent>();
  if (onProgress) {
    ch.onmessage = onProgress;
    onProgress.jobId = ch.id;
  }
  return ch;
}

export const api = {
  /** Files and folders whose names contain `query`, below `dir` (skips .git and node_modules). */
  findFiles: (sessionId: string, dir: string, query: string) => invoke<FindReport>("find_files", { sessionId, dir, query }),
  /** A folder as one .tar.gz, streamed into `localPath`; a job (progress, stoppable). */
  dockerVersion: (sessionId: string) => invoke<string | null>("docker_version", { sessionId }),
  /** nginx -t, a request on the server, and one from this computer. */
  websiteTest: (sessionId: string, domain: string, https: boolean) => invoke<SiteTest>("website_test", { sessionId, domain, https }),
  /** Remove a site's nginx file (backed up; put back if nginx -t fails), optionally its certificate and folder; a job. Returns the backup path. */
  removeWebsite: (sessionId: string, file: string, certificate: string | null, root: string | null, onProgress?: OnProgress) =>
    invoke<string>("remove_website", { sessionId, file, certificate, root, progress: progressChannel(onProgress) }),
  installOptions: () => invoke<InstallOption[]>("install_options"),
  /** Install supported software from the server's package repositories and start it; a job. */
  installSoftware: (sessionId: string, id: string, force: boolean, onProgress?: OnProgress) =>
    invoke<ExecOutput>("install_software", { sessionId, id, force, progress: progressChannel(onProgress) }),
  /** What moving a container to `to` would do, and what's in the way there. */
  movePreview: (sessionId: string, id: string, to: string, options: MoveOptions | null = null) => invoke<MovePreview>("move_preview", { sessionId, id, to, options }),
  /** Recreate a container on another server (image, volumes, files, network, options); a job. */
  moveContainer: (sessionId: string, id: string, to: string, mode: MoveMode, force: boolean, options: MoveOptions | null, onProgress?: OnProgress) =>
    invoke<MovePlan>("move_container", { sessionId, id, to, mode, force, options, progress: progressChannel(onProgress) }),
  downloadFolder: (sessionId: string, remotePath: string, localPath: string, onProgress?: OnProgress) =>
    invoke<number>("download_folder", { sessionId, remotePath, localPath, progress: progressChannel(onProgress) }),
  agentStatus: (sessionId: string) => invoke<AgentStatus>("agent_status", { sessionId }),
  /** Checks the bot token with Telegram and the AI key with its provider; returns the bot's @name. */
  agentCheck: (setup: AgentSetup) => invoke<AgentCheck>("agent_check", { setup }),
  agentInstall: (sessionId: string, setup: AgentSetup, onProgress?: OnProgress) => invoke<PairCode>("agent_install", { sessionId, setup, progress: progressChannel(onProgress) }),
  agentPairCode: (sessionId: string) => invoke<PairCode>("agent_pair_code", { sessionId }),
  agentUpdate: (sessionId: string, onProgress?: OnProgress) => invoke<void>("agent_update", { sessionId, progress: progressChannel(onProgress) }),
  agentSetRunning: (sessionId: string, on: boolean) => invoke<void>("agent_set_running", { sessionId, on }),
  agentSetAlerts: (sessionId: string, on: boolean) => invoke<void>("agent_set_alerts", { sessionId, on }),
  agentRemove: (sessionId: string) => invoke<void>("agent_remove", { sessionId }),
  /** The models the agent's key can use (listed by the agent on the server; Orbital doesn't keep the key). */
  agentModels: (sessionId: string) => invoke<AiModel[]>("agent_models", { sessionId }),
  /** Switch the agent's model; it applies from the next message. */
  agentSetModel: (sessionId: string, model: string) => invoke<void>("agent_set_model", { sessionId, model }),
  /** The agent messages the paired account; returns who it reached. */
  agentTest: (sessionId: string) => invoke<string | null>("agent_test", { sessionId }),
  /** Daily summary at "HH:MM" (server time), or null for off. */
  agentSetDaily: (sessionId: string, time: string | null) => invoke<void>("agent_set_daily", { sessionId, time }),
  cleanupScan: (sessionId: string) => invoke<CleanupItem[]>("cleanup_scan", { sessionId }),
  cleanupRun: (sessionId: string, id: string, onProgress?: OnProgress) => invoke<CleanupItem[]>("cleanup_run", { sessionId, id, progress: progressChannel(onProgress) }),
  /** SIGTERM (or SIGKILL with force). PID 1 and sshd are refused. */
  stopProcess: (sessionId: string, pid: number, force: boolean) => invoke<void>("stop_process", { sessionId, pid, force }),
  updates: (sessionId: string) => invoke<UpdatesReport>("updates", { sessionId }),
  refreshUpdates: (sessionId: string, onProgress?: OnProgress) => invoke<UpdatesReport>("refresh_updates", { sessionId, progress: progressChannel(onProgress) }),
  installUpdates: (sessionId: string, onProgress?: OnProgress) => invoke<ExecOutput>("install_updates", { sessionId, progress: progressChannel(onProgress) }),
  /** Reboots the server; the connection drops a few seconds later. */
  rebootServer: (sessionId: string) => invoke<void>("reboot_server", { sessionId }),
  /** Stop a running job (see OnProgress.jobId). False when it already finished. */
  cancelJob: (id: number) => invoke<boolean>("cancel_job", { id }),
  /** A system notification (for jobs that finish while Orbital is in the background). */
  notify: (title: string, body: string) => invoke<void>("notify", { title, body }),
  listProfiles: () => invoke<Profile[]>("list_profiles"),
  createProfile: (input: ProfileInput) => invoke<Profile>("create_profile", { input }),
  updateProfile: (id: string, input: ProfileInput) => invoke<Profile>("update_profile", { id, input }),
  deleteProfile: (id: string) => invoke<void>("delete_profile", { id }),
  /** Hosts in `path` (`~/…` works), or in ~/.ssh/config when it's null. */
  sshConfigPreview: (path: string | null = null) => invoke<SshImportPreview>("ssh_config_preview", { path }),
  importSshConfig: (path: string | null, keys: string[]) => invoke<SshImportReport>("import_ssh_config", { path, keys }),
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
  readBackup: (sessionId: string, path: string, backup: string, privileged: boolean) => invoke<string>("read_backup", { sessionId, path, backup, privileged }),
  /** Put a backup back through safe save (validated; the current version is kept as a backup). */
  restoreBackup: (sessionId: string, path: string, backup: string, privileged: boolean) => invoke<SafeWriteReport>("restore_backup", { sessionId, path, backup, privileged }),
  listBackups: (sessionId: string, path: string, privileged: boolean) =>
    invoke<string[]>("list_backups", { sessionId, path, privileged }),
  mkdir: (sessionId: string, path: string) => invoke<void>("mkdir", { sessionId, path }),
  createFile: (sessionId: string, path: string) => invoke<void>("create_file", { sessionId, path }),
  rename: (sessionId: string, from: string, to: string) => invoke<void>("rename", { sessionId, from, to }),
  remove: (sessionId: string, path: string, recursive: boolean) => invoke<void>("remove", { sessionId, path, recursive }),
  chmod: (sessionId: string, path: string, mode: number) => invoke<void>("chmod", { sessionId, path, mode }),
  upload: (sessionId: string, localPath: string, remotePath: string, onProgress?: OnProgress) =>
    invoke<number>("upload", { sessionId, localPath, remotePath, progress: progressChannel(onProgress) }),
  download: (sessionId: string, remotePath: string, localPath: string) => invoke<number>("download", { sessionId, remotePath, localPath }),

  /** Enabled nginx files that break the configuration (read-only). */
  nginxBrokenSites: (sessionId: string) => invoke<BrokenSite[]>("nginx_broken_sites", { sessionId }),
  /** Turn a site off by removing its sites-enabled link (its file is kept); put back if nginx still fails. */
  disableSite: (sessionId: string, link: string, onProgress?: OnProgress) => invoke<string>("disable_site", { sessionId, link, progress: progressChannel(onProgress) }),
  /** Turn nginx's default site off by its sites-enabled link (the file isn't touched); put back if nginx still fails. */
  disableDefaultSite: (sessionId: string, onProgress?: OnProgress) => invoke<string>("disable_default_site", { sessionId, progress: progressChannel(onProgress) }),
  /** Paste on the same server: copy a file or folder to another place there (cp, sudo when refused). */
  copyWithin: (sessionId: string, from: string, to: string, onProgress?: OnProgress) =>
    invoke<void>("copy_within", { sessionId, from, to, progress: progressChannel(onProgress) }),
  /** What this computer's clipboard holds: copied files, text, or an image saved as a temporary PNG. */
  clipboardRead: () => invoke<{ files: string[]; text: string | null; image: string | null; native: boolean }>("clipboard_read"),

  /** Copy a file or folder from one connected server to another (streamed through this computer). */
  copyBetween: (a: { srcSessionId: string; srcPath: string; dstSessionId: string; dstPath: string; overwrite: boolean }, onProgress?: OnProgress) =>
    invoke<TransferReport>("copy_between", { ...a, progress: progressChannel(onProgress) }),

  detectTools: (sessionId: string) => invoke<DetectedTool[]>("detect_tools", { sessionId }),
  nginxConfig: (sessionId: string) => invoke<NginxConfig>("nginx_config", { sessionId }),
  websitePlan: (sessionId: string, input: WebsiteInput) => invoke<WebsitePlan>("website_plan", { sessionId, input }),
  createWebsite: (sessionId: string, input: WebsiteInput, onProgress?: OnProgress) =>
    invoke<WebsiteReport>("create_website", { sessionId, input, progress: progressChannel(onProgress) }),
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
  certbotIssue: (sessionId: string, request: IssueRequest, onProgress?: OnProgress) =>
    invoke<ExecOutput>("certbot_issue", { sessionId, request, progress: progressChannel(onProgress) }),
  certbotRenew: (sessionId: string, name: string | null, mode: RenewMode, onProgress?: OnProgress) =>
    invoke<ExecOutput>("certbot_renew", { sessionId, name, mode, progress: progressChannel(onProgress) }),
  certbotDelete: (sessionId: string, name: string, force = false, onProgress?: OnProgress) =>
    invoke<ExecOutput>("certbot_delete", { sessionId, name, force, progress: progressChannel(onProgress) }),

  /** Forward 127.0.0.1:localPort on this computer to remoteHost:remotePort as the server sees it. */
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
  apacheStatus: (sessionId: string, container: string | null = null) => invoke<ApacheStatus>("apache_status", { sessionId, container }),
  /** a2ensite / a2dissite, undone if configtest fails. */
  apacheSite: (sessionId: string, name: string, enable: boolean) => invoke<ApacheStatus>("apache_site", { sessionId, name, enable }),
  haproxyStatus: (sessionId: string) => invoke<HaproxyStatus>("haproxy_status", { sessionId }),
  haproxyServerState: (sessionId: string, backend: string, server: string, state: HaServerState) =>
    invoke<HaproxyStatus>("haproxy_server_state", { sessionId, backend, server, state }),
  listDatabases: (sessionId: string, kind: DbKind, container: string | null = null) => invoke<DbServer>("list_databases", { sessionId, kind, container }),
  /** Dump on the server, download to `localPath`, remove the server copy. Returns bytes downloaded. */
  backupDatabase: (sessionId: string, kind: DbKind, name: string, format: BackupFormat, localPath: string, onProgress?: OnProgress) =>
    invoke<number>("backup_database", { sessionId, kind, name, format, localPath, progress: progressChannel(onProgress) }),
  redisInfo: (sessionId: string, container: string | null = null) => invoke<RedisInfo>("redis_info", { sessionId, container }),
  redisAction: (sessionId: string, action: RedisAction, container: string | null = null) => invoke<RedisInfo>("redis_action", { sessionId, action, container }),
  systemInfo: (sessionId: string) => invoke<SystemInfo>("system_info", { sessionId }),
  metrics: (sessionId: string) => invoke<Metrics>("metrics", { sessionId }),
  stacks: (sessionId: string) => invoke<Stack[]>("stacks", { sessionId }),
  logins: (sessionId: string) => invoke<LoginsReport>("logins", { sessionId }),
  searchLogs: (sessionId: string, term: string, hours: number, source: SearchSource) => invoke<LogSearch>("search_logs", { sessionId, term, hours, source }),
  /** A new stack in /opt/stacks/<name>/compose.yaml (checked by docker compose config); deployed with `deploy`. */
  createStack: (sessionId: string, name: string, content: string, deploy: boolean, onProgress?: OnProgress) =>
    invoke<ExecOutput | null>("create_stack", { sessionId, name, content, deploy, progress: progressChannel(onProgress) }),
  appOptions: () => invoke<AppOption[]>("app_options"),
  /** Install a ready-made app as stack `name` on 127.0.0.1:`port`; generated passwords come back once in `shown`. */
  installApp: (sessionId: string, app: string, name: string, port: number, force: boolean, onProgress?: OnProgress) =>
    invoke<AppInstalled>("install_app", { sessionId, app, name, port, force, progress: progressChannel(onProgress) }),
  stackAction: (sessionId: string, name: string, file: string, action: StackAction, onProgress?: OnProgress) =>
    invoke<ExecOutput>("stack_action", { sessionId, name, file, action, progress: progressChannel(onProgress) }),
  /** Add a swap file of this many MB (refused when swap is already on or the disk is short). Returns the swap size in bytes. */
  addSwap: (sessionId: string, mb: number, onProgress?: OnProgress) => invoke<number>("add_swap", { sessionId, mb, progress: progressChannel(onProgress) }),
  /** Turn a command into a running systemd service; returns [running, systemctl output]. */
  createService: (sessionId: string, spec: ServiceSpec, onProgress?: OnProgress) => invoke<[boolean, string]>("create_service", { sessionId, spec, progress: progressChannel(onProgress) }),
  removeService: (sessionId: string, name: string) => invoke<void>("remove_service", { sessionId, name }),
  backupJobs: (sessionId: string) => invoke<BackupStatus[]>("backup_jobs", { sessionId }),
  saveBackupJob: (sessionId: string, job: BackupJob, onProgress?: OnProgress) => invoke<BackupJob>("save_backup_job", { sessionId, job, progress: progressChannel(onProgress) }),
  deleteBackupJob: (sessionId: string, id: string, files: boolean) => invoke<void>("delete_backup_job", { sessionId, id, files }),
  runBackup: (sessionId: string, id: string, onProgress?: OnProgress) => invoke<string>("run_backup", { sessionId, id, progress: progressChannel(onProgress) }),
  restoreScheduledBackup: (sessionId: string, id: string, file: string, to: RestoreTo, onProgress?: OnProgress) =>
    invoke<string>("restore_scheduled_backup", { sessionId, id, file, to, progress: progressChannel(onProgress) }),
  /** A server block's routes (location blocks), from its real file. */
  nginxRoutes: (sessionId: string, file: string, line: number) => invoke<RoutesView>("nginx_routes", { sessionId, file, line }),
  /** Preview (apply false) or make a route change: safe save with nginx -t, then reload. */
  nginxRouteChange: (sessionId: string, file: string, line: number, change: RouteChange, apply: boolean, onProgress?: OnProgress) =>
    invoke<RouteChangeReport>("nginx_route_change", { sessionId, file, line, change, apply, progress: progressChannel(onProgress) }),
  /** Services systemd reports as failed (for the health dot and the overview). */
  failedUnits: (sessionId: string) => invoke<string[]>("failed_units", { sessionId }),
  listServices: (sessionId: string) => invoke<Service[]>("list_services", { sessionId }),
  /** Ping, DNS, a TCP port, HTTP(S) or traceroute, run on the server. */
  netCheck: (sessionId: string, check: NetCheck, target: string, port: number | null) => invoke<NetResult>("net_check", { sessionId, check, target, port }),
  /** A unit's journal; since/until are Unix seconds. */
  unitLogs: (sessionId: string, unit: string, tail = 400, since: number | null = null, until: number | null = null) =>
    invoke<string>("unit_logs", { sessionId, unit, tail, since, until }),
  serviceDetail: (sessionId: string, unit: string) => invoke<ServiceDetail>("service_detail", { sessionId, unit }),
  serviceAction: (sessionId: string, unit: string, action: ServiceAction, onProgress?: OnProgress) =>
    invoke<ActionReport>("service_action", { sessionId, unit, action, progress: progressChannel(onProgress) }),
  listContainers: (sessionId: string) => invoke<Container[]>("list_containers", { sessionId }),
  containerStats: (sessionId: string) => invoke<ContainerStats[]>("container_stats", { sessionId }),
  inspectContainer: (sessionId: string, id: string) => invoke<any>("inspect_container", { sessionId, id }),
  /** The last `tail` lines; with since/until (ISO times) the last `tail` written between them. */
  containerLogs: (sessionId: string, id: string, tail = 500, since: string | null = null, until: string | null = null) =>
    invoke<string>("container_logs", { sessionId, id, tail, since, until }),
  containerAction: (sessionId: string, id: string, action: ContainerAction, onProgress?: OnProgress) =>
    invoke<ExecOutput>("container_action", { sessionId, id, action, progress: progressChannel(onProgress) }),
  /** docker compose up -d / restart / pull for a project or one service; returns Compose's output. */
  composeAction: (sessionId: string, target: ComposeTarget, action: ComposeAction, onProgress?: OnProgress) =>
    invoke<ExecOutput>("compose_action", { sessionId, target, action, progress: progressChannel(onProgress) }),
  /** The server's shell history, newest first (root's for root terminals), for terminal suggestions. */
  shellHistory: (sessionId: string, root: boolean) => invoke<string[]>("shell_history", { sessionId, root }),
  runCommand: (sessionId: string, command: string, privileged: boolean) =>
    invoke<ExecOutput>("run_command", { sessionId, command, privileged }),

  openTerminal: (sessionId: string, cols: number, rows: number, cwd: string | null, asRoot: boolean, onEvent: (m: TermMessage) => void, launch: TermLaunch = { kind: "shell" }, record = false) => {
    const ch = new Channel<TermMessage>();
    ch.onmessage = onEvent;
    return invoke<string>("open_terminal", { sessionId, cols, rows, cwd, asRoot, launch, record, onEvent: ch });
  },
  /** tmux sessions on the server, to reattach (root's for root terminals). */
  tmuxSessions: (sessionId: string, root: boolean) => invoke<TmuxSession[]>("tmux_sessions", { sessionId, root }),
  terminalWrite: (terminalId: string, data: string) => invoke<void>("terminal_write", { terminalId, data }),
  terminalResize: (terminalId: string, cols: number, rows: number) => invoke<void>("terminal_resize", { terminalId, cols, rows }),
  terminalClose: (terminalId: string) => invoke<void>("terminal_close", { terminalId }),

  processes: (sessionId: string) => invoke<Proc[]>("processes", { sessionId }),
  processDetail: (sessionId: string, pid: number) => invoke<ProcDetail>("process_detail", { sessionId, pid }),
  signalProcess: (sessionId: string, pid: number, signal: ProcSignal) => invoke<void>("signal_process", { sessionId, pid, signal }),
  reniceProcess: (sessionId: string, pid: number, nice: number) => invoke<void>("renice_process", { sessionId, pid, nice }),
  diskReport: (sessionId: string) => invoke<DiskReport>("disk_report", { sessionId }),
  installedPackages: (sessionId: string) => invoke<Package[]>("installed_packages", { sessionId }),
  searchPackages: (sessionId: string, term: string) => invoke<FoundPackage[]>("search_packages", { sessionId, term }),
  packagePlan: (sessionId: string, name: string, action: PkgAction) => invoke<PkgPlan>("package_plan", { sessionId, name, action }),
  /** Install or remove one package; a job. Removing what the server needs is refused unless forced. */
  packageChange: (sessionId: string, name: string, action: PkgAction, force: boolean, onProgress?: OnProgress) =>
    invoke<ExecOutput>("package_change", { sessionId, name, action, force, progress: progressChannel(onProgress) }),
  tlsInspect: (host: string, port: number) => invoke<TlsReport>("tls_inspect", { host, port }),
  checkSchedule: (sessionId: string, schedule: string) => invoke<ScheduleCheck>("check_schedule", { sessionId, schedule }),
  createTimer: (sessionId: string, spec: TimerSpec, onProgress?: OnProgress) => invoke<string>("create_timer", { sessionId, spec, progress: progressChannel(onProgress) }),
  removeTimer: (sessionId: string, name: string) => invoke<void>("remove_timer", { sessionId, name }),
  kernelReport: (sessionId: string) => invoke<KernelReport>("kernel_report", { sessionId }),
  /** Write Orbital's sysctl file with exactly these values (empty removes it) and load it; a job. */
  kernelApply: (sessionId: string, values: Record<string, string>, onProgress?: OnProgress) =>
    invoke<KernelReport>("kernel_apply", { sessionId, values, progress: progressChannel(onProgress) }),
  hardenState: (sessionId: string) => invoke<HardenState>("harden_state", { sessionId }),
  /** Write Orbital's sshd drop-in with exactly these settings (empty removes it), test and reload sshd. */
  hardenSsh: (sessionId: string, choices: Record<string, string>, force: boolean) => invoke<HardenResult>("harden_ssh", { sessionId, choices, force }),
  securityAudit: (sessionId: string, onProgress?: OnProgress) => invoke<AuditReport>("security_audit", { sessionId, progress: progressChannel(onProgress) }),
  sqlQuery: (sessionId: string, kind: DbKind, container: string | null, db: string, query: string, readOnly: boolean) =>
    invoke<SqlResult>("sql_query", { sessionId, kind, container, db, query, readOnly }),
  recordings: () => invoke<RecordingInfo[]>("recordings"),
  readRecording: (name: string) => invoke<string>("read_recording", { name }),
  deleteRecording: (name: string) => invoke<void>("delete_recording", { name }),
  snippets: () => invoke<Snippet[]>("snippets"),
  saveSnippet: (id: string | null, input: SnippetInput) => invoke<Snippet>("save_snippet", { id, input }),
  deleteSnippet: (id: string) => invoke<void>("delete_snippet", { id }),
  addExampleSnippets: () => invoke<Snippet[]>("add_example_snippets"),
  runSnippet: (id: string, sessionIds: string[], values: Record<string, string>, withSudo = false) => invoke<SnippetRun[]>("run_snippet", { id, sessionIds, values, withSudo }),
  localSshKeys: () => invoke<LocalKey[]>("local_ssh_keys"),
  generateSshKey: (name: string, comment: string, passphrase: string | null) => invoke<LocalKey>("generate_ssh_key", { name, comment, passphrase }),
  authorizedKeys: (sessionId: string) => invoke<AuthorizedKeys>("authorized_keys", { sessionId }),
  addAuthorizedKey: (sessionId: string, line: string) => invoke<AuthorizedKeys>("add_authorized_key", { sessionId, line }),
  removeAuthorizedKey: (sessionId: string, line: string, force: boolean) => invoke<AuthorizedKeys>("remove_authorized_key", { sessionId, line, force }),
  dnsAccounts: () => invoke<DnsAccount[]>("dns_accounts"),
  addDnsAccount: (name: string, kind: DnsProvider, token: string) => invoke<DnsAccount>("add_dns_account", { name, kind, token }),
  removeDnsAccount: (id: string) => invoke<void>("remove_dns_account", { id }),
  dnsZones: (accountId: string) => invoke<DnsZone[]>("dns_zones", { accountId }),
  dnsRecords: (accountId: string, zoneId: string) => invoke<DnsRecord[]>("dns_records", { accountId, zoneId }),
  createDnsRecord: (accountId: string, zoneId: string, zone: string, input: DnsRecordInput) =>
    invoke<DnsRecord>("create_dns_record", { accountId, zoneId, zone, input }),
  updateDnsRecord: (accountId: string, zoneId: string, zone: string, recordId: string, input: DnsRecordInput) =>
    invoke<DnsRecord>("update_dns_record", { accountId, zoneId, zone, recordId, input }),
  /** `label` ("A www.example.com") is only for the activity log. */
  deleteDnsRecord: (accountId: string, zoneId: string, recordId: string, label: string) =>
    invoke<void>("delete_dns_record", { accountId, zoneId, recordId, label }),
  /** What public resolvers answer for a name; `expected` empty means any answer. */
  dnsCheck: (name: string, rtype: DnsType, expected: string[]) => invoke<DnsCheck>("dns_check", { name, rtype, expected }),
  serverPublicAddresses: (sessionId: string) => invoke<string[]>("server_public_addresses", { sessionId }),
  /** Do these names resolve to this server? */
  dnsPointsHere: (sessionId: string, names: string[]) => invoke<{ addresses: string[]; checks: DnsCheck[] }>("dns_points_here", { sessionId, names }),

  auditQuery: (filter: { profileId?: string; category?: AuditCategory; search?: string; limit?: number; beforeSeq?: number }) =>
    invoke<AuditEvent[]>("audit_query", { filter }),
  auditVerify: () => invoke<{ ok: boolean; checked: number; firstBrokenSeq: number | null }>("audit_verify"),
  statsState: () => invoke<StatsState>("stats_state"),
  setStats: (enabled: boolean) => invoke<StatsState>("set_stats", { enabled }),
  statsPing: (event: "opened" | "active") => invoke<void>("stats_ping", { event }),
  /** Sent to the project's feedback server; only what was typed, plus version and OS. */
  sendFeedback: (feedback: FeedbackInput) => invoke<void>("send_feedback", { feedback }),
  /** Whether this build has a feedback server (release builds); otherwise feedback goes to GitHub issues. */
  feedbackAvailable: () => invoke<boolean>("feedback_available"),
  saveText: (path: string, text: string) => invoke<void>("save_text", { path, text }),
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
