# Roadmap

## 0.1 (this release)
- SFTP browser with uploads (including Finder drag and drop), downloads, rename, chmod and delete
- Monaco editor with conflict detection; safe save with validators, backup and rollback; sudo save
- systemd services with pre-flight checks; journal logs
- Docker containers, stats, logs, inspect and lifecycle actions
- Agentless monitoring
- PTY terminal tabs, including "open terminal here"
- Hash-chained audit log with verify and export
- Keychain secrets, host-key pinning, password, key and agent auth, sudo modes

## Next
- [x] **Jump hosts / bastions** (ProxyJump): a profile connects through another profile, up to three hops
- [ ] Import of `~/.ssh/config` hosts (including their `ProxyJump` lines)
- [ ] **Backups browser**: list, diff and restore `*.orbital-bak-*` versions from the editor
- [x] Touch ID gate before using saved credentials (app-enforced, `LAContext` device-owner policy)
- [ ] Optionally keep the vault key in the Keychain behind `SecAccessControl(.userPresence)` for signed builds (stronger against other apps, one Touch ID prompt)
- [x] **Tool tabs**: Orbital detects the software on a server (nginx, Apache, Caddy, HAProxy, Docker, PostgreSQL, MySQL, Redis, MongoDB, PHP-FPM, sshd, fail2ban, ufw, cron) and offers each as a tab behind the "+" button
- [x] **Nginx tab**: every site (domain names, ports, TLS, what it serves), upstreams and config files, read from `nginx -T`; editing goes through safe save
- [x] Folder upload from the Mac, and copying files and folders from one connected server to another
- [ ] Nginx: enable and disable a site, new-site template
- [x] **HTTPS certificates** (certbot): list with expiry and auto-renew status, request new ones for nginx, Apache, a folder or standalone, renew, test renewal, delete with an in-use check; "Get HTTPS certificate" from an nginx site
- [ ] Wildcard certificates through DNS challenges (provider API credentials in the vault)
- [x] **Firewall** (ufw): see, add, change and delete rules, turn it on and off, with a check that SSH stays reachable
- [x] **Fail2ban**: jails, banned addresses, ban and unban
- [x] **SSH server review**: effective `sshd -T` settings checked against hardening advice, who is logged in
- [x] **Open ports**: what listens where, which program and service, reachable from outside or not, firewall verdict, Docker bypass warning
- [ ] One-click hardening fixes for sshd (a drop-in file through safe save), and authorized_keys management
- [ ] firewalld (RHEL family) alongside ufw
- [ ] Dedicated tabs for more tools (PostgreSQL databases and roles, cron jobs); they share the generic tab for now
- [ ] Transfer queue with progress and resume; folder download
- [ ] Port forwarding (local and remote) and an "open in browser" button for exposed ports
- [ ] Docker Compose projects as first-class objects: up, down, pull, logs across services
- [ ] Log viewer with follow (`journalctl -f`, `docker logs -f`, `tail -F`) and search
- [ ] Optional terminal session recording (asciinema format), linked from the audit log
- [ ] Signed audit-log anchors (export hash heads to a remote, append-only store)
- [ ] Team edition: shared server catalogue, central audit log, SSO, role-based permissions
- [ ] Windows and Linux builds (Tauri already supports both)
- [ ] Auto-update (tauri-plugin-updater) and crash reporting (opt-in)

## Ideas
- Alerts when disk is above 90% or a service fails, via macOS notifications
- Snippets: saved, parameterized commands with the same audit trail
- Cron editor with "next runs" preview
