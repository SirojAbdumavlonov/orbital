# Changelog

All notable changes to Orbital are listed here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow [Semantic Versioning](https://semver.org/).

## Unreleased

### Added
- **Import from `~/.ssh/config`** (Welcome screen, the sidebar's "+" menu, or ⌘K): every `Host` in the file and the files it includes, with its address, user, port, key and route shown before anything is saved. `ProxyJump` (including chains such as `bastion,admin@10.0.0.5:2200`) and the old `ProxyCommand ssh -W %h:%p` form become jump hosts, created first and reused when they are already saved. Hosts already saved (same host, port and user) are marked and skipped. `Match` blocks, unsupported `ProxyCommand`s, loops and routes with more than three jump hosts are pointed out instead of guessed at. Only the config files are read; keys stay where they are and nothing secret is copied.

### Fixed
- Text fields no longer capitalise the first letter, autocorrect or spell-check what you type (macOS did this to user names, hosts and paths, so connections failed).

## 1.0.0 (2026-11-15)

### Added
- **Updates inside the app**: Orbital looks for a new version at launch and every six hours (Settings → Updates, or ⌘K → Check for updates). Click Update to download it, Restart to use it; servers, passwords and settings stay. Updates are checked against Orbital's signing key before they are installed. `npm run release -- patch|minor|major|beta` bumps the version everywhere, dates this file, commits and tags.
- **More kinds of connection**: FTP, FTPS (explicit TLS), WebDAV over HTTP or HTTPS (Nextcloud, ownCloud and others) and S3 with S3-compatible storage (Amazon S3, Backblaze B2, Cloudflare R2, Google Cloud Storage with HMAC keys, Wasabi, DigitalOcean Spaces, MinIO). Pick one in the server dialog; presets fill in endpoints and regions. These connections have the Files tab (browse, edit, upload files and folders, download, rename, delete) and the activity log; server tools stay SSH-only. Passwords and S3 secret keys go to the vault like the others.
- **Deploy tab**: register an app (folder, branch, build command, how to restart: service, Compose or a command), then deploy in one click: fetch, fast-forward only, build, restart, with each step's output live. Refuses when tracked files were changed on the server. Roll back to any recent commit; history with full logs.
- **Logs tab**: follow log files, service journals and container output live, several at once in different colours, with search, an errors-and-warnings filter, pause and wrap.
- **Tunnels tab**: use a port of the server (or of a machine only it can reach) from this Mac, for example a database for TablePlus. Listens on 127.0.0.1 only; suggestions from the server's listening ports; Open ports has a "Forward" button. Shows connections and traffic.
- **Cron tab**: every scheduled job (users' crontabs, /etc/crontab, /etc/cron.d) with its schedule in words, next runs in server time and last run from the cron log; add, edit, pause, resume and delete jobs in users' crontabs (through `crontab`, with a live schedule preview); Run now; systemd timers with Run now; the hourly/daily/weekly/monthly scripts.
- **Monitor: what uses the resources.** CPU (measured over one second) and memory by program, service, user or process, refreshed every 5 seconds; disk usage by folder, from any disk, clicking into folders to find what takes the space.
- Folders opened from a tool tab (Cron, nginx, fail2ban, …) are listed in the right-hand panel, with files opening in the editor below, instead of switching to Files.
- Terminals open as root (`sudo -i`, or `sudo -s` in a chosen folder) when the server's settings allow sudo, typing a saved sudo password invisibly. A `#` marks root tabs; a button opens a shell as your own user instead; Settings has the default.
- The window can be dragged by its top bars (Tauri drag regions; `-webkit-app-region` does nothing in the macOS web view).
- **Database tabs**: PostgreSQL, MySQL/MariaDB and MongoDB list databases with size, tables and connections, and back any one up to this Mac in one click (dumped on the server, downloaded, removed from the server). Redis shows memory, keys per database, clients and hit rate, with snapshot and flush (typed confirmation).
- **Apache tab**: virtual hosts from `apachectl -S`, Debian sites to enable or disable (undone if configtest fails), loaded modules, reload.
- **HAProxy tab**: frontends, backends and servers from haproxy.cfg; with a stats socket, live server status and Drain / Maintenance / Back in rotation.
- **Users tab** on every server: accounts with groups, admin rights, shell, sign-in status and SSH keys; add a user with password and/or SSH keys, groups, shell, home, primary group, UID, expiry and system-account options; change groups, shell, password, add keys, lock, unlock, delete. Your own account and root are protected from lockout.
- The sidebar can be resized by dragging its edge, and hidden (button or ⌃⌘S); the sidebar and back/forward buttons stay in the same place either way.
- Light theme, with a sun/moon switch at the top of the sidebar and "Match macOS" in Settings.
- **Settings page** (bottom of the sidebar): theme, editor font size, word wrap and minimap, terminal font size, monitoring refresh rate, reset remembered tabs, about.
- **⌘K command menu** (cmdk): jump to any server, tab or page, open detected software, reconnect, switch theme.
- Drag the line between the two parts of any page to resize them; widths are remembered, double-click restores the default.
- Close buttons (and Esc) on detail panels: nginx sites, containers, services.
- Docker: filter containers by state, sort by name, state, image, CPU or memory, ascending or descending; a **Compose** tab beside Logs and Inspect with the compose file, Edit, Apply changes (`up -d`), Restart service and Pull images.
- Monitor: **Server details** with hostname, OS, machine type and vendor, addresses, gateway, DNS, time zone, boot time, pending updates, reboot needed, failed services, logged-in sessions and AppArmor/SELinux.
- **HTTPS certificates tab** (certbot): certificates with expiry and auto-renewal status; request, renew, test renewal and delete; "Get HTTPS certificate" from an nginx site.
- **Firewall tab** (ufw): add, change and delete rules, turn the firewall on and off, with a check that the current SSH connection stays reachable.
- **Fail2ban tab**: jails, banned addresses, ban and unban.
- **SSH server tab**: effective sshd settings checked against hardening advice, and who is logged in.
- **Open ports tab**: listening ports with program, service, exposure and firewall verdict.
- Tool tabs behind a "+" button for software detected on the server, and an Nginx tab with sites, upstreams and config files.
- Folder upload, and copying files and folders between two connected servers.
- Jump hosts (ProxyJump), with a separate host-key check and credentials per hop.
- Optional Touch ID before saved credentials are used.
- Back and forward between pages (arrows next to the window buttons, two-finger swipe, ⌘[ and ⌘], mouse side buttons).
- After changing the connection settings of a connected server, Orbital offers to reconnect so they take effect (or later, from a banner).
- Files opened from a tool tab (nginx, SSH server, …) open in an editor beside that tab instead of switching to Files.
- Terminal tabs can be renamed by double-clicking them.
- GitHub release workflow, issue and pull request templates, Dependabot, code of conduct.

### Changed
- **No more Keychain prompts.** Saved passwords live in Orbital's own encrypted vault (ChaCha20-Poly1305, keyed to this Mac). Data moved to a hidden folder, `~/Library/Application Support/.orbital`; the database moves there by itself. Passwords saved in the Keychain before aren't read (that would prompt again): Orbital asks for each once, on the next connect, and remembers it.
- Notifications use Sonner, dialogs use Radix Dialog (focus stays inside, screen readers get titles), live numbers animate with NumberFlow.
- **Renamed from Outpost to Orbital**, with a new logo. On first start Orbital copies the old database (servers, trusted host keys, activity log) from `~/Library/Application Support/app.outpost.desktop`, leaving the original in place, and moves saved passwords from the old Keychain entries the first time each is used. Backups made under the old name (`*.outpost-bak-*`) are still listed.
- The "+" menu only lists software Orbital has something to show for (its own tab, or config and log files it found).
- "Save and connect" on a connected server reconnects it when connection settings changed, instead of keeping the old connection.
- Docker is now a tool tab that opens by default when the server has Docker.
- Security problems are reported through GitHub private vulnerability reporting.

## 0.1.0

First version: SFTP browser, Monaco editor with safe save, systemd services, Docker, monitoring, terminal and a hash-chained activity log.
