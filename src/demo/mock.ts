// Demo mode: a fake backend so the UI runs in a plain browser (`npm run demo`)
// for design work, screenshots and marketing. Never included in the Tauri build path.
import { mockIPC } from "@tauri-apps/api/mocks";

const now = () => Math.floor(Date.now() / 1000);
const profiles = [
  { id: "p1", name: "prod-web-01", host: "203.0.113.10", port: 22, username: "deploy", authMethod: "agent", keyPath: null, sudoMode: "password", groupName: "Production", color: "#e5645a", initialPath: null, createdAt: "2026-10-01T10:00:00Z", lastConnectedAt: "2026-11-14T18:00:00Z" },
  { id: "p2", name: "db-primary", host: "10.0.4.21", port: 22, username: "ops", authMethod: "key", keyPath: "~/.ssh/id_ed25519", sudoMode: "noPassword", groupName: "Production", color: "#8fb4ff", initialPath: null, createdAt: "2026-10-01T10:00:00Z", lastConnectedAt: "2026-11-12T08:00:00Z" },
  { id: "p3", name: "staging", host: "staging.example.dev", port: 2222, username: "sirojiddin", authMethod: "password", keyPath: null, sudoMode: "auto", groupName: "Staging", color: null, initialPath: null, createdAt: "2026-10-01T10:00:00Z", lastConnectedAt: null },
];
const trusted = new Set(["p1", "p2"]);

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
  "/etc": [["docker", "dir", 4096, "rwxr-xr-x"], ["nginx", "dir", 4096, "rwxr-xr-x"], ["ssh", "dir", 4096, "rwxr-xr-x"], ["systemd", "dir", 4096, "rwxr-xr-x"], ["hosts", "file", 221, "rw-r--r--"], ["sudoers", "file", 1800, "r--r-----"]],
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
    memCached: 4_900_000_000, swapTotal: 2_147_483_648, swapUsed: 182_000_000, uptimeSecs: 3_456_789 + (t % 1000),
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
  durationMs: 40 + i * 31, details: outcome === "failure" ? { error: "Validation failed, change was rolled back: nginx: [emerg] unexpected \"}\" in /etc/nginx/sites-enabled/shop:41", validator: "nginx" } : { sha256: "9f86d081884c7d65…", backup: target?.toString().startsWith("/etc") ? `${target}.outpost-bak-20261114" ` : undefined },
  prevHash: "a3c1…", hash: "5be0d2f4c8a1e9b7…",
}));

let termCb: ((m: any) => void) | null = null;
let line = "";
const b64 = (s: string) => btoa(unescape(encodeURIComponent(s)));
const prompt = "\x1b[1;33mdeploy@prod-web-01\x1b[0m:\x1b[1;34m/srv/shop\x1b[0m$ ";

export function install() {
  mockIPC((cmd, args: any) => {
    switch (cmd) {
      case "list_profiles": return profiles;
      case "has_secret": return true;
      case "connect": {
        if (!trusted.has(args.profileId) && !args.opts?.acceptFingerprint)
          throw { kind: "UnknownHostKey", message: "unknown", data: { host: "staging.example.dev", port: 2222, algorithm: "ssh-ed25519", fingerprint: "SHA256:kq3W0dF7vYV1oR6nZQh2xJmUe8pT5cB4sLaGi9NwEo0" } };
        trusted.add(args.profileId);
        const p = profiles.find((x) => x.id === args.profileId)!;
        return { id: "s-" + p.id, profileId: p.id, label: `${p.username}@${p.host}`, home: "/home/deploy", hasSystemd: true, dockerVersion: "27.3.1", elevation: true, connectedAt: new Date().toISOString() };
      }
      case "list_dir": return listing(args.path);
      case "read_file": return { path: args.path, content: args.path.endsWith("nginx.conf") ? NGINX : "# demo file\n", size: 1482, mtime: now() - 9000, modeOctal: 0o644, writable: !args.path.startsWith("/etc") };
      case "validator_for": return args.path.startsWith("/etc/nginx/") ? { name: "nginx", command: "nginx -t", stage: "postInstall" } : null;
      case "safe_write": return { path: args.path, bytes: args.content.length, privileged: args.privileged, backupPath: args.path + ".outpost-bak-20261115011500", validator: "nginx", validatorOutput: "syntax is ok", newMtime: now(), suggestedReload: "nginx.service" };
      case "write_file": return now();
      case "metrics": return metric();
      case "list_services": return services;
      case "service_detail": return {
        unit: args.unit,
        properties: { MainPID: "812", ActiveEnterTimestamp: "Tue 2026-10-27 09:14:02 UTC", MemoryCurrent: "48234496", TasksCurrent: "9", NRestarts: "0", User: "", FragmentPath: `/lib/systemd/system/${args.unit}` },
        logs: `2026-11-14T22:10:01+0000 prod-web-01 systemd[1]: Reloading ${args.unit}...\n2026-11-14T22:10:01+0000 prod-web-01 nginx[30211]: nginx: the configuration file /etc/nginx/nginx.conf syntax is ok\n2026-11-14T22:10:01+0000 prod-web-01 nginx[30211]: nginx: configuration file /etc/nginx/nginx.conf test is successful\n2026-11-14T22:10:01+0000 prod-web-01 systemd[1]: Reloaded ${args.unit}.\n`,
      };
      case "list_containers": return containers;
      case "container_stats": return containers.filter((c) => c.state === "running").map((c, i) => ({ id: c.id, name: c.names, cpuPercent: `${(0.4 + i * 3.7).toFixed(2)}%`, memUsage: `${120 + i * 210}MiB / 15.5GiB`, memPercent: "2.1%", netIo: "1.2GB / 900MB", blockIo: "0B / 0B", pids: "12" }));
      case "container_logs": return "2026-11-15T01:02:11.120Z  INFO  Started ShopApiApplication in 6.81 seconds\n2026-11-15T01:02:12.004Z  INFO  Tomcat started on port 8081 (http)\n2026-11-15T01:04:40.551Z  WARN  Slow query: SELECT * FROM orders WHERE … (1840 ms)\n";
      case "inspect_container": return { Id: args.id, Config: { Image: "nginx:1.27-alpine", Env: ["TZ=Asia/Tashkent"] }, State: { Status: "running", Health: { Status: "healthy" } } };
      case "audit_query": return audit.filter((e) => !args.filter?.category || e.category === args.filter.category);
      case "audit_verify": return { ok: true, checked: 1284, firstBrokenSeq: null };
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
      default: return null;
    }
  });
}
