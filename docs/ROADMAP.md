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
- [ ] **Jump hosts / bastions** (ProxyJump) and import of `~/.ssh/config` hosts
- [ ] **Backups browser**: list, diff and restore `*.outpost-bak-*` versions from the editor
- [ ] Touch ID gate before using saved credentials (`SecAccessControl` with `.userPresence`)
- [ ] Transfer queue with progress, resume and folder upload and download
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
