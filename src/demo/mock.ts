// Demo mode: a fake backend so the UI runs in a plain browser (`npm run demo`)
// for design work, screenshots and marketing. Never included in the Tauri build path.
import { mockIPC } from "@tauri-apps/api/mocks";

const now = () => Math.floor(Date.now() / 1000);
const profiles: any[] = [
  { id: "p4", name: "media (R2)", host: "a1b2c3d4.r2.cloudflarestorage.com", port: 443, username: "6f1e0c2b9a", authMethod: "password", keyPath: null, sudoMode: "never", groupName: "Storage", color: null, initialPath: null, jumpProfileId: null, protocol: "s3", region: "auto", pathStyle: false, createdAt: "2026-10-01T10:00:00Z", lastConnectedAt: null },
  { id: "p5", name: "team cloud", host: "cloud.example.com", port: 443, username: "sirojiddin", authMethod: "password", keyPath: null, sudoMode: "never", groupName: "Storage", color: null, initialPath: "/remote.php/dav/files/sirojiddin", jumpProfileId: null, protocol: "webdavs", region: null, pathStyle: false, createdAt: "2026-10-01T10:00:00Z", lastConnectedAt: null },
  { id: "p1", name: "prod-web-01", host: "203.0.113.10", port: 22, username: "deploy", authMethod: "agent", keyPath: null, sudoMode: "password", groupName: "Production", color: "#e5645a", initialPath: null, jumpProfileId: null, protocol: "sftp", region: null, pathStyle: false, createdAt: "2026-10-01T10:00:00Z", lastConnectedAt: "2026-11-14T18:00:00Z" },
  { id: "p2", name: "db-primary", host: "10.0.4.21", port: 22, username: "ops", authMethod: "key", keyPath: "~/.ssh/id_ed25519", sudoMode: "noPassword", groupName: "Production", color: "#8fb4ff", initialPath: null, jumpProfileId: "p1", protocol: "sftp", region: null, pathStyle: false, createdAt: "2026-10-01T10:00:00Z", lastConnectedAt: "2026-11-12T08:00:00Z" },
  { id: "p3", name: "staging", host: "staging.example.dev", port: 2222, username: "sirojiddin", authMethod: "password", keyPath: null, sudoMode: "auto", groupName: "Staging", color: null, initialPath: null, jumpProfileId: null, protocol: "sftp", region: null, pathStyle: false, createdAt: "2026-10-01T10:00:00Z", lastConnectedAt: null },
];
const trusted = new Set(["p1", "p2"]);

// Snippets: saved commands; runs answer with plausible output per server.
let snippetList: any[] = [];
const snipParams = (c: string) => [...new Set([...c.matchAll(/\{\{\s*([A-Za-z_][A-Za-z0-9_]{0,31})\s*\}\}/g)].map((m) => m[1]))];
const mkSnippet = (id: string, i: any) => ({ id, name: i.name.trim(), command: i.command.trimEnd(), description: i.description?.trim() || null, sudo: !!i.sudo, params: snipParams(i.command), createdAt: new Date().toISOString(), updatedAt: new Date().toISOString() });
const EXAMPLES = [
  { name: "Disk space", command: "df -h -x tmpfs -x devtmpfs -x overlay -x squashfs", description: "Free space on every real disk.", sudo: false },
  { name: "Biggest folders", command: "du -xh --max-depth=1 {{folder}} 2>/dev/null | sort -rh | head -15", description: "What takes the space in a folder.", sudo: true },
  { name: "Memory and swap", command: "free -h && echo && ps -eo pid,user,rss,comm --sort=-rss | head -11", description: "Memory use and the ten largest processes.", sudo: false },
  { name: "Failed services", command: "systemctl --failed --no-legend --plain", description: "systemd units that have failed.", sudo: false },
  { name: "Restart a service", command: "systemctl restart {{service}} && systemctl --no-pager status {{service}} | head -12", description: "Restart a unit and show its status.", sudo: true },
  { name: "Recent errors", command: "journalctl -p err --since {{since}} --no-pager | tail -40", description: "Errors in the system journal, e.g. since \"1 hour ago\".", sudo: true },
  { name: "Pending updates", command: "(command -v apt >/dev/null && apt list --upgradable 2>/dev/null) || echo 'no apt'", description: "Packages with updates available.", sudo: false },
  { name: "Who is logged in", command: "who && echo && last -n 10", description: "Current sessions and the last logins.", sudo: false },
];
const shq = (v: string) => (/^[A-Za-z0-9_./:=@%+-]+$/.test(v) ? v : `'${v.replace(/'/g, "'\\''")}'`);
function fakeRun(cmd: string, sid: string) {
  const host = sid === "s-p1" ? "prod-web-01" : sid === "s-p2" ? "db-primary" : "staging";
  if (cmd.startsWith("df")) return { stdout: `Filesystem      Size  Used Avail Use% Mounted on\n/dev/sda1        75G   ${sid === "s-p2" ? "68G  3.9G  95%" : "31G   41G  44%"} /\n/dev/sdb1       200G  120G   80G  60% /var/lib/postgresql`, stderr: "", exitCode: 0 };
  if (cmd.startsWith("systemctl --failed")) return sid === "s-p2" ? { stdout: "pgbackrest-archive.service loaded failed failed pgBackRest archive push", stderr: "", exitCode: 0 } : { stdout: "", stderr: "", exitCode: 0 };
  if (cmd.startsWith("systemctl restart")) return sid === "s-p3" ? { stdout: "", stderr: `Failed to restart ${cmd.split(" ")[2]}.service: Unit not found.`, exitCode: 5 } : { stdout: `● ${cmd.split(" ")[2]}.service - running\n     Active: active (running) since ${new Date().toUTCString()}; 1s ago`, stderr: "", exitCode: 0 };
  if (cmd.startsWith("free")) return { stdout: "               total        used        free      shared  buff/cache   available\nMem:            15Gi       5.8Gi       1.2Gi       220Mi       8.6Gi        9.4Gi\nSwap:          2.0Gi          0B       2.0Gi", stderr: "", exitCode: 0 };
  return { stdout: `${host}: ok`, stderr: "", exitCode: 0 };
}

// SSH keys: two key pairs on this computer; the server lets one of them and a colleague's in.
const KEY_A = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl sirojiddin@macbook";
const KEY_B = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIK8sZ3Lr6xv1k7mJdF0q2WcB9yPnT4eRuHa5oGiXsQ2D work@macbook";
let localKeys: any[] = [
  { name: "id_ed25519", path: "~/.ssh/id_ed25519", algorithm: "ssh-ed25519", comment: "sirojiddin@macbook", fingerprint: "SHA256:Xw2pRfa9Lw0YkU0yV5mJbqzB2hD1tXcN7pGfE4sHjAo", publicLine: KEY_A, encrypted: false },
  { name: "id_ed25519_work", path: "~/.ssh/id_ed25519_work", algorithm: "ssh-ed25519", comment: "work@macbook", fingerprint: "SHA256:q8Lr1mVt7cNbS3xK2wPz9yHd4fJe6aUo0gTiR5nQkBc", publicLine: KEY_B, encrypted: true },
];
let authLines = [
  KEY_A,
  'command="/usr/local/bin/backup-only",no-pty,no-port-forwarding ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIJ3n backup@nas',
  "ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAABgQC7 anna@laptop",
];
const fpOf = (line: string) => localKeys.find((k) => line.includes(k.publicLine.split(" ")[1]))?.fingerprint ?? (line.includes("backup@nas") ? "SHA256:bK3uP0aZ1x9QmT5vLc2rN8yWe4dHs6oJiG7fRqA1nVk" : "SHA256:Ann4Lp0tK9xYv2mQ7cRs3wE5uZ8dH1oJbN6gTfi0aWq");
const authReport = () => ({
  user: "deploy", path: "~/.ssh/authorized_keys", missing: false,
  keys: authLines.map((line) => {
    const m = line.match(/(?:^|\s)(ssh-ed25519|ssh-rsa)\s+\S+\s*(.*)$/)!;
    const options = line.startsWith("ssh-") ? null : line.slice(0, line.indexOf(m[1])).trim();
    return { line, options, algorithm: m[1], comment: m[2], fingerprint: fpOf(line), onThisComputer: localKeys.some((k) => line.includes(k.publicLine.split(" ")[1])) };
  }),
});

// DNS: one Cloudflare account; records "propagate" a few seconds after they change.
const dnsAccounts: any[] = [{ id: "cf1", name: "Personal", kind: "cloudflare", createdAt: "2026-11-01T10:00:00Z" }];
const dnsZones: Record<string, any[]> = {
  cf1: [
    { id: "z1", name: "example.com", status: "active", nameServers: ["ada.ns.cloudflare.com", "bob.ns.cloudflare.com"] },
    { id: "z2", name: "shop-demo.dev", status: "pending", nameServers: ["kim.ns.cloudflare.com", "lou.ns.cloudflare.com"] },
  ],
};
let dnsRecs: any[] = [
  { zone: "z1", id: "r1", name: "example.com", type: "A", content: "203.0.113.10", ttl: 1, proxied: true, proxiable: true, priority: null, comment: null, at: 0 },
  { zone: "z1", id: "r2", name: "www.example.com", type: "CNAME", content: "example.com", ttl: 1, proxied: true, proxiable: true, priority: null, comment: null, at: 0 },
  { zone: "z1", id: "r3", name: "shop.example.com", type: "A", content: "203.0.113.10", ttl: 300, proxied: false, proxiable: true, priority: null, comment: "nginx on prod-web-01", at: 0 },
  { zone: "z1", id: "r4", name: "example.com", type: "MX", content: "mail.example.com", ttl: 3600, proxied: false, proxiable: false, priority: 10, comment: null, at: 0 },
  { zone: "z1", id: "r5", name: "example.com", type: "TXT", content: "v=spf1 include:_spf.example.net -all", ttl: 3600, proxied: false, proxiable: false, priority: null, comment: null, at: 0 },
  { zone: "z1", id: "r6", name: "example.com", type: "NS", content: "ada.ns.cloudflare.com", ttl: 86400, proxied: false, proxiable: false, priority: null, comment: null, at: 0 },
];
const dnsOut = ({ zone: _z, at: _a, ...r }: any) => ({ ...r });
const DOH = [["Cloudflare 1.1.1.1", 4000], ["Google 8.8.8.8", 12000]] as const;
function dnsCheck(name: string, type: string, expected: string[], any = false) {
  const norm = (v: string) => v.replace(/\.$/, "").toLowerCase();
  const answers = DOH.map(([resolver, delay]) => {
    const recs = dnsRecs.filter((r) => r.name === name && Date.now() - r.at >= delay);
    const values = recs.filter((r) => r.type === type).map((r) => (r.proxied ? (type === "A" ? "104.21.48.12" : "2606:4700:3033::6815:300c") : type === "MX" ? `${r.priority} ${r.content}` : r.content));
    const ok = recs.length > 0 && (expected.length === 0 ? values.length > 0 : any ? expected.some((e) => values.map(norm).includes(norm(e))) : expected.every((e) => values.map(norm).includes(norm(e))));
    return { resolver, status: recs.length || name.endsWith("example.org") ? "NOERROR" : "NXDOMAIN", values, ok, error: null };
  });
  return { name, type, expected, answers, verified: answers.every((a) => a.ok), checkedAt: new Date().toISOString() };
}
const presence = { available: true, enabled: false };

const NGINX = `user www-data;
worker_processes auto;
pid /run/nginx.pid;

events {
    worker_connections 2048;
}

http {
    sendfile on;
    tcp_nopush on;
    keepalive_timeout 65;
    server_tokens off;

    include /etc/nginx/mime.types;
    default_type application/octet-stream;

    # Rate limit the public API
    limit_req_zone $binary_remote_addr zone=api:10m rate=20r/s;

    upstream shop_api {
        server 127.0.0.1:8081 max_fails=3;
        server 127.0.0.1:8082 max_fails=3;
    }

    server {
        listen 443 ssl http2;
        server_name shop.example.com;

        ssl_certificate     /etc/letsencrypt/live/shop.example.com/fullchain.pem;
        ssl_certificate_key /etc/letsencrypt/live/shop.example.com/privkey.pem;

        location /api/ {
            limit_req zone=api burst=40 nodelay;
            proxy_pass http://shop_api;
            proxy_set_header Host $host;
            proxy_set_header X-Real-IP $remote_addr;
        }

        location ~* \\.(js|css|png|svg|woff2)$ {
            root /srv/shop/public;
            expires 30d;
        }
    }

    include /etc/nginx/sites-enabled/*;
}
`;

const dirs: Record<string, [string, "file" | "dir" | "symlink", number, string][]> = {
  "/": [["etc", "dir", 4096, "rwxr-xr-x"], ["home", "dir", 4096, "rwxr-xr-x"], ["srv", "dir", 4096, "rwxr-xr-x"], ["var", "dir", 4096, "rwxr-xr-x"]],
  "/etc/cron.daily": [["apt-compat", "file", 1478, "rwxr-xr-x"], ["dpkg", "file", 652, "rwxr-xr-x"], ["logrotate", "file", 377, "rwxr-xr-x"]],
  "/etc/cron.weekly": [["man-db", "file", 1020, "rwxr-xr-x"]],
  "/etc": [["cron.d", "dir", 4096, "rwxr-xr-x"], ["cron.daily", "dir", 4096, "rwxr-xr-x"], ["docker", "dir", 4096, "rwxr-xr-x"], ["nginx", "dir", 4096, "rwxr-xr-x"], ["ssh", "dir", 4096, "rwxr-xr-x"], ["systemd", "dir", 4096, "rwxr-xr-x"], ["hosts", "file", 221, "rw-r--r--"], ["sudoers", "file", 1800, "r--r-----"]],
  "/home/deploy": [[".bashrc", "file", 3771, "rw-r--r--"], [".ssh", "dir", 4096, "rwx------"], ["apps", "dir", 4096, "rwxr-xr-x"], ["backup.sh", "file", 812, "rwxr-xr-x"], ["docker-compose.yml", "file", 1460, "rw-r--r--"]],
  "/etc/nginx": [["conf.d", "dir", 4096, "rwxr-xr-x"], ["sites-available", "dir", 4096, "rwxr-xr-x"], ["sites-enabled", "dir", 4096, "rwxr-xr-x"], ["snippets", "dir", 4096, "rwxr-xr-x"], ["fastcgi_params", "file", 1125, "rw-r--r--"], ["mime.types", "file", 5465, "rw-r--r--"], ["nginx.conf", "file", 1482, "rw-r--r--"]],
};
const listing = (path: string) =>
  (dirs[path] ?? [["README.md", "file", 2048, "rw-r--r--"]]).map(([name, kind, size, mode], i) => ({
    name, path: (path === "/" ? "" : path) + "/" + name, kind, size, mode, modeOctal: 0o644,
    mtime: now() - (i + 1) * 5400 * (i + 2), owner: path.startsWith("/etc") ? "root" : "deploy", group: path.startsWith("/etc") ? "root" : "deploy",
    uid: 0, gid: 0, hidden: name.startsWith("."),
  }));

let cpuBase = [3712131, 46828483];
const metric = () => {
  cpuBase = [cpuBase[0] + 40 + Math.random() * 60, cpuBase[1] + 180];
  const t = Date.now() / 1000;
  const cpu = 22 + 14 * Math.sin(t / 9) + Math.random() * 8;
  return {
    hostname: "prod-web-01", osName: "Ubuntu 24.04.1 LTS", kernel: "Linux 6.8.0-45-generic x86_64",
    cpuModel: "AMD EPYC 7B13 64-Core Processor", cores: 8, load: [1.42 + Math.random() * 0.3, 1.21, 0.98],
    memTotal: 16_658_984_960, memAvailable: 7_100_000_000 - Math.random() * 2e8, memUsed: 9_558_984_960 + Math.random() * 2e8,
    memCached: 4_900_000_000, swapTotal: 2_147_483_648, swapUsed: 182_000_000,
    memory: (() => {
      const total = 16_658_984_960, free = 2_150_000_000 - Math.random() * 1e8, buffCache = 5_210_000_000;
      return {
        installed: 17_179_869_184, used: total - free - buffCache, buffCache, free, buffers: 310_000_000, pageCache: 4_590_000_000,
        anon: 7_480_000_000, shared: 212_000_000, slabReclaimable: 310_000_000, slabUnreclaimable: 196_000_000, kernelStack: 24_000_000,
        pageTables: 88_000_000, dirty: 3_400_000 + Math.random() * 2e6, committed: 11_900_000_000, commitLimit: 10_476_975_104,
        active: 8_100_000_000, inactive: 5_300_000_000, swapCached: 21_000_000, hugePages: 0,
      };
    })(), uptimeSecs: 3_456_789 + (t % 1000),
    disks: [
      { filesystem: "/dev/nvme0n1p1", fsType: "ext4", mount: "/", totalBytes: 160e9, usedBytes: 61e9, availableBytes: 91e9, usedPercent: 40.1 },
      { filesystem: "/dev/nvme1n1", fsType: "xfs", mount: "/var/lib/docker", totalBytes: 500e9, usedBytes: 455e9, availableBytes: 45e9, usedPercent: 91.0 },
      { filesystem: "/dev/nvme0n1p15", fsType: "vfat", mount: "/boot/efi", totalBytes: 105e6, usedBytes: 6e6, availableBytes: 99e6, usedPercent: 5.8 },
    ],
    net: [{ name: "eth0", rxBytes: 0, txBytes: 0 }],
    processes: [
      { pid: 812, user: "www-data", cpuPercent: 12.5, memPercent: 1.2, rssKb: 98304, command: "nginx" },
      { pid: 2210, user: "deploy", cpuPercent: 9.8, memPercent: 14.1, rssKb: 2348000, command: "java" },
      { pid: 1022, user: "root", cpuPercent: 3.1, memPercent: 4.0, rssKb: 327680, command: "dockerd" },
      { pid: 611, user: "postgres", cpuPercent: 2.0, memPercent: 12.5, rssKb: 1024000, command: "postgres" },
      { pid: 77, user: "root", cpuPercent: 0.4, memPercent: 0.3, rssKb: 22000, command: "systemd-journald" },
    ],
    cpuPercent: cpu, perCorePercent: Array.from({ length: 8 }, (_, i) => Math.max(2, cpu + (i % 3) * 9 - 10 + Math.random() * 12)),
    netRxPerSec: 1_200_000 + Math.random() * 400_000, netTxPerSec: 3_400_000 + Math.random() * 900_000, sampledAtMs: Date.now(),
  };
};

const services = [
  ["nginx.service", "active", "running", "A high performance web server and a reverse proxy server", "enabled"],
  ["shop-api.service", "active", "running", "Shop API (Spring Boot)", "enabled"],
  ["docker.service", "active", "running", "Docker Application Container Engine", "enabled"],
  ["postgresql.service", "active", "exited", "PostgreSQL RDBMS", "enabled"],
  ["ssh.service", "active", "running", "OpenBSD Secure Shell server", "enabled"],
  ["cron.service", "active", "running", "Regular background program processing daemon", "enabled"],
  ["fail2ban.service", "active", "running", "Fail2Ban Service", "enabled"],
  ["report-worker.service", "failed", "failed", "Nightly report worker", "enabled"],
  ["redis-server.service", "inactive", "dead", "Advanced key-value store", "disabled"],
].map(([unit, active, sub, description, unitFileState]) => ({ unit, load: "loaded", active, sub, description, unitFileState }));

const containers = [
  { id: "3f2a9c11aa01", names: "shop-web-1", image: "nginx:1.27-alpine", state: "running", status: "Up 2 weeks", ports: "0.0.0.0:8080->80/tcp", createdAt: "", composeProject: "shop" },
  { id: "8b1d0e22bb02", names: "shop-api-1", image: "registry.example.com/shop-api:2.14.0", state: "running", status: "Up 3 days (healthy)", ports: "8081/tcp", createdAt: "", composeProject: "shop" },
  { id: "c4e5f633cc03", names: "shop-worker-1", image: "registry.example.com/shop-api:2.14.0", state: "restarting", status: "Restarting (1) 12 seconds ago", ports: "", createdAt: "", composeProject: "shop" },
  { id: "d9a8b744dd04", names: "grafana", image: "grafana/grafana:11.3.0", state: "running", status: "Up 6 weeks", ports: "127.0.0.1:3000->3000/tcp", createdAt: "", composeProject: "observability" },
  { id: "e1f2a355ee05", names: "loki", image: "grafana/loki:3.2.1", state: "running", status: "Up 6 weeks", ports: "3100/tcp", createdAt: "", composeProject: "observability" },
  { id: "f7c6d466ff06", names: "migrate-once", image: "flyway/flyway:10", state: "exited", status: "Exited (0) 3 days ago", ports: "", createdAt: "", composeProject: null },
];

const audit = [
  ["config.write", "config", "/etc/nginx/nginx.conf", null, "success", null],
  ["service.reload", "service", "nginx.service", "sudo -k -S -p '' -- sh -c 'systemctl reload -- nginx.service'", "success", 0],
  ["config.write", "config", "/etc/nginx/sites-enabled/shop", null, "failure", null],
  ["docker.restart", "docker", "shop-api-1", "docker restart -- shop-api-1", "success", 0],
  ["terminal.open", "terminal", "/srv/shop", null, "success", null],
  ["file.upload", "file", "/srv/shop/public/robots.txt", null, "success", null],
  ["service.stop", "service", "redis-server.service", "sudo -k -S -p '' -- sh -c 'systemctl stop -- redis-server.service'", "success", 0],
  ["hostkey.trust", "security", "deploy@203.0.113.10:22", null, "success", null],
  ["session.open", "connection", "deploy@203.0.113.10:22", null, "success", null],
].map(([action, category, target, command, outcome, exitCode], i) => ({
  seq: 100 - i, id: String(i), ts: new Date(Date.now() - i * 1000 * 60 * 37).toISOString(), profileId: "p1",
  server: "deploy@203.0.113.10", actor: "sirojiddin", category, action, target, command, outcome, exitCode,
  durationMs: 40 + i * 31, details: outcome === "failure" ? { error: "Validation failed, change was rolled back: nginx: [emerg] unexpected \"}\" in /etc/nginx/sites-enabled/shop:41", validator: "nginx" } : { sha256: "9f86d081884c7d65…", backup: target?.toString().startsWith("/etc") ? `${target}.orbital-bak-20261114" ` : undefined },
  prevHash: "a3c1…", hash: "5be0d2f4c8a1e9b7…",
}));

let statsState: { available: boolean; enabled: boolean | null } = { available: true, enabled: null };

const tools = ([
  { id: "nginx", name: "nginx", category: "Web server", running: true, binary: "/usr/sbin/nginx", unit: "nginx.service", unitState: "active", paths: [{ path: "/etc/nginx", isDir: true }, { path: "/var/log/nginx", isDir: true }] },
  { id: "docker", name: "Docker", category: "Containers", running: true, binary: "/usr/bin/docker", unit: "docker.service", unitState: "active", paths: [{ path: "/etc/docker/daemon.json", isDir: false }] },
  { id: "postgresql", name: "PostgreSQL", category: "Database", running: true, binary: "/usr/bin/psql", unit: "postgresql.service", unitState: "active", paths: [{ path: "/etc/postgresql", isDir: true }, { path: "/var/log/postgresql", isDir: true }] },
  { id: "sshd", name: "SSH server", category: "Access", running: true, binary: "/usr/sbin/sshd", unit: "ssh.service", unitState: "active", paths: [{ path: "/etc/ssh/sshd_config", isDir: false }, { path: "/etc/ssh/sshd_config.d", isDir: true }] },
  { id: "fail2ban", name: "Fail2ban", category: "Access", running: true, binary: "/usr/bin/fail2ban-server", unit: "fail2ban.service", unitState: "active", paths: [{ path: "/etc/fail2ban", isDir: true }] },
  { id: "ufw", name: "UFW firewall", category: "Access", running: true, binary: "/usr/sbin/ufw", unit: "ufw.service", unitState: "active", paths: [{ path: "/etc/ufw", isDir: true }, { path: "/etc/default/ufw", isDir: false }] },
  { id: "certbot", name: "HTTPS certificates", category: "Certificates", running: true, binary: "/usr/bin/certbot", unit: "certbot.timer", unitState: "active", paths: [{ path: "/etc/letsencrypt/renewal", isDir: true }, { path: "/var/log/letsencrypt", isDir: true }] },
  { id: "apache", name: "Apache", category: "Web server", running: true, binary: "/usr/sbin/apache2", unit: "apache2.service", unitState: "active", paths: [{ path: "/etc/apache2", isDir: true }] },
  { id: "haproxy", name: "HAProxy", category: "Load balancer", running: true, binary: "/usr/sbin/haproxy", unit: "haproxy.service", unitState: "active", paths: [{ path: "/etc/haproxy/haproxy.cfg", isDir: false }] },
  { id: "mysql", name: "MySQL / MariaDB", category: "Database", running: true, binary: "/usr/sbin/mysqld", unit: "mysql.service", unitState: "active", paths: [{ path: "/etc/mysql", isDir: true }] },
  { id: "cron", name: "Cron", category: "Scheduling", running: true, binary: "/usr/sbin/cron", unit: "cron.service", unitState: "active", paths: [{ path: "/etc/crontab", isDir: false }, { path: "/etc/cron.d", isDir: true }] },
  { id: "redis", name: "Redis", category: "Database", running: true, binary: "/usr/bin/redis-server", unit: "redis-server.service", unitState: "inactive", paths: [{ path: "/etc/redis/redis.conf", isDir: false }], containers: ["shop-cache-1"] },
  { id: "mongodb", name: "MongoDB", category: "Database", running: true, binary: null, unit: null, unitState: null, paths: [], onHost: false, containers: ["analytics-mongo"] },
] as any[]).map((t) => ({ onHost: true, containers: [], ...t }));

const nginxConfig: { testOk: boolean; testOutput: string; files: { path: string; lines: number }[]; upstreams: any[]; sites: any[] } = {
  testOk: true,
  testOutput: "nginx: the configuration file /etc/nginx/nginx.conf syntax is ok\nnginx: configuration file /etc/nginx/nginx.conf test is successful",
  files: [
    { path: "/etc/nginx/nginx.conf", lines: 47 },
    { path: "/etc/nginx/mime.types", lines: 98 },
    { path: "/etc/nginx/sites-enabled/shop", lines: 41 },
    { path: "/etc/nginx/sites-enabled/grafana", lines: 18 },
    { path: "/etc/nginx/conf.d/default.conf", lines: 9 },
  ],
  upstreams: [{ name: "shop_api", servers: ["127.0.0.1:8081 max_fails=3", "127.0.0.1:8082 max_fails=3"], file: "/etc/nginx/nginx.conf", line: 21 }],
  sites: [
    { file: "/etc/nginx/sites-enabled/shop", line: 1, serverNames: ["shop.example.com", "www.shop.example.com"], listens: ["80", "[::]:80"], ssl: false, sslCertificate: null, root: null, redirect: "301 https://$host$request_uri", locations: [] },
    { file: "/etc/nginx/sites-enabled/shop", line: 8, serverNames: ["shop.example.com"], listens: ["443 ssl http2"], ssl: true, sslCertificate: "/etc/letsencrypt/live/shop.example.com/fullchain.pem", root: "/srv/shop/public", redirect: null, locations: [
      { path: "/api/", proxyPass: "http://shop_api", root: null, line: 16 },
      { path: "~* \\.(js|css|png|svg|woff2)$", proxyPass: null, root: "/srv/shop/public", line: 23 },
      { path: "/", proxyPass: null, root: null, line: 28 },
    ] },
    { file: "/etc/nginx/sites-enabled/grafana", line: 1, serverNames: ["grafana.example.com"], listens: ["443 ssl"], ssl: true, sslCertificate: "/etc/letsencrypt/live/grafana.example.com/fullchain.pem", root: null, redirect: null, locations: [{ path: "/", proxyPass: "http://127.0.0.1:3000", root: null, line: 9 }] },
    { file: "/etc/nginx/sites-enabled/blog", line: 1, serverNames: ["blog.example.com"], listens: ["80"], ssl: false, sslCertificate: null, root: "/srv/blog/public", redirect: null, locations: [{ path: "/", proxyPass: null, root: null, line: 5 }] },
    { file: "/etc/nginx/conf.d/default.conf", line: 1, serverNames: [], listens: ["80 default_server"], ssl: false, sslCertificate: null, root: "/usr/share/nginx/html", redirect: null, locations: [] },
  ],
};

// ---- new website (the same rules and template as website.rs) ----
const SYSTEM_DIRS = ["/", "/bin", "/boot", "/dev", "/etc", "/home", "/lib", "/opt", "/proc", "/root", "/run", "/sbin", "/srv", "/sys", "/tmp", "/usr", "/var", "/var/lib", "/var/log", "/var/www"];
function websiteInput(input: any) {
  const domain = String(input.domain).trim().toLowerCase().replace(/\.$/, "");
  if (!/^(?:[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?\.)+[a-z][a-z0-9-]{0,61}[a-z0-9]$/.test(domain)) throw { kind: "InvalidInput", message: `"${domain}" is not a domain name` };
  const site = input.site;
  if (site.kind === "proxy") {
    if (!site.port) throw { kind: "InvalidInput", message: "choose the port your app listens on (1–65535)" };
    if (site.port === 80 || site.port === 443) throw { kind: "InvalidInput", message: `port ${site.port} is nginx itself; use the port your app listens on, for example 3000 or 8080` };
    if (site.port === 22) throw { kind: "InvalidInput", message: "port 22 is SSH, not a web app" };
  } else {
    const r = String(site.root).trim().replace(/(.)\/+$/, "$1");
    if (!/^(\/[A-Za-z0-9._@+-]+)+$/.test(r) || r.split("/").some((c) => c === "." || c === "..")) throw { kind: "InvalidInput", message: `"${r}": use an absolute folder path made of letters, digits and . _ - @ +, for example /var/www/example.com` };
    if (SYSTEM_DIRS.includes(r) || /^\/(etc|proc|sys|dev|boot|root|run)\//.test(r)) throw { kind: "InvalidInput", message: `${r} is a system folder; serve the site from its own folder, for example /var/www/<domain>` };
    site.root = r;
  }
  const names = input.includeWww && !domain.startsWith("www.") ? [domain, `www.${domain}`] : [domain];
  return { domain, names, site };
}
function websitePlan(input: any) {
  const { domain, names, site } = websiteInput(input);
  const body = site.kind === "static"
    ? `    root ${site.root};\n    index index.html index.htm;\n\n    location / {\n        try_files $uri $uri/ =404;\n    }\n`
    : `    location / {\n        proxy_pass http://127.0.0.1:${site.port};\n        proxy_http_version 1.1;\n        proxy_set_header Host $host;\n        proxy_set_header X-Real-IP $remote_addr;\n        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;\n        proxy_set_header X-Forwarded-Proto $scheme;\n        # WebSockets\n        proxy_set_header Upgrade $http_upgrade;\n        proxy_set_header Connection "upgrade";\n        proxy_read_timeout 300s;\n    }\n`;
  const content = `# ${domain}: created by Orbital. Edit it like any other nginx file;\n# HTTPS is added by certbot once the certificate is issued.\nserver {\n    listen 80;\n    listen [::]:80;\n    server_name ${names.join(" ")};\n\n${body}}\n`;
  const file = `/etc/nginx/sites-available/${domain}`;
  const link = `/etc/nginx/sites-enabled/${domain}`;
  const blockers = [
    ...nginxConfig.files.filter((f) => f.path === link).map((f) => `${f.path} already exists`),
    ...nginxConfig.sites.flatMap((x) => x.serverNames.filter((n: string) => names.includes(n.toLowerCase())).map((n: string) => `${n} is already served by ${x.file}:${x.line}`)),
  ];
  const root = site.kind === "static" ? site.root : null;
  return { domain, names, layout: "sitesAvailable", file, enabledLink: link, root, rootExists: root === "/var/www/html" || root?.startsWith("/srv/") === true, content, blockers };
}
function createWebsite(input: any) {
  const plan = websitePlan(input);
  if (plan.blockers.length) throw { kind: "InvalidInput", message: plan.blockers.join("\n") };
  nginxConfig.files.push({ path: plan.enabledLink, lines: plan.content.split("\n").length - 1 });
  nginxConfig.sites.push({ file: plan.enabledLink, line: 3, serverNames: plan.names, listens: ["80", "[::]:80"], ssl: false, sslCertificate: null, root: plan.root, redirect: null,
    locations: [{ path: "/", proxyPass: input.site.kind === "proxy" ? `http://127.0.0.1:${input.site.port}` : null, root: null, line: plan.root ? 12 : 9 }] });
  return {
    plan,
    write: { path: plan.file, bytes: plan.content.length, privileged: true, backupPath: null, validator: "nginx", validatorOutput: "nginx: the configuration file /etc/nginx/nginx.conf syntax is ok\nnginx: configuration file /etc/nginx/nginx.conf test is successful", newMtime: now(), suggestedReload: "nginx.service" },
    rootCreated: !!plan.root && !plan.rootExists,
    reloaded: true,
    reloadOutput: "",
  };
}
/** After certbot --nginx: the site gets its HTTPS server block and the http one redirects. */
function addHttps(domains: string[]) {
  const site = nginxConfig.sites.find((x) => x.serverNames.includes(domains[0]) && !x.ssl);
  if (!site) return;
  const cert = `/etc/letsencrypt/live/${domains[0]}/fullchain.pem`;
  nginxConfig.sites.push({ ...site, line: site.line + 20, listens: ["443 ssl", "[::]:443 ssl"], ssl: true, sslCertificate: cert });
  Object.assign(site, { redirect: "301 https://$host$request_uri", root: null, locations: [] });
}

// ---- security tools ----
const raw = (to: string, action: string, from: string, comment?: string) => `${to} ${action} ${from}${comment ? ` # ${comment}` : ""}`;
let ufwRules: [string, string, string, string, string | null][] = [
  ["22/tcp", "LIMIT", "IN", "Anywhere", "SSH"],
  ["80,443/tcp", "ALLOW", "IN", "Anywhere", "web"],
  ["5432/tcp", "ALLOW", "IN", "10.0.0.0/8", "PostgreSQL from the VPC"],
  ["Anywhere", "DENY", "IN", "198.51.100.23", null],
  ["22/tcp (v6)", "LIMIT", "IN", "Anywhere (v6)", "SSH"],
  ["80,443/tcp (v6)", "ALLOW", "IN", "Anywhere (v6)", "web"],
];
let ufwActive = true;
const ufw = () => ({
  active: ufwActive, defaultIncoming: "deny", defaultOutgoing: "allow", logging: "on (low)",
  rules: ufwActive ? ufwRules.map(([to, action, direction, from, comment], i) => ({ number: i + 1, to, action, direction, from, v6: to.includes("(v6)"), comment, raw: raw(to, `${action} ${direction}`, from, comment ?? undefined) })) : [],
  added: ufwActive ? [] : ["limit 22/tcp comment 'SSH'", "allow 80,443/tcp comment 'web'"],
  apps: ["Nginx Full", "Nginx HTTP", "Nginx HTTPS", "OpenSSH"],
});
const lockout = { kind: "PreflightFailed", message: "Pre-flight check failed", data: { check: "SSH access", output: "After this change the firewall would block SSH on port 22 from 203.0.113.9, which would cut off this connection and lock you out. Allow port 22 first." } };
const banned: Record<string, string[]> = { sshd: ["45.155.205.233", "193.32.162.81", "218.92.0.112"], "nginx-http-auth": [] };
const f2b = () => ({
  version: "1.0.2", running: true, error: null, clientIp: "203.0.113.9",
  jails: [
    { name: "sshd", currentlyFailed: 4, totalFailed: 1893, currentlyBanned: banned.sshd.length, totalBanned: 212, banned: banned.sshd, watching: ["_SYSTEMD_UNIT=sshd.service + _COMM=sshd"], bantime: "3600", findtime: "600", maxretry: "5", ignoreIp: ["127.0.0.1/8", "::1"] },
    { name: "nginx-http-auth", currentlyFailed: 0, totalFailed: 7, currentlyBanned: banned["nginx-http-auth"].length, totalBanned: 1, banned: banned["nginx-http-auth"], watching: ["/var/log/nginx/error.log"], bantime: "600", findtime: "600", maxretry: "3", ignoreIp: [] },
  ],
});
const days = (n: number) => new Date(Date.now() + n * 86400e3).toISOString();
const certs = () => ({
  version: "2.9.0", plugins: ["nginx", "standalone", "webroot"], error: null,
  autoRenew: { via: "certbot.timer", active: true, nextRun: "Sun 2026-11-15 22:41:00 UTC" },
  certificates: [
    { name: "shop.example.com", domains: ["shop.example.com", "www.shop.example.com"], expires: days(56), daysLeft: 56, valid: true, keyType: "ECDSA", certificatePath: "/etc/letsencrypt/live/shop.example.com/fullchain.pem", privateKeyPath: "/etc/letsencrypt/live/shop.example.com/privkey.pem", status: "VALID: 56 days" },
    { name: "grafana.example.com", domains: ["grafana.example.com"], expires: days(12), daysLeft: 12, valid: true, keyType: "RSA", certificatePath: "/etc/letsencrypt/live/grafana.example.com/fullchain.pem", privateKeyPath: "/etc/letsencrypt/live/grafana.example.com/privkey.pem", status: "VALID: 12 days" },
  ],
});
/** Play a job's progress (steps, output lines, transfers) on its channel, then return. */
const stopped = new Set<number>();
async function progress(ch: any, events: ([number, any] | [number, "step" | "line", string])[]) {
  for (const [ms, a, b] of events as any[]) {
    await new Promise((r) => setTimeout(r, ms));
    if (ch && stopped.has(ch.id)) throw { kind: "Cancelled", message: "Stopped before it finished" };
    ch?.onmessage?.(typeof a === "string" ? { type: a, text: b } : a);
  }
}
const transferEvents = (files: number, total: number, names: string[], ms: number) =>
  Array.from({ length: 12 }, (_, i) => [ms / 12, { type: "transfer", files: Math.round(((i + 1) / 12) * files), bytes: Math.round(((i + 1) / 12) * total), current: names[i % names.length] }] as [number, any]);

const cleaned = new Set<string>();
const cleanupItems = () => [
  { id: "journal", title: "System journal older than a week", detail: "systemd's log of everything. The last 7 days are kept.", bytes: cleaned.has("journal") ? 88 * MB : 700 * MB },
  { id: "apt", title: "Downloaded package files (apt)", detail: "Installers apt kept after installing updates. Downloaded again if ever needed.", bytes: cleaned.has("apt") ? 0 : 214 * MB },
  { id: "oldlogs", title: "Rotated old log files", detail: "Compressed and numbered logs in /var/log (*.gz, *.1, *.old). Current logs are kept.", bytes: cleaned.has("oldlogs") ? 0 : 96 * MB },
  { id: "docker", title: "Docker leftovers", detail: "Stopped containers, unused networks, dangling images and build cache. Running containers and volumes (data) are kept.", bytes: cleaned.has("docker") ? 0 : 1900 * MB },
  { id: "dockerimages", title: "Docker images no container uses", detail: "Pulled again when a container needs one. Can take a while to download later.", bytes: cleaned.has("dockerimages") ? 0 : 3100 * MB },
];
const agent: any = { installed: false, running: false, version: null, provider: null, model: null, bot: null, alerts: true, pairCodeActive: false, owner: null, pairedAt: null, alerting: [], recent: [], updateAvailable: false, dailyReport: null, latestVersion: "1.3.1" };
const agentLog: any[] = [];
/** A pairing code; in the demo, "you" send it from Telegram 8 seconds later. */
function agentCode() {
  const code = "K7RW4QZP";
  const stamp = () => new Date().toISOString().slice(0, 19);
  setTimeout(() => {
    if (!agent.installed) return;
    Object.assign(agent, { owner: "sirojiddin", pairedAt: Math.floor(Date.now() / 1000), pairCodeActive: false });
    agentLog.push({ time: stamp(), event: "paired", name: "sirojiddin" }, { time: stamp(), event: "question", chars: 18 }, { time: stamp(), event: "tool", tool: "server_overview" }, { time: stamp(), event: "tool", tool: "container_logs" }, { time: stamp(), event: "approval_asked", tool: "container_action", action: "docker restart shop-worker-1" }, { time: stamp(), event: "approved", tool: "container_action", action: "docker restart shop-worker-1" }, { time: stamp(), event: "action_done", tool: "container_action", action: "docker restart shop-worker-1" });
  }, 8000);
  return { code, expires: Math.floor(Date.now() / 1000) + 15 * 60, bot: "my_server_helper_bot" };
}
let upgraded = false;
const updatesReport = () => ({
  manager: "apt",
  refreshedAt: Math.floor(Date.now() / 1000) - 5 * 3600,
  rebootRequired: true,
  packages: upgraded ? [] : [
    { name: "libssl3t64", current: "3.0.13-0ubuntu3.1", available: "3.0.13-0ubuntu3.4", security: true },
    { name: "openssl", current: "3.0.13-0ubuntu3.1", available: "3.0.13-0ubuntu3.4", security: true },
    { name: "linux-image-generic", current: "6.8.0-45.45", available: "6.8.0-49.49", security: true },
    { name: "curl", current: "8.5.0-2ubuntu10.4", available: "8.5.0-2ubuntu10.5", security: false },
    { name: "nginx", current: "1.24.0-2ubuntu7.1", available: "1.24.0-2ubuntu7.2", security: false },
    { name: "docker-ce", current: "5:27.3.1-1~ubuntu.24.04~noble", available: "5:27.4.0-1~ubuntu.24.04~noble", security: false },
  ],
});

const certbotOut = (title: string) => ({ stdout: `Saving debug log to /var/log/letsencrypt/letsencrypt.log\n${title}\n`, stderr: "", exitCode: 0, durationMs: 8200 });
const ports = {
  source: "ss", complete: true, firewallActive: true,
  ports: [
    { protocol: "tcp", address: "0.0.0.0", port: 22, scope: "all", process: "sshd", pid: 900, unit: "ssh.service", firewall: "open" },
    { protocol: "tcp", address: "::", port: 22, scope: "all", process: "sshd", pid: 900, unit: "ssh.service", firewall: "open" },
    { protocol: "udp", address: "127.0.0.53", port: 53, scope: "local", process: "systemd-resolve", pid: 500, unit: "systemd-resolved.service", firewall: null },
    { protocol: "tcp", address: "0.0.0.0", port: 80, scope: "all", process: "nginx", pid: 812, unit: "nginx.service", firewall: "open" },
    { protocol: "tcp", address: "0.0.0.0", port: 443, scope: "all", process: "nginx", pid: 812, unit: "nginx.service", firewall: "open" },
    { protocol: "tcp", address: "127.0.0.1", port: 3000, scope: "local", process: "grafana", pid: 2210, unit: "grafana-server.service", firewall: null },
    { protocol: "tcp", address: "0.0.0.0", port: 5432, scope: "all", process: "postgres", pid: 1200, unit: "postgresql@16-main.service", firewall: "restricted" },
    { protocol: "tcp", address: "127.0.0.1", port: 6379, scope: "local", process: "redis-server", pid: 1302, unit: "redis-server.service", firewall: null },
    { protocol: "tcp", address: "0.0.0.0", port: 8080, scope: "all", process: "docker-proxy", pid: 3001, unit: "docker.service", firewall: "docker" },
    { protocol: "tcp", address: "10.0.0.5", port: 9100, scope: "address", process: "node_exporter", pid: 1410, unit: "node-exporter.service", firewall: "blocked" },
  ],
};
const sshd = {
  version: "OpenSSH_9.6p1 Ubuntu-3ubuntu13.5", testOk: true, testOutput: "", readable: true,
  configFiles: ["/etc/ssh/sshd_config", "/etc/ssh/sshd_config.d/50-cloud-init.conf"],
  sessions: [{ user: "deploy", tty: "pts/0", since: "2026-11-15 10:01", from: "203.0.113.9" }, { user: "deploy", tty: "pts/1", since: "2026-11-15 11:42", from: "203.0.113.9" }],
  settings: { port: "22", permitrootlogin: "yes", passwordauthentication: "yes", kbdinteractiveauthentication: "no", pubkeyauthentication: "yes", permitemptypasswords: "no", maxauthtries: "6", x11forwarding: "yes", logingracetime: "120", usepam: "yes", clientaliveinterval: "0", allowtcpforwarding: "yes" },
  checks: [
    { id: "root-login", title: "Root login", level: "bad", setting: "PermitRootLogin", value: "yes", advice: "Anyone who guesses root's password owns the server. Set PermitRootLogin to no (or prohibit-password) once you can log in as another user with sudo." },
    { id: "password-login", title: "Password login", level: "warn", setting: "PasswordAuthentication", value: "yes", advice: "Bots try passwords on every public server all day. Once your key works, set PasswordAuthentication to no. Make sure this profile logs in with a key first, or you will lock yourself out." },
    { id: "max-auth-tries", title: "Attempts per connection", level: "warn", setting: "MaxAuthTries", value: "6", advice: "Each connection may try this many passwords or keys. 3 or 4 slows guessing down." },
    { id: "x11", title: "X11 forwarding", level: "warn", setting: "X11Forwarding", value: "yes", advice: "Lets remote graphical apps reach your screen. Servers rarely need it: set X11Forwarding to no." },
    { id: "grace", title: "Time to log in", level: "info", setting: "LoginGraceTime", value: "120", advice: "Seconds an unauthenticated connection may stay open. 30 frees slots held by bots sooner." },
    { id: "allow-list", title: "Who may log in", level: "info", setting: "AllowUsers / AllowGroups", value: "everyone with an account", advice: "Any account with a valid key or password can log in. AllowUsers or AllowGroups limits SSH to the people who need it." },
    { id: "port", title: "Port", level: "info", setting: "Port", value: "22", advice: "The standard port. Moving it cuts log noise from bots but doesn't stop a real attacker; keys and fail2ban matter more." },
  ],
};

const COMPOSE = `services:
  web:
    image: nginx:1.27-alpine
    ports:
      - "8080:80"
    volumes:
      - ./public:/usr/share/nginx/html:ro
    depends_on: [api]
    restart: unless-stopped

  api:
    image: ghcr.io/example/shop-api:2.4.1
    environment:
      SPRING_PROFILES_ACTIVE: prod
      DATABASE_URL: postgres://shop@db:5432/shop
    restart: unless-stopped
`;

// ---- databases, web servers, users ----
const MB = 1024 * 1024;
const dbs: Record<string, any> = {
  postgres: { version: "16.4 (Ubuntu 16.4-1.pgdg24.04+1)", connections: 9, maxConnections: 100, error: null, databases: [
    { name: "postgres", sizeBytes: 7.6 * MB, connections: 1, tables: null, system: true },
    { name: "shop", sizeBytes: 1830 * MB, connections: 6, tables: null, system: false },
    { name: "shop_analytics", sizeBytes: 412 * MB, connections: 2, tables: null, system: false },
  ] },
  mysql: { version: "8.0.39-0ubuntu0.24.04.2", connections: 4, maxConnections: 151, error: null, databases: [
    { name: "information_schema", sizeBytes: 0, connections: 0, tables: 79, system: true },
    { name: "mysql", sizeBytes: 2.5 * MB, connections: 0, tables: 38, system: true },
    { name: "wordpress", sizeBytes: 96 * MB, connections: 3, tables: 12, system: false },
  ] },
};
let redisKeys = [{ db: 0, keys: 15230, expires: 4210 }, { db: 1, keys: 88, expires: 0 }];
const redis = () => ({ version: "7.2.4", role: "master", uptimeSecs: 3456000, connectedClients: 23, usedMemory: 182 * MB, maxMemory: 512 * MB, evictionPolicy: "allkeys-lru", hits: 9234112, misses: 412330, lastSave: Math.floor(Date.now() / 1000) - 1800, lastSaveOk: true, aofEnabled: false, keyspace: redisKeys, error: null });
let apacheSites = [{ name: "000-default", file: "/etc/apache2/sites-available/000-default.conf", enabled: true }, { name: "wiki", file: "/etc/apache2/sites-available/wiki.conf", enabled: true }, { name: "old-blog", file: "/etc/apache2/sites-available/old-blog.conf", enabled: false }];
const apache = () => ({ testOk: true, testOutput: "Syntax OK", serverRoot: "/etc/apache2", documentRoot: "/var/www/html", managesSites: true, sites: apacheSites,
  vhosts: [
    { serverName: "intranet.example.com", aliases: [], port: 8081, file: "/etc/apache2/sites-enabled/000-default.conf", line: 1 },
    { serverName: "wiki.example.com", aliases: ["docs.example.com"], port: 8081, file: "/etc/apache2/sites-enabled/wiki.conf", line: 1 },
  ],
  modules: ["alias", "auth_basic", "core", "deflate", "dir", "headers", "mime", "mpm_event", "php8.3", "proxy", "proxy_fcgi", "rewrite", "ssl"] });
const haState: Record<string, string> = { "web/web1": "UP", "web/web2": "UP", "web/web3": "MAINT", "api/api1": "UP", "api/api2": "DOWN" };
const haproxy = () => ({ configFile: "/etc/haproxy/haproxy.cfg", testOk: true, testOutput: "Configuration file is valid", socket: "/run/haproxy/admin.sock",
  sections: [
    { kind: "frontend", name: "www", line: 18, binds: ["*:80", "*:443"], mode: "http", balance: null, defaultBackend: "web", useBackends: ["api if { path_beg /api }"], servers: [], status: "OPEN" },
    { kind: "backend", name: "web", line: 26, binds: [], mode: "http", balance: "roundrobin", defaultBackend: null, useBackends: [], status: "UP", servers: ["web1", "web2", "web3"].map((n, i) => ({ name: n, address: `10.0.0.1${i + 1}:8080`, options: "check", status: haState[`web/${n}`], sessions: haState[`web/${n}`] === "UP" ? 12 - i * 3 : 0, check: haState[`web/${n}`] === "UP" ? "L7OK" : "" })) },
    { kind: "backend", name: "api", line: 33, binds: [], mode: "http", balance: "leastconn", defaultBackend: null, useBackends: [], status: "UP", servers: ["api1", "api2"].map((n, i) => ({ name: n, address: `10.0.0.2${i + 1}:8081`, options: "check inter 2s", status: haState[`api/${n}`], sessions: haState[`api/${n}`] === "UP" ? 7 : 0, check: haState[`api/${n}`] === "UP" ? "L7OK" : "L4CON" })) },
  ] });
const groups = [["sudo", 27, ["deploy"]], ["docker", 998, ["deploy", "anna"]], ["www-data", 33, ["deploy"]], ["adm", 4, ["syslog"]], ["systemd-journal", 101, []], ["deploy", 1000, []], ["anna", 1001, []], ["ci", 1002, []], ["users", 100, []], ["plugdev", 46, []]].map(([name, gid, members]) => ({ name, gid, members }));
let users: any[] = [
  { name: "root", uid: 0, gid: 0, primaryGroup: "root", groups: [], fullName: "root", home: "/root", shell: "/bin/bash", system: false, admin: true, passwordStatus: "L", passwordChanged: "2026-10-01", sshKeys: 1, canLogin: true },
  { name: "deploy", uid: 1000, gid: 1000, primaryGroup: "deploy", groups: ["sudo", "docker", "www-data"], fullName: "Deploy", home: "/home/deploy", shell: "/bin/bash", system: false, admin: true, passwordStatus: "P", passwordChanged: "2026-11-01", sshKeys: 2, canLogin: true },
  { name: "anna", uid: 1001, gid: 1001, primaryGroup: "anna", groups: ["docker"], fullName: "Anna Karimova", home: "/home/anna", shell: "/usr/bin/zsh", system: false, admin: false, passwordStatus: "L", passwordChanged: "2026-10-12", sshKeys: 1, canLogin: true },
  { name: "ci", uid: 1002, gid: 1002, primaryGroup: "ci", groups: [], fullName: "CI runner", home: "/home/ci", shell: "/bin/bash", system: false, admin: false, passwordStatus: "L", passwordChanged: "2026-09-02", sshKeys: 1, canLogin: true },
  { name: "www-data", uid: 33, gid: 33, primaryGroup: "www-data", groups: [], fullName: "www-data", home: "/var/www", shell: "/usr/sbin/nologin", system: true, admin: false, passwordStatus: "L", passwordChanged: null, sshKeys: null, canLogin: false },
];
const usersReport = () => ({ users, groups, shells: ["/bin/sh", "/bin/bash", "/usr/bin/zsh"], uidMin: 1000, adminGroup: "sudo", complete: true });

// ---- cron and resource usage ----
const iso = (d: Date) => d.toISOString().replace(/\.\d+Z$/, "+00:00");
const inMin = (m: number) => iso(new Date(Date.now() + m * 60000));
let cronJobs: any[] = [
  { source: "user", user: "deploy", file: null, line: 3, schedule: "0 3 * * *", command: "/usr/local/bin/backup.sh >> /var/log/backup.log 2>&1", note: "Nightly database backup", paused: false, description: "Every day at 03:00", nextRuns: [inMin(14 * 60), inMin(38 * 60)], lastRun: iso(new Date(Date.now() - 10 * 3600e3)).replace("+00:00", "+0000"), raw: "0 3 * * * /usr/local/bin/backup.sh >> /var/log/backup.log 2>&1" },
  { source: "user", user: "deploy", file: null, line: 4, schedule: "*/5 * * * *", command: "curl -fsS -m 10 https://hc.example.com/ping/shop", note: "Uptime ping", paused: false, description: "Every 5 minutes", nextRuns: [inMin(3), inMin(8)], lastRun: iso(new Date(Date.now() - 2 * 60e3)).replace("+00:00", "+0000"), raw: "*/5 * * * * curl -fsS -m 10 https://hc.example.com/ping/shop" },
  { source: "user", user: "deploy", file: null, line: 5, schedule: "30 2 * * 0", command: "cd /srv/shop && docker compose pull -q", note: null, paused: true, description: "Every Sunday at 02:30", nextRuns: [], lastRun: null, raw: "#orbital-paused# 30 2 * * 0 cd /srv/shop && docker compose pull -q" },
  { source: "user", user: "root", file: null, line: 1, schedule: "@reboot", command: "/opt/scripts/mount-volumes.sh", note: null, paused: false, description: "When the server starts", nextRuns: [], lastRun: null, raw: "@reboot /opt/scripts/mount-volumes.sh" },
  { source: "system", user: "root", file: "/etc/cron.d/certbot", line: 12, schedule: "0 */12 * * *", command: "test -x /usr/bin/certbot && perl -e 'sleep int(rand(43200))' && certbot -q renew", note: null, paused: false, description: "Every 12 hours, on the hour", nextRuns: [inMin(4 * 60)], lastRun: iso(new Date(Date.now() - 8 * 3600e3)).replace("+00:00", "+0000"), raw: "" },
  { source: "system", user: "root", file: "/etc/crontab", line: 17, schedule: "17 * * * *", command: "cd / && run-parts --report /etc/cron.hourly", note: null, paused: false, description: "Every hour at :17", nextRuns: [inMin(37)], lastRun: iso(new Date(Date.now() - 23 * 60e3)).replace("+00:00", "+0000"), raw: "" },
];
const cron = () => ({ jobs: cronJobs, serverNow: iso(new Date()), timezone: "UTC", complete: true,
  periodic: [{ period: "daily", path: "/etc/cron.daily/apt-compat" }, { period: "daily", path: "/etc/cron.daily/logrotate" }, { period: "weekly", path: "/etc/cron.weekly/man-db" }],
  timers: [
    { unit: "certbot.timer", activates: "certbot.service", calendar: "*-*-* 00,12:00:00", next: "Mon 2026-11-16 00:00:00 UTC", last: "Sun 2026-11-15 12:00:04 UTC", description: "Run certbot twice daily" },
    { unit: "apt-daily.timer", activates: "apt-daily.service", calendar: "*-*-* 6,18:00", next: "Sun 2026-11-15 18:22:31 UTC", last: "Sun 2026-11-15 06:41:10 UTC", description: "Daily apt download activities" },
    { unit: "logrotate.timer", activates: "logrotate.service", calendar: "daily", next: "Mon 2026-11-16 00:00:00 UTC", last: "Sun 2026-11-15 00:00:02 UTC", description: "Daily rotation of log files" },
  ] });
const describe = (s: string) => ({ "*/5 * * * *": "Every 5 minutes", "0 * * * *": "Every hour at :00", "0 3 * * *": "Every day at 03:00", "0 9 * * 1-5": "Every weekday at 09:00", "0 2 * * 0": "Every Sunday at 02:00", "0 4 1 * *": "Every month on day 1 at 04:00", "@reboot": "When the server starts" } as Record<string, string>)[s.trim()] ?? `Custom: ${s}`;
const GB = 1024 ** 3;
const disk: Record<string, [string, number, boolean][]> = {
  "/": [["var", 38.2 * GB, true], ["usr", 6.1 * GB, true], ["home", 4.4 * GB, true], ["srv", 3.2 * GB, true], ["opt", 1.1 * GB, true], ["swap.img", 2 * GB, false], ["boot", 0.2 * GB, true], ["etc", 0.01 * GB, true]],
  "/var": [["lib", 31.5 * GB, true], ["log", 5.8 * GB, true], ["cache", 0.7 * GB, true], ["backups", 0.2 * GB, true]],
  "/var/lib": [["docker", 26.9 * GB, true], ["postgresql", 3.6 * GB, true], ["apt", 0.6 * GB, true], ["redis", 0.3 * GB, true]],
  "/var/log": [["journal", 3.9 * GB, true], ["nginx", 1.4 * GB, true], ["syslog.1", 0.3 * GB, false], ["syslog", 0.12 * GB, false]],
};
const procs = [
  ["java", "deploy", 61.4, 1.8 * GB, "shop-api.service", 1], ["postgres", "postgres", 22.8, 1.2 * GB, "postgresql@16-main.service", 14], ["nginx", "www-data", 9.6, 0.11 * GB, "nginx.service", 9],
  ["dockerd", "root", 4.1, 0.16 * GB, "docker.service", 1], ["redis-server", "redis", 2.2, 0.19 * GB, "redis-server.service", 1], ["node", "deploy", 1.9, 0.42 * GB, "docker.service", 2],
  ["sshd", "root", 0.4, 0.02 * GB, "ssh.service", 3], ["systemd", "root", 0.1, 0.01 * GB, null, 1],
] as const;
const processUsage = () => {
  const jitter = () => 0.85 + Math.random() * 0.3;
  const processes = procs.map(([c, u, cpu, rss, unit], i) => ({ pid: 800 + i * 37, user: u, command: c, cpu: Math.round(cpu * jitter() * 10) / 10, memPercent: (rss / (16 * GB)) * 100, rssBytes: rss, unit }));
  const group = (k: (p: any) => string | null) => {
    const m: Record<string, any> = {};
    processes.forEach((p, i) => { const n = k(p); if (!n) return; m[n] ??= { name: n, processes: 0, cpu: 0, rssBytes: 0 }; m[n].processes += procs[i][5]; m[n].cpu += p.cpu; m[n].rssBytes += p.rssBytes; });
    return Object.values(m).sort((a: any, b: any) => b.cpu - a.cpu);
  };
  return { cores: 8, method: "sampled", processes, byProgram: group((p) => p.command), byService: group((p) => p.unit), byUser: group((p) => p.user) };
};

// ---- file storage (S3, WebDAV) ----
const storage: Record<string, [string, number, boolean][]> = {
  "/": [["media-prod", 0, true], ["backups", 0, true], ["site-assets", 0, true]],
  "/media-prod": [["uploads", 0, true], ["thumbs", 0, true], ["robots.txt", 64, false]],
  "/media-prod/uploads": [["2026", 0, true], ["cover.jpg", 2.4 * 1024 * 1024, false], ["hero.webp", 812 * 1024, false], ["manifest.json", 3120, false]],
  "/backups": [["shop-2026-11-14.dump", 305 * 1024 * 1024, false], ["shop-2026-11-15.dump", 309 * 1024 * 1024, false]],
};
const cloud: Record<string, [string, number, boolean][]> = {
  "/": [["Documents", 0, true], ["Photos", 0, true], ["Shared", 0, true], ["notes.md", 2048, false], ["Readme.md", 1200, false]],
  "/Documents": [["invoices", 0, true], ["contract.pdf", 412 * 1024, false], ["plan.md", 5400, false]],
};
const storageListing = (tree: Record<string, [string, number, boolean][]>, path: string) =>
  (tree[path] ?? []).map(([name, size, dir]) => ({ name, path: `${path === "/" ? "" : path}/${name}`, kind: dir ? "dir" : "file", size, mode: "", modeOctal: 0, mtime: dir ? null : now() - 3600 * 7, uid: null, gid: null, owner: null, group: null, hidden: name.startsWith(".") }));

// ---- tunnels, logs, deploys ----
let tunnels: any[] = [{ id: "tn1", sessionId: "s-p1", localPort: 5432, remoteHost: "127.0.0.1", remotePort: 5432, openConnections: 1, totalConnections: 6, bytes: 4.2 * 1024 * 1024, label: "PostgreSQL" }];
const LOG_LINES = [
  '203.0.113.50 - - "GET /api/products HTTP/2.0" 200 1832 "-" "Mozilla/5.0"',
  '198.51.100.7 - - "POST /api/cart HTTP/2.0" 201 412 "-" "Mozilla/5.0"',
  '[warn] 812#812: *4411 upstream response is buffered to a temporary file',
  '203.0.113.50 - - "GET /static/app.js HTTP/2.0" 304 0 "-" "Mozilla/5.0"',
  'shop-api: INFO  OrderService - order 18422 paid (12.40 EUR)',
  '192.0.2.14 - - "GET /admin HTTP/1.1" 404 153 "-" "curl/8.5"',
  'shop-api: ERROR PaymentClient - timeout after 5000 ms calling provider',
  '203.0.113.50 - - "GET /api/products/77 HTTP/2.0" 200 940 "-" "Mozilla/5.0"',
];
const logTimers: Record<string, number> = {};
let apps: any[] = [{ id: "app1", profileId: "p1", name: "shop", path: "/srv/shop", branch: "main", build: "npm ci && npm run build", restartKind: "service", restartTarget: "shop-api.service", createdAt: "2026-10-02T09:00:00Z" }];
const commits = [
  { id: "9f2c1e7a", short: "9f2c1e7", author: "Aziza", when: "3 hours ago", subject: "Checkout: retry the payment provider once" },
  { id: "4be81d02", short: "4be81d0", author: "Sirojiddin", when: "19 hours ago", subject: "Product page: lazy-load images" },
  { id: "c07a9b13", short: "c07a9b1", author: "Aziza", when: "2 days ago", subject: "Bump dependencies" },
];
let head = "4be81d02";
let deploys: any[] = [{ id: "d1", appId: "app1", kind: "deploy", startedAt: "2026-11-14T17:45:00Z", finishedAt: "2026-11-14T17:46:10Z", fromCommit: "c07a9b13", toCommit: "4be81d02", outcome: "success", summary: "c07a9b1 → 4be81d0" }];

let termCb: ((m: any) => void) | null = null;
let line = "";
const b64 = (s: string) => btoa(unescape(encodeURIComponent(s)));
const prompt = "\x1b[1;33mdeploy@prod-web-01\x1b[0m:\x1b[1;34m/srv/shop\x1b[0m$ ";

// What `ssh_config_preview` returns for a realistic ~/.ssh/config (see ssh_config.rs tests).
const note = (text: string, level: "info" | "warn" = "info") => ({ level, text });
const cand = (c: any) => ({ aliases: [], authMethod: c.keyPath ? "key" : "agent", keyPath: null, jumpKey: null, jumpOnly: false, importable: true, source: null, notes: [], existingId: null, existingName: null, port: 22, ...c });
const sshImport = () => withSaved({
  path: "~/.ssh/config",
  found: true,
  notes: [
    note("Host *.prod !bastion.prod (~/.ssh/config:31) is a pattern, not a server: its settings are applied to the hosts it matches"),
    note('Match host *.corp exec "test -f ~/.vpn" (~/.ssh/config:40) was skipped: Match conditions can\'t be checked before connecting', "warn"),
  ],
  candidates: [
    cand({ key: "sirojiddin@staging.example.dev:2222", name: "staging", host: "staging.example.dev", port: 2222, username: "sirojiddin", source: "~/.ssh/config.d/10-staging:1", notes: [note("No IdentityFile: signs in with ssh-agent")] }),
    cand({ key: "ops@bastion.example.com:2222", name: "bastion", host: "bastion.example.com", port: 2222, username: "ops", keyPath: "~/.ssh/id_ed25519", source: "~/.ssh/config:5" }),
    cand({ key: "deploy@203.0.113.10:22", name: "web", aliases: ["web.example.com"], host: "203.0.113.10", username: "deploy", keyPath: "~/.ssh/id_ed25519", source: "~/.ssh/config:11", notes: [note("Also listed as web.example.com")], }),
    cand({ key: "postgres@10.0.4.30:22 via admin@10.0.0.5:2200 via ops@bastion.example.com:2222", name: "db-replica", host: "10.0.4.30", username: "postgres", keyPath: "~/.ssh/keys/10.0.4.30_postgres", jumpKey: "admin@10.0.0.5:2200 via ops@bastion.example.com:2222", source: "~/.ssh/config:15", notes: [note("2 identity files are listed; Orbital uses the first")] }),
    cand({ key: "deploy@app.prod:22 via ops@bastion.example.com:2222", name: "app.prod", host: "app.prod", username: "deploy", keyPath: "~/.ssh/id_ed25519", jumpKey: "ops@bastion.example.com:2222", source: "~/.ssh/config:34", notes: [note("Port forwards aren't imported; open them from the Tunnels tab")] }),
    cand({ key: "sirojiddin@legacy.internal:22 via ops@bastion.example.com:2222", name: "legacy", host: "legacy.internal", username: "sirojiddin", jumpKey: "ops@bastion.example.com:2222", source: "~/.ssh/config:21", notes: [note("ProxyCommand (~/.ssh/config:23) is read as jump host bastion"), note("No IdentityFile: signs in with ssh-agent")] }),
    cand({ key: "sirojiddin@weird.internal:22", name: "weird", host: "weird.internal", username: "sirojiddin", source: "~/.ssh/config:25", notes: [note("ProxyCommand (~/.ssh/config:27) isn't supported: this server is imported without it. Set a jump host after importing if it needs one", "warn")] }),
    cand({ key: "!loop-a", name: "loop-a", host: "loop-a", username: "sirojiddin", importable: false, source: "~/.ssh/config:44", notes: [note("ProxyJump loop: loop-a → loop-b → loop-a", "warn")] }),
    cand({ key: "admin@10.0.0.5:2200 via ops@bastion.example.com:2222", name: "admin@10.0.0.5:2200", host: "10.0.0.5", port: 2200, username: "admin", jumpKey: "ops@bastion.example.com:2222", jumpOnly: true, notes: [note("No IdentityFile: signs in with ssh-agent")] }),
  ],
});
/** Mark candidates whose host, port and user match a saved profile, as the engine does. */
function withSaved<T extends { candidates: any[] }>(pv: T): T {
  for (const c of pv.candidates) {
    const hit = profiles.find((p) => p.protocol === "sftp" && p.host === c.host && p.port === c.port && p.username === c.username);
    if (hit) { c.existingId = hit.id; c.existingName = hit.name; }
  }
  return pv;
}
const importSsh = (keys: string[]) => {
  const all = sshImport().candidates;
  const byKey = new Map(all.map((c) => [c.key, c]));
  const ids = new Map<string, string>();
  const created: any[] = [];
  const failed: any[] = [];
  const make = (c: any): string | null => {
    if (c.existingId) return c.existingId;
    if (ids.has(c.key)) return ids.get(c.key)!;
    if (!c.importable) { failed.push({ name: c.name, error: c.notes[0]?.text ?? "can't be imported" }); return null; }
    const jump = c.jumpKey ? make(byKey.get(c.jumpKey)) : null;
    if (c.jumpKey && !jump) return null;
    const p = { id: `imp${profiles.length}`, name: c.name, host: c.host, port: c.port, username: c.username, authMethod: c.authMethod, keyPath: c.keyPath, sudoMode: "auto", groupName: null, color: null, initialPath: null, jumpProfileId: jump, protocol: "sftp", region: null, pathStyle: false, createdAt: new Date().toISOString(), lastConnectedAt: null };
    profiles.push(p);
    created.push({ ...p });
    ids.set(c.key, p.id);
    return p.id;
  };
  for (const k of keys) { const c = byKey.get(k); if (c) make(c); }
  return { created, failed };
};

// ---- uptime checks ----
const MIN = 60_000;
type MockCheck = { id: string; name: string; kind: "http" | "tcp"; target: string; expectedStatus: number; contains: string | null; intervalMinutes: number; paused: boolean; createdAt: string };
let uptimeChecks: MockCheck[] = [
  { id: "u1", name: "Shop", kind: "http", target: "https://shop.example.com/", expectedStatus: 200, contains: "Add to cart", intervalMinutes: 1, paused: false, createdAt: "2026-11-01T10:00:00Z" },
  { id: "u2", name: "Shop API health", kind: "http", target: "https://shop.example.com/api/health", expectedStatus: 200, contains: null, intervalMinutes: 1, paused: false, createdAt: "2026-11-01T10:01:00Z" },
  { id: "u3", name: "grafana.example.com", kind: "http", target: "https://grafana.example.com/", expectedStatus: 200, contains: null, intervalMinutes: 5, paused: false, createdAt: "2026-11-02T10:00:00Z" },
  { id: "u4", name: "Status page", kind: "http", target: "https://status.example.org/", expectedStatus: 200, contains: null, intervalMinutes: 15, paused: false, createdAt: "2026-11-03T10:00:00Z" },
  { id: "u5", name: "PostgreSQL (db-primary)", kind: "tcp", target: "10.0.4.21:5432", expectedStatus: 0, contains: null, intervalMinutes: 5, paused: false, createdAt: "2026-11-04T10:00:00Z" },
  { id: "u6", name: "Old landing page", kind: "http", target: "http://promo.example.com/", expectedStatus: 200, contains: null, intervalMinutes: 15, paused: true, createdAt: "2026-11-05T10:00:00Z" },
];
const certIn = (days: number) => new Date(Date.now() + days * 86400_000).toISOString();
const CERTS: Record<string, string | null> = { u1: certIn(61), u2: certIn(61), u3: certIn(9), u4: certIn(44) };
let uptimeResults: Record<string, any[]> = {};
let uptimeChanges: Record<string, any[]> = {};
const fakeResult = (c: MockCheck, at: number, up: boolean, error?: string) => {
  const base = c.kind === "tcp" ? 4 : c.id === "u4" ? 180 : 60;
  const cert = CERTS[c.id] ?? null;
  const days = cert ? (new Date(cert).getTime() - Date.now()) / 86400_000 : 99;
  return {
    at: new Date(at).toISOString(), up, latencyMs: up ? Math.round(base + Math.random() * base * 0.6 + (Math.sin(at / 9e5) + 1) * base * 0.3) : null,
    statusCode: c.kind === "http" ? (up ? c.expectedStatus : error?.startsWith("got 502") ? 502 : null) : null,
    error: up ? null : error ?? "timed out", certExpires: cert, warning: up && days <= 14 ? `the TLS certificate expires in ${Math.floor(days)} days` : null,
  };
};
for (const c of uptimeChecks) {
  const step = c.intervalMinutes * MIN;
  const list: any[] = [];
  for (let i = 59; i >= 0; i--) {
    const at = Date.now() - i * step - 20_000;
    // The API had a short outage about forty minutes ago; the status page was down for one check.
    const down = (c.id === "u2" && i >= 38 && i <= 41) || (c.id === "u4" && i === 30);
    list.push(fakeResult(c, at, !down, c.id === "u2" ? "got 502 Bad Gateway, expected 200 OK" : "timed out"));
  }
  uptimeResults[c.id] = list;
  uptimeChanges[c.id] = [];
  for (let i = 1; i < list.length; i++) if (list[i].up !== list[i - 1].up) uptimeChanges[c.id].unshift({ at: list[i].at, up: list[i].up, error: list[i].error });
}
const uptimeStatus = (c: MockCheck) => {
  const recent = uptimeResults[c.id] ?? [];
  const last = recent[recent.length - 1] ?? null;
  const share = (ms: number) => { const r = recent.filter((x) => Date.now() - new Date(x.at).getTime() <= ms); return r.length ? (r.filter((x) => x.up).length * 100) / r.length : null; };
  const lastErr = [...recent].reverse().find((r) => !r.up);
  return {
    check: { ...c }, state: c.paused || !last ? "unknown" : last.up ? "up" : "down", last, recent,
    uptime24h: share(86400_000), uptime7d: share(7 * 86400_000) === null ? null : Math.min(100, (share(7 * 86400_000) ?? 100) - (c.id === "u2" ? 0.12 : 0)),
    lastError: lastErr ? { at: lastErr.at, up: false, error: lastErr.error } : null,
    since: uptimeChanges[c.id]?.[0]?.at ?? null, changes: uptimeChanges[c.id] ?? [],
  };
};
let uptimeCh: any = null;
const uptimeRun = (c: MockCheck, up = true, error?: string) => {
  const r = fakeResult(c, Date.now(), up, error);
  const list = (uptimeResults[c.id] ??= []);
  const prev = list[list.length - 1];
  list.push(r);
  if (list.length > 60) list.shift();
  if ((prev ? prev.up : true) !== up) {
    (uptimeChanges[c.id] ??= []).unshift({ at: r.at, up, error: r.error });
    audit.unshift({ ...audit[0], seq: audit[0].seq + 1, id: `u${Date.now()}`, ts: r.at, profileId: null, server: null, category: "uptime", action: up ? "uptime.up" : "uptime.down", target: c.name === c.target ? c.target : `${c.name} (${c.target})`, command: null, outcome: up ? "success" : "failure", exitCode: null, details: { error: r.error } } as any);
    uptimeCh?.onmessage({ type: "changed", checkId: c.id, name: c.name, target: c.target, change: up ? "recovered" : "wentDown", error: r.error });
  }
  uptimeCh?.onmessage({ type: "result", checkId: c.id, result: r });
  return r;
};

export function install() {
  mockIPC(async (cmd, args: any) => {
    switch (cmd) {
      case "list_profiles": return profiles.map((p) => ({ ...p }));
      case "ssh_config_preview": return args.path ? { ...sshImport(), path: args.path.replace(/^\/Users\/[^/]+/, "~"), candidates: sshImport().candidates.slice(0, 2) } : sshImport();
      case "import_ssh_config": return importSsh(args.keys);
      case "update_profile": {
        const i = profiles.findIndex((p) => p.id === args.id);
        Object.assign(profiles[i], args.input);
        return { ...profiles[i] };
      }
      case "set_secret": return null;
      case "has_secret": return true;
      case "presence_status": return presence;
      case "set_presence_gate": presence.enabled = args.enabled; return { ...presence };
      case "connect": {
        if (profiles.find((x) => x.id === args.profileId)?.protocol === "sftp" && !trusted.has(args.profileId) && !args.opts?.acceptFingerprint)
          throw { kind: "UnknownHostKey", message: "unknown", data: { host: "staging.example.dev", port: 2222, algorithm: "ssh-ed25519", fingerprint: "SHA256:kq3W0dF7vYV1oR6nZQh2xJmUe8pT5cB4sLaGi9NwEo0" } };
        trusted.add(args.profileId);
        const p = profiles.find((x) => x.id === args.profileId)!;
        if (p.protocol !== "sftp")
          return { id: "s-" + p.id, profileId: p.id, label: `${p.protocol === "s3" ? "s3" : "https"}://${p.username}@${p.host}`, home: "/", hasSystemd: false, dockerVersion: null, elevation: false, via: [], connectedAt: new Date().toISOString(), protocol: p.protocol };
        return { protocol: "sftp", id: "s-" + p.id, profileId: p.id, label: `${p.username}@${p.host}`, home: "/home/deploy", hasSystemd: true, dockerVersion: "27.3.1", elevation: true, via: p.jumpProfileId ? ["deploy@203.0.113.10:22"] : [], connectedAt: new Date().toISOString() };
      }
      case "list_dir": return args.sessionId === "s-p4" ? storageListing(storage, args.path) : args.sessionId === "s-p5" ? storageListing(cloud, args.path) : listing(args.path);
      case "list_tunnels": return tunnels.map((t) => ({ ...t }));
      case "open_tunnel": {
        const t = { id: "tn" + (tunnels.length + 2), sessionId: args.sessionId, localPort: args.localPort ?? (args.remotePort >= 1024 ? args.remotePort : 40000 + args.remotePort), remoteHost: args.remoteHost, remotePort: args.remotePort, openConnections: 0, totalConnections: 0, bytes: 0, label: args.label ?? null };
        tunnels = [...tunnels, t];
        return t;
      }
      case "close_tunnel": tunnels = tunnels.filter((t) => t.id !== args.id); return null;
      case "log_files": return [{ path: "/var/log/syslog", size: 0.12 * GB }, { path: "/var/log/auth.log", size: 2.1 * MB }, { path: "/var/log/nginx/access.log", size: 48 * MB }, { path: "/var/log/nginx/error.log", size: 1.2 * MB }, { path: "/var/log/postgresql/postgresql-16-main.log", size: 6.4 * MB }];
      case "follow_logs": {
        const id = "lg" + Math.random().toString(36).slice(2, 8);
        const send = (n: number) => args.onEvent.onmessage({ type: "data", text: Array.from({ length: n }, () => `2026-11-15T${new Date().toTimeString().slice(0, 8)} ${LOG_LINES[Math.floor(Math.random() * LOG_LINES.length)]}`).join("\n") + "\n" });
        if (args.lines > 0) setTimeout(() => send(Math.min(40, args.lines)), 100);
        logTimers[id] = window.setInterval(() => send(1 + Math.floor(Math.random() * 3)), 900);
        return id;
      }
      case "stop_logs": clearInterval(logTimers[args.id]); delete logTimers[args.id]; return null;
      case "list_apps": return apps.filter((a) => a.profileId === args.profileId);
      case "save_app": {
        if (args.id) { apps = apps.map((a) => (a.id === args.id ? { ...a, ...args.input } : a)); return apps.find((a) => a.id === args.id); }
        const a = { ...args.input, id: "app" + (apps.length + 2), profileId: args.profileId, createdAt: new Date().toISOString() };
        apps = [...apps, a];
        return a;
      }
      case "delete_app": apps = apps.filter((a) => a.id !== args.id); return null;
      case "app_checkout": return { head, branch: "main", remote: "git@github.com:example/shop.git", dirty: [], recent: commits };
      case "deploy_history": return deploys.filter((d) => d.appId === args.appId);
      case "deploy_log": return "$ git fetch --prune origin main\n$ git merge --ff-only FETCH_HEAD\nUpdating c07a9b1..4be81d0\nFast-forward\n$ npm ci && npm run build\nadded 812 packages in 21s\n✓ built in 9.4s\n$ systemctl restart shop-api.service\n";
      case "run_deploy": {
        const ev = (m: any) => args.onEvent.onmessage(m);
        const target = args.rollbackTo ?? commits[0].id;
        const steps: [string, string[]][] = args.rollbackTo
          ? [["Check out " + target.slice(0, 7), [`HEAD is now at ${target.slice(0, 7)}`]], ["Build", ["added 812 packages in 19s", "✓ built in 9.1s"]], ["Restart shop-api.service", ["shop-api.service is active"]]]
          : [["Fetch main", ["From github.com:example/shop", ` * branch            main       -> FETCH_HEAD`]], ["Update to the latest commit", [`Updating ${head.slice(0, 7)}..${target.slice(0, 7)}`, "Fast-forward", " src/payments/client.ts | 14 +++++++++-----"]], ["Build", ["added 812 packages in 21s", "vite v6.0.3 building for production...", "✓ built in 9.4s"]], ["Restart shop-api.service", ["shop-api.service is active"]]];
        return new Promise((resolve) => {
          let t = 0;
          steps.forEach(([name, out]) => {
            setTimeout(() => ev({ type: "step", name }), (t += 300));
            out.forEach((o) => setTimeout(() => ev({ type: "output", text: o + "\n" }), (t += 350)));
            setTimeout(() => ev({ type: "stepEnd", ok: true, code: 0 }), (t += 250));
          });
          setTimeout(() => {
            const rec = { id: "d" + (deploys.length + 2), appId: args.appId, kind: args.rollbackTo ? "rollback" : "deploy", startedAt: new Date().toISOString(), finishedAt: new Date().toISOString(), fromCommit: head, toCommit: target, outcome: "success", summary: `${head.slice(0, 7)} → ${target.slice(0, 7)}` };
            head = target;
            deploys = [rec, ...deploys];
            ev({ type: "done", ok: true, record: rec });
            resolve(rec);
          }, (t += 300));
        });
      }
      case "read_file": return { path: args.path, content: args.path.endsWith("nginx.conf") ? NGINX : args.path.endsWith("docker-compose.yml") ? COMPOSE : "# demo file\n", size: 1482, mtime: now() - 9000, modeOctal: 0o644, writable: !args.path.startsWith("/etc") };
      case "validator_for": return args.path.startsWith("/etc/nginx/") ? { name: "nginx", command: "nginx -t", stage: "postInstall" } : null;
      case "list_backups": return args.path.endsWith("nginx.conf") ? [`${args.path}.orbital-bak-20261117081205`, `${args.path}.orbital-bak-20261114190033`, `${args.path}.orbital-bak-20261102101500`] : [];
      case "read_backup": {
        await new Promise((r) => setTimeout(r, 250));
        if (args.backup.endsWith("20261117081205")) return NGINX.replace("worker_connections 2048", "worker_connections 1024").replace("rate=20r/s", "rate=10r/s");
        if (args.backup.endsWith("20261114190033")) return NGINX.replace("    server_tokens off;\n", "").replace("max_fails=3", "max_fails=1");
        return NGINX.replace(/\n    # Rate limit the public API[\s\S]*?zone=api:10m rate=20r\/s;\n/, "\n");
      }
      case "restore_backup": await new Promise((r) => setTimeout(r, 700)); return { path: args.path, bytes: 1400, privileged: args.privileged, backupPath: args.path + ".orbital-bak-20261117100000", validator: "nginx", validatorOutput: "syntax is ok\ntest is successful", newMtime: now(), suggestedReload: "nginx.service" };
      case "safe_write": return { path: args.path, bytes: args.content.length, privileged: args.privileged, backupPath: args.path + ".orbital-bak-20261115011500", validator: "nginx", validatorOutput: "syntax is ok", newMtime: now(), suggestedReload: "nginx.service" };
      case "write_file": return now();
      case "metrics": return metric();
      case "detect_tools": return tools;
      case "nginx_config": return structuredClone(nginxConfig);
      case "website_plan": await new Promise((r) => setTimeout(r, 250)); return websitePlan(structuredClone(args.input));
      case "create_website":
        await progress(args.progress, [
          [0, "step", `Checking nginx's sites for ${args.input.domain} …`],
          [700, "step", `Creating /var/www/${args.input.domain} …`],
          [600, "step", `Writing /etc/nginx/sites-available/${args.input.domain} and testing it with nginx -t …`],
          [1100, "step", "Reloading nginx …"],
          [500, "line", ""],
        ]);
        return createWebsite(structuredClone(args.input));
      case "listening_ports": return ports;
      case "cron_report": return cron();
      case "cron_preview": return /^(@reboot|[0-9*/,\-a-z]+( [0-9*/,\-a-z]+){4})$/i.test(args.schedule.trim()) ? { description: describe(args.schedule), nextRuns: [inMin(60), inMin(24 * 60)], error: null } : { description: "", nextRuns: [], error: "a schedule has five fields: minute hour day-of-month month day-of-week (for example 0 3 * * *)" };
      case "cron_change": {
        const c = args.change;
        if (c.type === "add") cronJobs = [...cronJobs, { source: "user", user: args.user, file: null, line: 9, schedule: c.schedule, command: c.command, note: c.note, paused: false, description: describe(c.schedule), nextRuns: [inMin(60)], lastRun: null, raw: `${c.schedule} ${c.command}` }];
        else if (c.type === "delete") cronJobs = cronJobs.filter((j) => j.raw !== c.raw);
        else cronJobs = cronJobs.map((j) => j.raw !== c.raw ? j : c.type === "pause" ? { ...j, paused: true, nextRuns: [], raw: `#orbital-paused# ${j.raw}` } : c.type === "resume" ? { ...j, paused: false, nextRuns: [inMin(30)], raw: j.raw.replace("#orbital-paused# ", "") } : { ...j, schedule: c.schedule, command: c.command, note: c.note, description: describe(c.schedule), raw: `${c.schedule} ${c.command}` });
        return cron();
      }
      case "cron_run": return new Promise((r) => setTimeout(() => r({ stdout: args.command.includes("backup") ? "Dumping shop… done (1.8 GB → 305 MB)\nUploading to s3://backups/shop/2026-11-15.dump… done\n" : "OK\n", stderr: "", exitCode: 0, durationMs: 1800 }), 900));
      case "disk_usage": {
        const e = (disk[args.path] ?? [["data", 0.4 * GB, true], ["cache", 0.1 * GB, true]]).map(([name, bytes, isDir]) => ({ name, bytes, isDir, path: `${args.path === "/" ? "" : args.path}/${name}` }));
        return new Promise((r) => setTimeout(() => r({ path: args.path, total: e.reduce((n, x) => n + x.bytes, 0) * 1.02, entries: e, fsSize: 80 * GB, fsUsed: 58.4 * GB, fsFree: 21.6 * GB, mount: "/", partial: false }), 700));
      }
      case "process_usage": return processUsage();
      case "list_databases":
        if (args.container) {
          await new Promise((r) => setTimeout(r, 300));
          return { version: "7.0.12", connections: 6, maxConnections: 838860, error: null, databases: [{ name: "admin", sizeBytes: 40960, connections: null, tables: 1, system: true }, { name: "events", sizeBytes: 1.4 * 1024 * MB, connections: null, tables: 9, system: false }] };
        }
        return dbs[args.kind] ?? { version: "7.0.12", connections: 4, maxConnections: 838860, error: null, databases: [{ name: "admin", sizeBytes: 40960, connections: null, tables: 1, system: true }, { name: "app", sizeBytes: 220 * MB, connections: null, tables: 14, system: false }] };
      case "backup_database": {
        const n = Math.round(dbs[args.kind]?.databases.find((d: any) => d.name === args.name)?.sizeBytes / 6 || 4 * MB);
        await progress(args.progress, [[0, "step", `Dumping ${args.name} on the server (large databases take a while) …`], [2600, "step", "Downloading the dump to this computer …"], ...transferEvents(1, n, [`/tmp/orbital-backup/${args.name}.dump`], 1800)]);
        return n;
      }
      case "notify": return null;
      case "find_files": {
        await new Promise((r) => setTimeout(r, 500));
        const q = String(args.query).toLowerCase();
        const all = ["/home/deploy/apps", "/home/deploy/apps/shop/docker-compose.yml", "/home/deploy/apps/shop/.env.example", "/home/deploy/apps/shop/nginx.conf", "/home/deploy/apps/shop/config", "/home/deploy/apps/shop/config/app.yml", "/home/deploy/backup.sh", "/home/deploy/docker-compose.yml", "/home/deploy/apps/blog/config.toml"];
        const dirs = new Set(["/home/deploy/apps", "/home/deploy/apps/shop/config"]);
        const files = all.filter((p) => p.startsWith(args.dir === "/" ? "/" : args.dir + "/") && p.split("/").pop()!.toLowerCase().includes(q)).map((p) => ({ path: p, dir: dirs.has(p) }));
        files.sort((a, b) => a.path.split("/").length - b.path.split("/").length || a.path.localeCompare(b.path));
        return { files, truncated: false };
      }
      case "download_folder":
        await progress(args.progress, [[0, "step", `Packing ${args.remotePath} on the server and downloading it …`], ...transferEvents(1, 38_400_000, [args.remotePath.split("/").pop()], 3600)]);
        return 38_400_000;
      case "cancel_job": stopped.add(args.id); return true;
      case "agent_status": return { ...agent, recent: agentLog.map((l) => JSON.stringify(l)) };
      case "agent_check":
        await new Promise((r) => setTimeout(r, 700));
        if (!/^\d+:[\w-]{30,}$/.test(args.setup.telegramToken)) throw { kind: "InvalidInput", message: "Invalid input: Telegram doesn't accept this bot token: copy it again from @BotFather" };
        if (args.setup.apiKey.startsWith("sk-ant-oat")) throw { kind: "InvalidInput", message: "Invalid input: this is a Claude subscription (OAuth) token, like the one Claude Code uses, not an API key. Create an API key at console.anthropic.com → API keys (it starts with sk-ant-api) and paste that" };
        if (args.setup.apiKey.includes("bad")) throw { kind: "Other", message: "the AI provider rejected the request (400): Your credit balance is too low to access the Anthropic API. Please go to Plans & Billing to upgrade or purchase credits." };
        return args.setup.provider === "openai"
          ? { bot: "my_server_helper_bot", models: [{ id: "gpt-5", name: "gpt-5" }, { id: "gpt-5-mini", name: "gpt-5-mini" }, { id: "o3", name: "o3" }] }
          : { bot: "my_server_helper_bot", models: [{ id: "claude-opus-5-5", name: "Claude Opus 5.5" }, { id: "claude-sonnet-5-5", name: "Claude Sonnet 5.5" }, { id: "claude-haiku-5-5", name: "Claude Haiku 5.5" }, { id: "claude-fable-5-2", name: "Claude Fable 5.2" }] };
      case "agent_install": {
        await progress(args.progress, [[0, "step", "Checking the bot token and the API key …"], [700, "step", "Setting up the agent on the server (the first time takes a minute or two) …"], [2600, "step", "Installing the agent …"], [500, "step", "Saving your settings …"], [500, "step", "Starting the agent …"], [600, "step", "Making a pairing code …"]]);
        Object.assign(agent, { installed: true, running: true, version: "1.3.0", updateAvailable: true, provider: args.setup.provider, model: args.setup.model || (args.setup.provider === "openai" ? "gpt-5" : "claude-opus-5-5"), bot: "my_server_helper_bot", alerts: args.setup.alerts, owner: null, pairedAt: null, pairCodeActive: true });
        agentLog.push({ time: new Date().toISOString().slice(0, 19), event: "start", version: "1.3.1" });
        return agentCode();
      }
      case "agent_pair_code": Object.assign(agent, { owner: null, pairedAt: null, pairCodeActive: true }); return agentCode();
      case "agent_update": await progress(args.progress, [[0, "step", "Updating the agent …"], [1700, "step", "Restarting it …"]]); agent.version = "1.3.1"; agent.updateAvailable = false; return null;
      case "agent_set_running": agent.running = args.on; return null;
      case "agent_set_alerts": agent.alerts = args.on; return null;
      case "agent_models":
        await new Promise((r) => setTimeout(r, 600));
        if (agent.version !== "1.3.1") throw { kind: "InvalidInput", message: "Invalid input: this agent is too old to list models: click Update agent first" };
        return agent.provider === "openai" ? [{ id: "gpt-5", name: "gpt-5" }, { id: "gpt-5-mini", name: "gpt-5-mini" }, { id: "o3", name: "o3" }] : [{ id: "claude-opus-5-5", name: "Claude Opus 5.5" }, { id: "claude-sonnet-5-5", name: "Claude Sonnet 5.5" }, { id: "claude-haiku-5-5", name: "Claude Haiku 5.5" }];
      case "agent_set_model": agent.model = args.model; agentLog.push({ time: new Date().toISOString().slice(0, 19), event: "config", provider: agent.provider, model: args.model }); return null;
      case "agent_test": await new Promise((r) => setTimeout(r, 900)); if (!agent.owner) throw { kind: "Other", message: "not paired yet: make a pairing code and send it to the bot first" }; agentLog.push({ time: new Date().toISOString().slice(0, 19), event: "test_message" }); return agent.owner;
      case "agent_set_daily": agent.dailyReport = args.time; return null;
      case "agent_remove": Object.assign(agent, { installed: false, running: false, owner: null }); agentLog.length = 0; return null;
      case "tmux_sessions": return [{ name: "orbital-1", windows: 2, attached: false, created: Math.floor(Date.now() / 1000) - 3 * 3600 }, { name: "deploy", windows: 1, attached: false, created: Math.floor(Date.now() / 1000) - 86400 }];
      case "cleanup_scan": return cleanupItems();
      case "cleanup_run":
        await progress(args.progress, [[0, "step", "Freeing space …"], [700, "line", args.id === "journal" ? "Vacuuming done, freed 612.0M of archived journals from /var/log/journal/3f2a." : args.id.startsWith("docker") ? "Total reclaimed space: 1.9GB" : "Done."], [600, "step", "Measuring again …"]]);
        cleaned.add(args.id);
        return cleanupItems();
      case "stop_process": if (args.pid <= 1) throw { kind: "InvalidInput", message: "Invalid input: PID 1 is the system itself" }; return null;
      case "updates": return updatesReport();
      case "refresh_updates":
        await progress(args.progress, [[0, "step", "Asking the package mirrors for new versions …"], [400, "line", "Hit:1 http://archive.ubuntu.com/ubuntu noble InRelease"], [700, "line", "Get:2 http://security.ubuntu.com/ubuntu noble-security InRelease [126 kB]"], [900, "line", "Get:3 http://archive.ubuntu.com/ubuntu noble-updates InRelease [126 kB]"], [1200, "line", "Fetched 1,822 kB in 2s (912 kB/s)"], [500, "line", "Reading package lists..."], [400, "step", "Listing what can be upgraded …"]]);
        return updatesReport();
      case "install_updates": {
        const lines = updatesReport().packages.flatMap((p: any) => [[500, "line", `Unpacking ${p.name} (${p.available}) over (${p.current}) ...`], [350, "line", `Setting up ${p.name} (${p.available}) ...`]] as [number, "line", string][]);
        await progress(args.progress, [[0, "step", "Installing updates (this can take several minutes) …"], [600, "line", "Reading package lists..."], [500, "line", `${updatesReport().packages.length} upgraded, 0 newly installed, 0 to remove and 0 not upgraded.`], ...lines, [600, "line", "Processing triggers for libc-bin (2.39-0ubuntu8.3) ..."]]);
        upgraded = true;
        return { stdout: "Reading package lists...\nAll packages upgraded.\n", stderr: "", exitCode: 0, durationMs: 9000 };
      }
      case "reboot_server": return null;
      case "shell_history":
        return ["docker compose logs -f --tail=100", "sudo systemctl restart nginx", "sudo nginx -t", "git pull", "tail -f /var/log/nginx/error.log", "systemctl status shop", "df -h", "cd /srv/shop", "ls -la", "htop", "sudo journalctl -u shop -n 50 --no-pager", "npm ci", "docker ps"];
      case "redis_info": return args.container ? { ...redis(), version: "7.4.1", usedMemory: 64 * MB, connectedClients: 4, keyspace: [{ db: 0, keys: 1832, expires: 1790 }] } : redis();
      case "redis_action": if (args.action === "flushAll") redisKeys = []; else if (args.action?.flushDb !== undefined) redisKeys = redisKeys.filter((k) => k.db !== args.action.flushDb); return redis();
      case "apache_status": return apache();
      case "apache_site": apacheSites = apacheSites.map((x) => (x.name === args.name ? { ...x, enabled: args.enable } : x)); return apache();
      case "haproxy_status": return haproxy();
      case "haproxy_server_state": haState[`${args.backend}/${args.server}`] = args.state === "ready" ? "UP" : args.state === "drain" ? "DRAIN" : "MAINT"; return haproxy();
      case "list_users": return usersReport();
      case "create_user": {
        const u = args.user;
        const uid = u.uid ?? Math.max(...users.map((x) => (x.uid < 60000 ? x.uid : 0))) + 1;
        users = [...users, { name: u.name, uid, gid: uid, primaryGroup: u.primaryGroup ?? u.name, groups: u.groups, fullName: u.fullName ?? "", home: u.home ?? `/home/${u.name}`, shell: u.shell ?? "/bin/sh", system: u.system, admin: u.groups.includes("sudo"), passwordStatus: args.password ? "P" : "L", passwordChanged: "2026-11-15", sshKeys: u.sshKeys.length || null, canLogin: true }];
        return usersReport();
      }
      case "change_user": {
        const c = args.change;
        if (args.name === "deploy" && !args.force && (c.type === "lock" || c.type === "delete" || (c.type === "groups" && !c.groups.includes("sudo"))))
          throw { kind: "PreflightFailed", message: "Pre-flight check failed", data: { check: "your access", output: c.type === "groups" ? "deploy would leave the sudo group and lose sudo, so Orbital couldn't change the server any more through this account." : "deploy is the account this connection uses." } };
        users = c.type === "delete" ? users.filter((u) => u.name !== args.name) : users.map((u) => u.name !== args.name ? u : {
          ...u,
          ...(c.type === "groups" ? { groups: c.groups, admin: c.groups.includes("sudo") } : {}),
          ...(c.type === "shell" ? { shell: c.shell } : {}),
          ...(c.type === "lock" ? { passwordStatus: "L", sshKeys: null } : {}),
          ...(c.type === "unlock" ? { passwordStatus: "P" } : {}),
          ...(c.type === "password" ? { passwordStatus: "P", passwordChanged: "2026-11-15" } : {}),
          ...(c.type === "addKey" ? { sshKeys: (u.sshKeys ?? 0) + 1 } : {}),
        });
        return usersReport();
      }
      case "system_info": return { hostname: "prod-web-01", fqdn: "prod-web-01.example.com", os: "Ubuntu 24.04.1 LTS", kernel: "6.8.0-45-generic", arch: "x86_64", virtualization: "kvm", vendor: "Hetzner", product: "vServer", cpuThreads: 8, bootTime: "2026-10-06 09:14:02", timezone: "Asia/Tashkent", addresses: [{ interface: "eth0", address: "203.0.113.10/32" }, { interface: "eth0", address: "2001:db8::1/64" }, { interface: "docker0", address: "172.17.0.1/16" }], gateway: "172.31.1.1", dns: ["185.12.64.1", "185.12.64.2"], usersLoggedIn: 2, processes: 214, failedUnits: [], updates: 12, securityUpdates: 3, rebootRequired: true, packageManager: "apt", securityModule: "AppArmor, on" };
      case "sshd_report": return sshd;
      case "ufw_status": return ufw();
      case "ufw_add": {
        const r = args.rule;
        const to = r.app ?? `${r.port}${r.protocol === "any" ? "" : "/" + r.protocol}`;
        ufwRules.push([to, r.action.toUpperCase(), r.direction.toUpperCase(), r.address ?? "Anywhere", r.comment]);
        return ufw();
      }
      case "ufw_update": {
        const r = args.rule;
        const at = Math.min(...args.old.map((o: any) => o.number)) - 1;
        const gone = new Set(args.old.map((o: any) => o.number - 1));
        const to = r.app ?? `${r.port}${r.protocol === "any" ? "" : "/" + r.protocol}`;
        const next = ufwRules.filter((_, i) => !gone.has(i));
        next.splice(at, 0, [to, r.action.toUpperCase(), "IN", r.address ?? "Anywhere", r.comment]);
        ufwRules = next;
        return ufw();
      }
      case "ufw_delete": {
        const gone = new Set(args.rules.map((o: any) => o.number - 1));
        const left = ufwRules.filter((_, i) => !gone.has(i));
        if (!args.force && !left.some(([to, a, , from]) => to.startsWith("22") && a !== "DENY" && from.startsWith("Anywhere"))) throw lockout;
        ufwRules = left;
        return ufw();
      }
      case "ufw_set_enabled": ufwActive = args.enabled; return ufw();
      case "fail2ban_status": return f2b();
      case "fail2ban_unban": for (const j of Object.keys(banned)) if (!args.jail || args.jail === j) banned[j] = banned[j].filter((x) => x !== args.ip); return f2b();
      case "fail2ban_ban":
        if (args.ip === "203.0.113.9") throw { ...lockout, data: { check: "SSH access", output: "203.0.113.9 is the address this connection comes from; banning it would lock you out." } };
        banned[args.jail] = [...banned[args.jail], args.ip]; return f2b();
      case "certbot_status": return certs();
      case "certbot_issue":
        await progress(args.progress, [
          [0, "step", `Asking Let's Encrypt for a certificate for ${args.request.domains.join(", ")} …`],
          [600, "line", "Saving debug log to /var/log/letsencrypt/letsencrypt.log"],
          [900, "line", `Requesting a certificate for ${args.request.domains.join(" and ")}`],
          [1800, "line", "Performing the following challenges:"],
          [400, "line", `http-01 challenge for ${args.request.domains[0]}`],
          [1500, "line", "Waiting for verification..."],
          [2200, "line", "Cleaning up challenges"],
          [700, "line", "Successfully received certificate."],
        ]);
        addHttps(args.request.domains); return certbotOut(`Requesting a certificate for ${args.request.domains.join(" and ")}\n\nSuccessfully received certificate.\nCertificate is saved at: /etc/letsencrypt/live/${args.request.domains[0]}/fullchain.pem\nSuccessfully deployed certificate for ${args.request.domains[0]} to ${nginxConfig.sites.find((x) => x.serverNames.includes(args.request.domains[0]))?.file ?? "/etc/nginx/sites-enabled/blog"}`);
      case "certbot_renew":
        await progress(args.progress, [
          [0, "step", args.mode === "dryRun" ? `Testing renewal of ${args.name ?? "all certificates"} against Let's Encrypt's staging servers …` : `Renewing ${args.name} with Let's Encrypt …`],
          [500, "line", "Saving debug log to /var/log/letsencrypt/letsencrypt.log"],
          [800, "line", `Processing /etc/letsencrypt/renewal/${args.name ?? "shop.example.com"}.conf`],
          [1200, "line", args.mode === "dryRun" ? `Simulating renewal of an existing certificate for ${args.name ?? "shop.example.com"}` : `Renewing an existing certificate for ${args.name}`],
          [1600, "line", "Performing the following challenges:"],
          [300, "line", `http-01 challenge for ${args.name ?? "shop.example.com"}`],
          [2400, "line", "Waiting for verification..."],
          [1800, "line", "Cleaning up challenges"],
        ]);
        return certbotOut(args.mode === "dryRun" ? "Processing /etc/letsencrypt/renewal/shop.example.com.conf\nSimulating renewal of an existing certificate for shop.example.com\n\nCongratulations, all simulated renewals succeeded" : `Renewing an existing certificate for ${args.name}`);
      case "certbot_delete":
        if (!args.force) throw { kind: "PreflightFailed", message: "Pre-flight check failed", data: { check: "web server config", output: `These files still use this certificate, so the web server would fail to start without it:\n/etc/nginx/sites-enabled/${args.name.split(".")[0]}` } };
        return certbotOut(`Deleted all files relating to certificate ${args.name}.`);
      case "copy_between":
        await progress(args.progress, [[0, "step", `Copying ${args.srcPath} …`], ...transferEvents(12, 48_211_300, ["app.js", "vendor.js", "logo.png", "index.html"], 5000)]);
        return { files: 12, dirs: 3, bytes: 48_211_300, skipped: 0 };
      case "upload":
        await progress(args.progress, transferEvents(1, 18_400_000, [args.remotePath], 3000));
        return 18_400_000;
      case "service_action":
        await progress(args.progress, [[0, "step", `Running systemctl ${args.action} ${args.unit} …`], ...(args.unit.startsWith("nginx") ? [[400, "line", "nginx: the configuration file /etc/nginx/nginx.conf syntax is ok"], [200, "line", "nginx: configuration file /etc/nginx/nginx.conf test is successful"]] as [number, "line", string][] : []), [900, "line", ""]]);
        return { command: `systemctl ${args.action} -- ${args.unit}`, preflight: args.unit.startsWith("nginx") ? ["nginx -t", "syntax is ok"] : null, output: { stdout: "", stderr: "", exitCode: 0, durationMs: 120 } };
      case "list_services": return services;
      case "net_check": {
        const t = String(args.target);
        await new Promise((r) => setTimeout(r, args.check === "trace" ? 2200 : 700));
        const down = /down|nope|10\.9\./.test(t);
        if (args.check === "port") return down ? { ok: false, summary: "closed, filtered or nothing listening", output: `nc: connect to ${t} port ${args.port} (tcp) failed: Connection refused\n`, command: `nc -z -w 5 ${t} ${args.port}` } : { ok: true, summary: "the port is open", output: `Connection to ${t} ${args.port} port [tcp/*] succeeded!\n`, command: `nc -z -w 5 ${t} ${args.port}` };
        if (args.check === "http") return down ? { ok: false, summary: "curl: (6) Could not resolve host: " + t.replace(/^https?:\/\//, ""), output: "curl: (6) Could not resolve host\n", command: `curl -sS -o /dev/null --max-time 15 -- '${t}'` } : { ok: true, summary: "HTTP 200 in 0.18 s from 203.0.113.20", output: "@@http 200 0.181 203.0.113.20\n", command: `curl -sS -o /dev/null --max-time 15 -- '${t}'` };
        if (args.check === "dns") return down ? { ok: false, summary: "the name didn't resolve", output: "", command: `getent ahosts ${t}` } : { ok: true, summary: "resolves to 203.0.113.20, 2001:db8::20", output: "203.0.113.20\n2001:db8::20\n", command: `getent ahosts ${t}` };
        if (args.check === "ping") return down ? { ok: false, summary: "no answer, 100% packet loss", output: `4 packets transmitted, 0 received, 100% packet loss, time 3060ms\n`, command: `ping -c 4 -- ${t}` } : { ok: true, summary: "reachable, 11.8 ms average, 0% packet loss", output: `PING ${t} 56(84) bytes of data.\n64 bytes from ${t}: icmp_seq=1 ttl=57 time=11.6 ms\n…\n4 packets transmitted, 4 received, 0% packet loss, time 3004ms\nrtt min/avg/max/mdev = 11.4/11.8/12.3/0.3 ms\n`, command: `ping -c 4 -- ${t}` };
        return { ok: true, summary: "6 hops", output: ` 1  10.0.0.1  0.4 ms\n 2  172.16.4.1  1.2 ms\n 3  198.51.100.9  3.9 ms\n 4  * \n 5  203.0.113.1  9.8 ms\n 6  203.0.113.20  11.6 ms\n`, command: `traceroute -n -q 1 -w 2 -m 20 ${t}` };
      }
      case "unit_logs": {
        await new Promise((r) => setTimeout(r, 300));
        const from = args.since ? args.since * 1000 : Date.now() - 3_600_000, to = args.until ? args.until * 1000 : Date.now();
        const n = args.since || args.until ? 30 : 12;
        return Array.from({ length: n }, (_, i) => `${new Date(from + ((to - from) * i) / n).toISOString().slice(0, 19)}+0000 prod-web-01 ${args.unit.replace(".service", "")}[812]: ${LOG_LINES[i % LOG_LINES.length]}`).join("\n") + "\n";
      }
      case "service_detail": return {
        unit: args.unit,
        properties: { MainPID: "812", ActiveEnterTimestamp: "Tue 2026-10-27 09:14:02 UTC", MemoryCurrent: "48234496", TasksCurrent: "9", NRestarts: "0", User: "", FragmentPath: `/lib/systemd/system/${args.unit}` },
        logs: `2026-11-14T22:10:01+0000 prod-web-01 systemd[1]: Reloading ${args.unit}...\n2026-11-14T22:10:01+0000 prod-web-01 nginx[30211]: nginx: the configuration file /etc/nginx/nginx.conf syntax is ok\n2026-11-14T22:10:01+0000 prod-web-01 nginx[30211]: nginx: configuration file /etc/nginx/nginx.conf test is successful\n2026-11-14T22:10:01+0000 prod-web-01 systemd[1]: Reloaded ${args.unit}.\n`,
      };
      case "list_containers": return containers;
      case "container_stats": return containers.filter((c) => c.state === "running").map((c, i) => ({ id: c.id, name: c.names, cpuPercent: `${(0.4 + i * 3.7).toFixed(2)}%`, memUsage: `${120 + i * 210}MiB / 15.5GiB`, memPercent: "2.1%", netIo: "1.2GB / 900MB", blockIo: "0B / 0B", pids: "12" }));
      case "container_logs":
        if (args.since || args.until) {
          const from = args.since ? Date.parse(args.since) : Date.now() - 86_400_000, to = args.until ? Date.parse(args.until) : Date.now();
          await new Promise((r) => setTimeout(r, 400));
          return Array.from({ length: 30 }, (_, i) => `${new Date(from + ((to - from) * i) / 30).toISOString()}  ${LOG_LINES[i % LOG_LINES.length]}`).join("\n") + "\n";
        }
        if (containers.find((x) => x.id === args.id)?.state === "restarting") return "2026-11-18T09:41:02.118Z  INFO  shop-worker 2.14.0 starting\n2026-11-18T09:41:02.410Z  INFO  Connecting to postgres://db:5432/shop\n2026-11-18T09:41:07.415Z  ERROR Error: connect ECONNREFUSED 127.0.0.1:5432\n    at TCPConnectWrap.afterConnect [as oncomplete] (node:net:1606:16)\n2026-11-18T09:41:07.416Z  FATAL worker can't start without the database, exiting\n";
        return "2026-11-15T01:02:11.120Z  INFO  Started ShopApiApplication in 6.81 seconds\n2026-11-15T01:02:12.004Z  INFO  Tomcat started on port 8081 (http)\n2026-11-15T01:04:40.551Z  WARN  Slow query: SELECT * FROM orders WHERE … (1840 ms)\n";
      case "inspect_container": {
        const c = containers.find((x) => x.id === args.id);
        const labels = c?.composeProject ? { "com.docker.compose.project": c.composeProject, "com.docker.compose.service": c.names.replace(`${c.composeProject}-`, "").replace(/-\d+$/, ""), "com.docker.compose.project.config_files": `/srv/${c.composeProject}/docker-compose.yml`, "com.docker.compose.project.working_dir": `/srv/${c.composeProject}` } : {};
        if (c?.state === "restarting") return { Id: args.id, RestartCount: 14, HostConfig: { RestartPolicy: { Name: "always" }, Memory: 0 }, Config: { Image: c.image, Env: ["TZ=Asia/Tashkent"], Labels: labels }, State: { Status: "restarting", Restarting: true, ExitCode: 1, OOMKilled: false, Error: "", FinishedAt: new Date(Date.now() - 12_000).toISOString() } };
        return { Id: args.id, Config: { Image: c?.image ?? "nginx:1.27-alpine", Env: ["TZ=Asia/Tashkent"], Labels: labels }, State: { Status: "running", Health: { Status: "healthy" } } };
      }
      case "compose_action":
        await progress(args.progress, [
          [0, "step", `docker compose ${args.action} for ${args.target.project} …`],
          ...(args.action === "pull"
            ? ([[500, "line", " web Pulling"], [600, "line", " api Pulling"], [900, "line", " 3f4ca6 Downloading  12.4MB/48.1MB"], [900, "line", " 3f4ca6 Downloading  39.9MB/48.1MB"], [800, "line", " 3f4ca6 Pull complete"], [500, "line", " ✔ web Pulled"], [400, "line", " ✔ api Pulled"]] as [number, "line", string][])
            : ([[700, "line", ` Container ${args.target.project}-web-1  Recreate`], [1500, "line", ` Container ${args.target.project}-web-1  Recreated`], [700, "line", ` Container ${args.target.project}-web-1  Starting`], [900, "line", ` Container ${args.target.project}-web-1  Started`]] as [number, "line", string][])),
        ]);
        return { stdout: "", stderr: args.action === "pull" ? ` ✔ web Pulled\n ✔ api Pulled\n` : ` ✔ Container ${args.target.project}-${args.target.service ?? "web"}-1  ${args.action === "up" ? "Running" : "Started"}\n`, exitCode: 0, durationMs: 2400 };
      case "uptime_watch": {
        const first = !uptimeCh;
        uptimeCh = args.onEvent;
        if (!first) return null;
        // The blog's server falls over a little after launch and comes back half a minute later.
        setTimeout(() => {
          let blog = uptimeChecks.find((c) => c.target === "http://blog.example.com/");
          const api = uptimeChecks.find((c) => c.id === "u2")!;
          uptimeRun(api, false, "got 502 Bad Gateway, expected 200 OK");
          setTimeout(() => uptimeRun(api, true), 30_000);
          if (blog) uptimeRun(blog, false);
        }, 12_000);
        setTimeout(() => uptimeCh?.onmessage({ type: "certWarning", checkId: "u3", name: "grafana.example.com", message: "the TLS certificate expires in 9 days" }), 4_000);
        return null;
      }
      case "list_uptime": return uptimeChecks.map(uptimeStatus);
      case "save_uptime_check": {
        const i = args.input;
        if (i.kind === "http" && !/^https?:\/\/[^/\s]+/.test(i.target.trim())) throw { kind: "invalidInput", message: "the URL must start with http:// or https://" };
        const c: MockCheck = { id: args.id ?? `u${Date.now()}`, name: i.name?.trim() || i.target.trim(), kind: i.kind, target: i.target.trim(), expectedStatus: i.kind === "http" ? i.expectedStatus ?? 200 : 0, contains: i.kind === "http" ? i.contains?.trim() || null : null, intervalMinutes: i.intervalMinutes, paused: !!i.paused, createdAt: new Date().toISOString() };
        if (args.id) { uptimeChecks = uptimeChecks.map((x) => (x.id === args.id ? { ...c, createdAt: x.createdAt } : x)); uptimeResults[c.id] = []; uptimeChanges[c.id] = []; }
        else uptimeChecks.push(c);
        return c;
      }
      case "set_uptime_paused": { const c = uptimeChecks.find((x) => x.id === args.id)!; c.paused = args.paused; return { ...c }; }
      case "delete_uptime_check": uptimeChecks = uptimeChecks.filter((x) => x.id !== args.id); return null;
      case "run_uptime_check": {
        const c = uptimeChecks.find((x) => x.id === args.id)!;
        await new Promise((r) => setTimeout(r, 500));
        const prev = uptimeResults[c.id]?.[uptimeResults[c.id].length - 1];
        uptimeRun(c, c.target.includes("blog.example.com") ? false : c.id === "u2" ? prev?.up ?? true : true, c.target.includes("blog.example.com") ? "couldn't connect: Connection refused (os error 61)" : undefined);
        return uptimeStatus(c);
      }
      case "audit_query": return audit.filter((e) => !args.filter?.category || e.category === args.filter.category);
      case "audit_verify": return { ok: true, checked: 1284, firstBrokenSeq: null };
      case "snippets": return snippetList.map((s) => ({ ...s }));
      case "add_example_snippets": if (!snippetList.length) snippetList = EXAMPLES.map((e, i) => mkSnippet("sn" + i, e)).sort((a, b) => a.name.localeCompare(b.name)); return snippetList.map((s) => ({ ...s }));
      case "save_snippet": {
        if (!args.input.name.trim() || !args.input.command.trim()) throw { kind: "InvalidInput", message: "give the snippet a short name" };
        const s = mkSnippet(args.id ?? "sn" + Date.now(), args.input);
        snippetList = [...snippetList.filter((x) => x.id !== s.id), s].sort((a, b) => a.name.localeCompare(b.name));
        return s;
      }
      case "delete_snippet": snippetList = snippetList.filter((s) => s.id !== args.id); return null;
      case "run_snippet": {
        const s = snippetList.find((x) => x.id === args.id);
        const missing = s.params.filter((p: string) => !args.values[p]);
        if (missing.length) throw { kind: "InvalidInput", message: `fill in ${missing.join(", ")}` };
        const cmd = s.command.replace(/\{\{\s*([A-Za-z_][A-Za-z0-9_]{0,31})\s*\}\}/g, (_: string, n: string) => shq(args.values[n]));
        await new Promise((r) => setTimeout(r, 900));
        return args.sessionIds.map((sid: string, i: number) => ({ sessionId: sid, server: sid === "s-p1" ? "deploy@203.0.113.10" : sid === "s-p2" ? "ops@10.0.4.21" : "sirojiddin@staging.example.dev", command: cmd, output: { ...fakeRun(cmd, sid), durationMs: 380 + i * 210 }, error: null }));
      }
      case "local_ssh_keys": return localKeys.map((k) => ({ ...k }));
      case "generate_ssh_key": {
        if (localKeys.some((k) => k.name === args.name)) throw { kind: "InvalidInput", message: `~/.ssh/${args.name} already exists; choose another name` };
        const b64 = "AAAAC3NzaC1lZDI1NTE5AAAAI" + Math.random().toString(36).slice(2, 12) + "Demo";
        const k = { name: args.name, path: `~/.ssh/${args.name}`, algorithm: "ssh-ed25519", comment: args.comment, fingerprint: "SHA256:" + Math.random().toString(36).slice(2) + "NewKey0rbital", publicLine: `ssh-ed25519 ${b64} ${args.comment}`, encrypted: !!args.passphrase };
        localKeys = [...localKeys, k].sort((a, b) => a.name.localeCompare(b.name));
        return k;
      }
      case "authorized_keys": await new Promise((r) => setTimeout(r, 300)); return authReport();
      case "add_authorized_key": {
        const line = args.line.trim();
        if (!/^(ssh-ed25519|ssh-rsa|ecdsa-sha2-nistp256) \S{8,}/.test(line)) throw { kind: "InvalidInput", message: "that isn't an SSH public key: it should look like \"ssh-ed25519 AAAA… name\" (the .pub file, never the private key)" };
        if (authLines.some((l) => l.includes(line.split(" ")[1]))) throw { kind: "InvalidInput", message: "this key can already sign in" };
        authLines = [...authLines, line];
        return authReport();
      }
      case "remove_authorized_key": {
        const mine = authLines.filter((l) => localKeys.some((k) => l.includes(k.publicLine.split(" ")[1])));
        if (!args.force && mine.length === 1 && mine[0] === args.line && profiles.find((p) => p.id === "p1")?.authMethod !== "password")
          throw { kind: "PreflightFailed", message: "preflight", data: { check: "your access", output: "This is the only key from this computer that can sign in to this account (SHA256:Xw2pRfa9Lw0YkU0yV5mJbqzB2hD1tXcN7pGfE4sHjAo). This connection signs in with a key, so after removing it you could only get back in with a password, if the server allows one." } };
        authLines = authLines.filter((l) => l !== args.line);
        return authReport();
      }
      case "dns_accounts": return dnsAccounts.map((a) => ({ ...a }));
      case "add_dns_account": {
        await new Promise((r) => setTimeout(r, 600));
        if (args.token.trim().length < 8) throw { kind: "PermissionDenied", message: "Cloudflare refused the token: Invalid API Token (1000)" };
        const a = { id: "cf" + (dnsAccounts.length + 1), name: args.name, kind: "cloudflare", createdAt: new Date().toISOString() };
        dnsAccounts.push(a);
        dnsZones[a.id] = [{ id: "z9", name: "side-project.app", status: "active", nameServers: [] }];
        return a;
      }
      case "remove_dns_account": dnsAccounts.splice(dnsAccounts.findIndex((a) => a.id === args.id), 1); return null;
      case "dns_zones": await new Promise((r) => setTimeout(r, 300)); return dnsZones[args.accountId] ?? [];
      case "dns_records": await new Promise((r) => setTimeout(r, 300)); return dnsRecs.filter((r) => r.zone === args.zoneId).map(dnsOut);
      case "create_dns_record": case "update_dns_record": {
        const i = args.input;
        if ((i.type === "A" && !/^\d+\.\d+\.\d+\.\d+$/.test(i.content.trim())) || !i.content.trim()) throw { kind: "InvalidInput", message: `not an IPv4 address for a A record: "${i.content}"` };
        const name = !i.name.trim() || i.name.trim() === "@" ? args.zone : i.name.trim().endsWith(args.zone) ? i.name.trim() : `${i.name.trim()}.${args.zone}`;
        const proxiable = ["A", "AAAA", "CNAME"].includes(i.type);
        const rec = { zone: args.zoneId, id: args.recordId ?? "r" + Math.floor(Math.random() * 1e6), name, type: i.type, content: i.content.trim(), ttl: i.ttl, proxied: proxiable && i.proxied, proxiable, priority: i.type === "MX" ? i.priority ?? 10 : null, comment: null, at: Date.now() };
        dnsRecs = [...dnsRecs.filter((r) => r.id !== rec.id), rec];
        return dnsOut(rec);
      }
      case "delete_dns_record": dnsRecs = dnsRecs.filter((r) => r.id !== args.recordId); return null;
      case "dns_check": await new Promise((r) => setTimeout(r, 400)); return dnsCheck(args.name.trim().toLowerCase(), args.rtype, args.expected);
      case "server_public_addresses": return args.sessionId === "s-p1" ? ["203.0.113.10", "2001:db8::1"] : args.sessionId === "s-p3" ? ["198.51.100.7"] : [];
      case "dns_points_here": {
        await new Promise((r) => setTimeout(r, 500));
        const addresses = args.sessionId === "s-p1" ? ["203.0.113.10", "2001:db8::1"] : args.sessionId === "s-p3" ? ["198.51.100.7"] : [];
        return { addresses, checks: addresses.length ? args.names.map((n: string) => dnsCheck(n, "A", addresses.filter((a) => !a.includes(":")), true)) : [] };
      }
      case "known_hosts": return [{ host: "203.0.113.10", port: 22, algorithm: "ssh-ed25519", fingerprint: "SHA256:Xw2pRfa9Lw0YkU0yV5mJbqzB2hD1tXcN7pGfE4sHjAo", addedAt: "2026-10-01T10:00:00Z" }];
      case "open_terminal": {
        const cb = (m: any) => args.onEvent.onmessage(m);
        termCb = cb;
        setTimeout(() => cb({ type: "data", data: b64(`Welcome to Ubuntu 24.04.1 LTS (GNU/Linux 6.8.0-45-generic x86_64)\r\n\r\n  System load:  1.42     Memory usage: 57%   Processes: 214\r\n  Usage of /:   40.1%    Swap usage:   8%     Users logged in: 1\r\n\r\n${prompt}`) }), 50);
        return "t1";
      }
      case "terminal_write": {
        const d: string = args.data;
        if (d === "\r") {
          const out = line.trim() === "docker ps --format '{{.Names}}'" ? containers.map((c) => c.names).join("\r\n") + "\r\n" : line.trim() ? `${line.trim()}: demo shell\r\n` : "";
          termCb?.({ type: "data", data: b64(`\r\n${out}${prompt}`) });
          line = "";
        } else {
          line += d;
          termCb?.({ type: "data", data: b64(d) });
        }
        return null;
      }
      case "plugin:event|listen": return 1;
      case "save_text": return null;
      case "stats_state": return statsState;
      case "set_stats": statsState = { available: true, enabled: args.enabled }; return statsState;
      case "stats_ping": return null;
      case "plugin:dialog|save": return `/Users/demo/Downloads/${args.options?.defaultPath ?? "download"}`;
      // Picking an SSH config file in the import dialog.
      case "plugin:dialog|open": return args.options?.title?.includes("SSH config") ? "/Users/demo/work/ssh_config" : null;
      // Updates: a newer version is always on offer; the download takes a few seconds.
      case "plugin:app|version": return "1.0.0";
      case "plugin:updater|check":
        await new Promise((r) => setTimeout(r, 700));
        return {
          rid: 1, currentVersion: "1.0.0", version: "1.1.0", date: "2026-11-20T12:00:00Z", rawJson: {},
          body: "### Added\n- Transfer queue with progress for uploads and downloads\n- Import hosts from ~/.ssh/config\n\n### Fixed\n- WebDAV listings on servers that omit content length",
        };
      case "plugin:updater|download_and_install": {
        const total = 14_200_000;
        const send = (m: any) => args.onEvent.onmessage(m);
        send({ event: "Started", data: { contentLength: total } });
        for (let got = 0; got < total; got += 710_000) {
          await new Promise((r) => setTimeout(r, 120));
          send({ event: "Progress", data: { chunkLength: Math.min(710_000, total - got) } });
        }
        send({ event: "Finished" });
        return null;
      }
      case "plugin:process|restart": location.reload(); return null;
      default: return null;
    }
  });
}
