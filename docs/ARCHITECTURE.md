# Architecture

```
┌──────────────────────── Orbital.app ────────────────────────┐
│  React UI (WKWebView)                                        │
│  Files · Editor · Services · Monitor · Terminal · Activity   │
│  + tool tabs (Nginx, Docker, Firewall, Fail2ban, HTTPS       │
│    certificates, SSH server, Open ports, …)                  │
│        │ invoke() / Channel (typed, src/lib/api.ts)          │
│  ──────┼──────────────────────────────────────────────────── │
│  src-tauri  (thin command layer, no logic)                   │
│        │                                                     │
│  orbital-core::Engine  ◄── the only entry point; audits      │
│   ├── ssh        connect, host-key check, auth, exec, sudo   │
│   ├── sftp       list/read/write/upload/download/chmod,      │
│   │               folder upload, server-to-server copy       │
│   ├── config_edit safe save: stage → validate → backup →     │
│   │               install → validate → rollback              │
│   ├── services   systemd list/show/journal + guarded actions │
│   ├── docker     docker CLI (JSON formats) + sudo fallback   │
│   ├── metrics    one-shot /proc script, parsed locally       │
│   ├── tools      detects installed software (fixed catalogue)│
│   ├── nginx      parses `nginx -T` into sites and upstreams  │
│   ├── firewall   ufw rules: read, build commands, lockout    │
│   │               check (would SSH still get through?)       │
│   ├── fail2ban   jails, bans; ban/unban commands             │
│   ├── certbot    Let's Encrypt certificates, renewal         │
│   ├── sshd       `sshd -T` settings vs hardening advice      │
│   ├── ports      listening sockets (ss / lsof) + ufw verdict │
│   ├── terminal   PTY channels streamed to xterm.js           │
│   ├── audit      append-only, SHA-256 hash chain             │
│   ├── known_hosts TOFU pinning + ~/.ssh/known_hosts          │
│   ├── profiles   bookmarks (no secrets), jump-host routes    │
│   ├── presence   Touch ID / login password (LAContext)       │
│   ├── secrets    secret store trait + the presence gate      │
│   └── vault      encrypted credentials file (ChaCha20)       │
│        │                         │                           │
│   ~/Library/Application Support/.orbital: SQLite, vault      │
└────────┼─────────────────────────────────────────────────────┘
         │  one SSH connection per server (multiplexed channels)
         ▼
   sshd ─ sftp-server · sh (exec) · PTY shell
```

## Principles

1. **Agentless.** Everything uses what any Linux server already has: sshd, the SFTP subsystem, POSIX `sh`, `/proc`, `systemctl`, `journalctl` and the `docker` CLI. Nothing is installed remotely.
2. **One connection, many channels.** SFTP, command execution, metrics polling and every terminal tab are channels on the same SSH session. You authenticate once.
3. **Every mutation goes through `Engine` and is audited.** The Tauri layer can't reach `SshSession` directly. Reads (listing, viewing, metrics) aren't logged; changes always are, including failed and blocked attempts.
4. **No string-built shell injection.** Every interpolated value passes through `shell::quote`. Unit names and container ids must also match a strict allow-list before they reach a command line.
5. **Core without UI.** `orbital-core` has no Tauri dependency, so it is tested headless against a real sshd in CI.

## Key flows

**Connecting.** `russh` calls `check_server_key`. The key is looked up first in Orbital's pinned keys, then in `~/.ssh/known_hosts`. An unknown key fails the connection with `UnknownHostKey { fingerprint }`, and the UI shows the trust dialog. Reconnecting with `accept_fingerprint` pins exactly that fingerprint and nothing else. A mismatch is always refused, even if a fingerprint was "accepted". Authentication then follows the profile: password (falling back to keyboard-interactive), a key file or ssh-agent identities.

**Jump hosts.** A profile may name another profile as its jump host. `profiles::chain` resolves the route (bastions first, target last; loops, missing profiles and more than four hops are errors). `SshSession::connect_route` connects the first hop over TCP and every later hop over a `direct-tcpip` channel of the hop before it, running the same host-key check and authentication for each against that hop's own pin and credentials. The target session owns its jump sessions and closes them with itself. `ConnectOptions` carries the endpoint a fingerprint was confirmed for (`accept_host`) and secrets typed for individual hops (`hop_secrets`), and errors name the profile they are about, so the UI can prompt hop by hop.

**Touch ID gate.** `Engine` wraps the secret store in `GatedStore`. With the gate on, reading a saved server credential fails with `PresenceRequired` unless an `Unlock` is alive. `Engine::connect` works out whether the route would read any saved secret (`saved_secrets_wanted`), asks `UserPresence::verify` if so, and holds the `Unlock` only until authentication finishes. The check that decides whether to ask is a convenience; the store is what refuses, so a mistake there fails closed.

**Privileged commands.** `exec_privileged(cmd)` wraps the command as `sudo -n -- sh -c '<cmd>'` (passwordless) or `sudo -k -S -p '' -- sh -c '<cmd>'` with the password written to the channel's stdin. `-k` forces sudo to read the password every time, so it can never leak into the command's own stdin.

**Safe save.** See `config_edit.rs`. Validators are picked by path. *Pre-install* validators (`visudo -cf`, `haproxy -c -f`) check the temp file before the real file is touched. *Post-install* validators (`nginx -t`, `sshd -t`) check the live tree and trigger a rollback from the backup if they fail. The write is `cat tmp > file` rather than a rename, so ownership, mode, ACLs and the inode (which matters for Docker bind mounts) are preserved.

**Metrics.** One `exec` per tick runs `SNAPSHOT_SCRIPT`, which prints tagged sections. `parse_snapshot` turns them into a struct, and `derive` computes CPU percentages and network rates from the previous sample. The UI polls every 2 s for the visible server and every 10 s for background ones.

**Tool detection.** `tools::CATALOG` is a fixed list of software Orbital knows (binaries, systemd units, process names, config and log paths). `detect_script` turns it into one POSIX `sh` script, with nothing taken from the user or the server, and `parse_detect` reads the tagged sections it prints. A tool is listed when its binary, unit or config path exists, and marked running when its unit is active or its process is found. The UI runs this once after connecting and offers the result behind the "+" button; the tabs a user opened are remembered per profile. Nginx and Docker have their own views, every other tool gets a generic one (service controls, config and log paths, journal).

**Nginx.** `nginx_config` runs `nginx -T` (through sudo when needed), which prints the validity check followed by every loaded file. `nginx::parse_dump` tokenizes that into server blocks (names, listens, TLS, root, redirect, locations with `proxy_pass`), upstreams and the file list, each with the file and line it came from. It is read-only: changes are made in the editor and go through safe save, so `nginx -t` and rollback apply.

**Transfers.** `upload_tree` walks a local file or folder and streams it over SFTP. `Engine::copy_between` copies a file or folder from one connected session to another by reading from the source's SFTP channel and writing to the destination's, so the data passes through the Mac and the servers never connect to each other or see each other's credentials. Permission bits are kept, symlinks are skipped and reported, and existing files are left alone unless overwrite was chosen. The copy is audited on the destination server.

**Security tools.** The firewall, fail2ban, certbot, sshd and open-ports views each read through one fixed script run with `exec_inspect`: as root when the profile has sudo, otherwise as the login user (the views then say what they can't see). Changes go through `Engine` methods that build the command from validated input (`firewall::rule_args`, `fail2ban::ban_command`, `certbot::issue_command`, …), run it with `exec_privileged` and record it under the Security category, including refusals. Names that come from the server, such as fail2ban jail names, stay in shell variables inside the script and are validated again before the user can send them back.

**Lockout guard.** Before a firewall change goes out, the engine works out the rule list it would produce and asks `firewall::ssh_reachable` whether the SSH connection in use would still get through: the first matching rule decides, otherwise the default incoming policy. The client address and the server port come from `$SSH_CONNECTION`, so this is right behind NAT, port forwarding and jump hosts. If SSH would be blocked, the change fails with `PreflightFailed` and the UI asks the user to type a confirmation before retrying with `force`. Rules are deleted by number only after checking that the number still shows the same rule text, and a rule's IPv6 copy is changed or deleted with it. Banning the connection's own address in fail2ban is refused outright, and deleting a certificate that a web-server config still references needs the same typed confirmation.

**Open ports.** `ports::SCRIPT` lists listening sockets with `ss -tulpn` (or `lsof` where `ss` is missing), maps each process to its systemd unit through `/proc/<pid>/cgroup`, and, when ufw is on, attaches what the firewall does with traffic from the internet. Ports published by Docker are flagged, since Docker's own iptables rules bypass ufw.

**Audit chain.** `hash_n = SHA-256(hash_{n-1} ‖ canonical_json(row_n))`, with the genesis hash being 64 zeros. Triggers reject `UPDATE`/`DELETE`. `verify()` walks the chain and reports the first row that doesn't match.
