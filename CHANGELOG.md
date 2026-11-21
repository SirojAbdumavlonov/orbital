# Changelog

All notable changes to Orbital are listed here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow [Semantic Versioning](https://semver.org/).

## Unreleased

## 1.3.8 (2026-11-21)

### Fixed
- **Port forwarding that couldn't work said nothing.** The tunnel opened ("Portainer is on your computer at localhost:9000"), then the browser just failed when the server refused each connection. Orbital now tries the route once when you open a tunnel and says why it can't work: nothing listens on that port on the server, or the server's SSH settings don't allow forwarding (`AllowTcpForwarding no`). A tunnel that stops working later shows the reason in the Port forwarding tab.

## 1.3.7 (2026-11-21)

### Fixed
- **Docker: "permission denied while trying to connect to the docker API" in a container's logs** (and in Compose actions) on servers where the login user isn't in the docker group, even with sudo allowed. Orbital retries Docker commands with sudo when the daemon refuses the user, but only looked for that refusal in the error output, while logs and Compose fold errors into normal output. It now looks at both.

## 1.3.6 (2026-11-21)

### Fixed
- **AI agent: "Problem: telegram: The read operation timed out" filled the activity list.** The agent waits for new Telegram messages over one long request; now and then the network between the server and Telegram stalls, that request gets no answer, and the next one works, so nothing was lost. Now:
  - a stall is silent; only Telegram being unreachable for 5 minutes or more is shown, once, in plain words ("Telegram didn't answer in time", "this server couldn't look up api.telegram.org", …), with a line when it's back;
  - after two stalls the agent talks to Telegram over IPv4 only (on servers with IPv6, such as Hetzner's, the IPv6 route to Telegram is the usual cause);
  - a revoked bot token, or the same bot used by another program, is said at once with what to do;
  - retries back off from 1 to 30 seconds.

  The agent is now version 1.3.3: the AI agent tab offers **Update agent**. Repeated lines in its activity list show once with a count.
- **Moving a container: "permission denied while trying to connect to the docker API".** Whether Docker needs sudo on a server was decided from the wording of Docker's error, which newer Docker versions changed, so the move ran without sudo. It's now decided by whether `docker ps` works for the login user. If sudo isn't allowed either, the message says what to change (add the user to the docker group, or allow sudo for the server).
- **Software installed from "+" works without reconnecting.** After installing Docker, its tab said Docker wasn't installed until you reconnected; Orbital now looks at Docker again whenever it looks for software.
- **The server tabs at the top line up with the window buttons** (with two or more servers open): the strip is as tall as the top row, and many tabs scroll in their own space instead of sliding under the sidebar and back/forward buttons.

## 1.3.5 (2026-11-21)

### Added
- **Move a container to another server** (Docker tab → a container → **Move to server…**): Orbital recreates it there with everything it needs: the image (pulled by its exact version when it came from a registry, otherwise copied), its volumes and bind-mounted files and folders, its network, ports, restart policy, labels, user, limits and environment. **Move** stops it here first so its data is copied as it is (and starts it here again if anything fails); **Copy** leaves it running. Before anything starts it shows what will be copied and what's in the way on the target (a container with that name, a port in use, files that would be overwritten). Environment variables travel in a private temporary file, so passwords in them never reach a command line or the activity log. Data goes through this computer; the servers don't connect to each other.
- **"You may try" in the "+" window**: software Orbital has a tab for that isn't on this server yet (Docker, nginx, PostgreSQL, MariaDB, Redis, certbot, Fail2ban, ufw, HAProxy, Apache), installed from the server's own package repositories and started with one click, then its tab opens. ufw is installed switched off; a second web server when port 80 is taken asks first.


## 1.3.4 (2026-11-21)

### Changed
- **Usage statistics go only to Orbital's own server**, never to an outside analytics service: `stats-server/` is one Python file with SQLite and a password-protected dashboard, set up with `install.sh`. Builds ask and send only when the `ORBITAL_STATS_URL` address is set (https only). What is sent is unchanged.

## 1.3.3 (2026-11-21)

### Added
- **Anonymous usage statistics, only if you agree.** Orbital asks once whether it may send: the app version, your OS, how long it's open, and how many servers are connected and saved (numbers only), with a random install id. Never server names, addresses, user names, files, commands or anything typed. Change it any time in Settings → Privacy.

### Changed
- README: Download first, a "Your data" section, and wording for Windows and Linux too.

## 1.3.2 (2026-11-20)

### Added
- **Network check** (new tab on every server, from "+"): is a host, port or web address reachable *from the server*? Port (nc), HTTP(S) with status and time (curl), DNS, ping and traceroute, each with its full output one click away; recent checks are remembered per server. Answers "why can't my app reach the database/API?" without typing commands.
- **Databases, Redis and Apache in Docker.** Their tabs now work inside the container: a picker next to the toolbar chooses this server or a container (shown when there is a choice). Inside a container Orbital uses the login the image was started with (POSTGRES_USER/POSTGRES_PASSWORD, MYSQL_ROOT_PASSWORD/MARIADB_ROOT_PASSWORD, REDIS_PASSWORD); those values stay in the container. One-click backups are for databases installed on the server itself.

### Changed
- **Updates arrive sooner**: Orbital checks at launch, every hour (was every six), and when you come back to it after 20 minutes or more; one small request each time, and each new version is announced once.

### Fixed
- **"Couldn't read Redis: Redis is not installed on this server"** (also Apache, PostgreSQL, MySQL, MongoDB) when the program runs in Docker. Detection saw the container's process on the host and offered the tab, but the tab looked for the client on the host. A process inside a container no longer counts as installed on the server; running containers whose image is the program (postgres, mariadb, redis, valkey, mongo, httpd, …) are attached to the tab instead.

## 1.3.1 (2026-11-19)

### Added
- **Live logs** for containers (Docker tab), services (Services tab) and the service of every tool tab: new lines appear as they're written, no Refresh needed; **Live** pauses and resumes. The view keeps the newest 2,000 lines and follows the end unless you scroll up.
- **Logs between two times** (the calendar button next to Logs): pick from and to, or the last 15 minutes, hour, 6 or 24 hours, in this computer's time zone (`docker logs --since/--until`, `journalctl --since/--until`, up to 5,000 lines).
- **Search a log and show only errors and warnings** (the filter button next to Logs); errors and warnings are coloured.
- **Save a log to a file** (the download button next to Logs): the lines shown, filters included.
- **Bookmarks in Files**: the ☆ next to the path saves the folder for this server; saved folders show as chips under the path, one click away.
- **Find files by name** (the magnifier in the Files toolbar): searches below the current folder on the server, case-insensitive, skipping .git and node_modules, up to 200 matches, nearest first. Click a match to open the file or folder.
- **Download a whole folder** (select a folder → **Download folder**): packed on the server and streamed as one .tar.gz into Downloads, with live bytes, and stoppable.
- **AI agent: daily summary** in Telegram at a time you choose (AI agent tab): health, disks, memory, failed services and waiting updates, once a day in the server's time. Built from the server's own data, so it uses no AI credit.
- **AI agent: Send a test message**: the agent messages your Telegram account, proving the whole path (server → Telegram → you) works without spending anything on AI.

### Changed
- **Change the AI agent's model** (AI agent tab → **Change model**): pick another model your key can use, for example Opus ↔ Sonnet, without typing the key again. The agent on the server lists the models (Orbital never keeps the key) and uses the new one from your next message, starting a fresh conversation.
- **AI agent setup: choose the model from a list.** Check now loads the models your key can use (newest first; for ChatGPT only models that can chat) and you pick one; Claude Opus 5.5 is preselected when the key has it. Install waits until the check passed.
- **Docker: one log view, resizable.** A selected container showed its log twice (in the crash explanation and in the Logs tab below). Now Logs is the top pane and Inspect/Compose the bottom one; drag the line between them to change their heights (double-click resets), fold either with its arrow, and the layout is remembered.
- **Server tabs in order of use**: Files, Monitor, Terminal, Services, Activity.
- **The "+" for tabs opens an "Add a tab" window**: a searchable grid of everything that can open here, the software found on the server (with whether it runs) and the tools every server has, scrollable, with Enter opening the first match. The old drop-down ran off the screen once the list grew.

### Fixed
- **AI agent: "this agent is too old to list models" with no way forward.** When a server runs an older agent, the AI agent tab now shows a banner with **Update agent** at the top (the button used to sit among the others), the Change-model dialog offers **Update agent now** right where the error appears, and an agent whose version can't be read counts as out of date instead of hiding the button.
- **AI agent: it started but never answered** ("Telegram.call() got multiple values for keyword argument 'timeout'"). Telegram's getUpdates has its own `timeout` parameter (how long to wait for new messages), and the agent's HTTP wait was also called `timeout`, so every poll failed. The agent is now version 1.3.1: on a server that runs the old one, the AI agent tab shows **Update agent**.
- Setting up the AI agent shows plain steps (setting up, installing, saving your settings, starting) instead of what the installer downloads.
- **AI agent: "the AI provider answered 400" said nothing useful.** The provider's own reason is shown now (for example "Your credit balance is too low…" or "OAuth authentication is currently not supported"), and a Claude subscription token (sk-ant-oat…, the kind Claude Code uses) is recognised before anything is sent, with where to create a real API key.


## 1.3.0 (2026-11-19)

### Added
- **AI agent in Telegram** (new tab on every server): ask about a server from Telegram, any time, in plain words ("is everything OK?", "why is the site slow?", "show nginx errors from the last hour"), answered by Claude (Claude Opus 5.5 by default) or ChatGPT with your own API key. The agent runs on the server itself, so it answers when this computer is off.
  - Set up in two steps: a bot from @BotFather (token) and an AI key; **Check** tests both before anything goes to the server. **Install** adds Python's AI libraries in their own folder (/opt/orbital-agent) and a service, with live progress.
  - **Paired to your Telegram account only**, with a one-time code (`/start CODE`, valid 15 minutes, 5 tries); everyone else gets no answer.
  - **It looks freely, and asks before changing anything**: restarting or stopping a service or container, or running a command, is shown in the chat with the reason and runs only after you tap **Approve**.
  - It never reads keys, passwords or its own config; the bot token and the AI key are stored only on the server, in a root-only file, written through stdin, and never kept in Orbital.
  - **Telegram alerts**: it messages you when a service fails, a disk is over 90% full or a container keeps restarting (checked every 5 minutes, each problem once, and again when it's fixed).
  - The tab shows who it answers, its recent activity (questions, what it looked at, approvals), and lets you make a new pairing code, turn alerts off, stop, update, change the bot or key, or remove it with everything it stored.


## 1.2.0 (2026-11-18)

### Added
- **Port forwarding is easy to find and use**: the Tunnels tab is now **Port forwarding**, and opens with a **From** (port on the server) → **To** (port on this computer) form, for example a container started with `-p 127.0.0.1:8080:8080` at `http://localhost:8080`. Docker shows each running container's published ports with a one-click forward (server-only ones marked); ⌘K has "Forward a port…".
- **tmux**: the Terminal tab's tmux menu lists the server's tmux sessions to reattach and opens a new named one; a tmux session keeps running when Orbital disconnects. Settings → "Open terminals in tmux" makes every new terminal a tmux session (orbital-1, orbital-2, …, so after a restart the first terminal picks its session up again). Without tmux on the server you get a normal shell and a note.
- **"Why did it stop?" for containers**: a container that crashed, keeps restarting or fails its health check gets a panel in Docker that explains it in plain words: the exit code (137 killed, often out of memory; 139 crashed; 127 command not found; …), whether Docker killed it for using too much memory and its memory limit, how many times it was restarted, and the usual causes found in its last log lines (a port already in use, a database it can't reach, a missing file or environment variable, permission denied), with those lines below. Checkup lists containers that keep restarting or stopped with an error.
- **Shell inside a container**: Docker → a running container → **Shell** opens a terminal in it (`docker exec`, bash if it has it, else sh).
- **Disk cleanup** tab: how much the system journal (beyond 7 days), the package download cache, rotated old logs and Docker leftovers use, and **Free** for each, as a job with live output. Files, databases, current logs and Docker volumes are never touched. Checkup's "disk full" now points here.
- **Stop a process** from Monitor → What uses the CPU and memory → Processes: asks, sends SIGTERM, and offers to force it if it doesn't stop. PID 1 and sshd are refused.
- **Mouse back and forward buttons** go back and forward through folders in Files too (and between tabs and pages as before). On macOS Orbital catches the buttons itself, since the web view doesn't pass them on reliably.

### Fixed
- Clicking a server several times while it was connecting opened several connections and tabs for it; now only the first click connects.


## 1.1.1 (2026-11-17)

### Added
- **Stop a running job.** Running jobs have **Stop** (in their progress bar and in the sidebar's Jobs list). It ends the command on the server (SIGTERM, passed on by sudo) or the transfer between chunks; the job shows "Stopped" and the activity log records it.
- **Updates** tab on every server: the packages that can be upgraded (security updates first), **Check for updates** (`apt-get update` / `dnf makecache`), **Install updates** as a job with live output (configuration files you changed are kept), and **Reboot** when the server needs one (type the server's name to confirm). apt, dnf and yum.
- **Checkup** tab on every server: one click looks at disk space, memory, CPU load, failed services, waiting (security) updates, a needed reboot, the firewall, risky SSH settings and expiring certificates, and lists what needs attention worst first, in plain words, each with a button to the tab that fixes it.
- **Accent colour** in Settings → Appearance: amber, blue, teal, green, violet, rose or graphite, for primary buttons, selection, charts and running jobs (the terminal cursor follows).
- **Send several files at once**: the send button (⇄) in the Files toolbar, next to the terminal button, opens a picker to tick one or more files and folders, across folders; they go in one job with one progress and one Stop. Also in ⌘K: "Send a file to another server…".
- Every file row shows its permissions (`rwxr-xr-x`) with rename, permissions and delete buttons; owner and modification time are in the tooltip and the bottom bar.

### Changed
- Monitor and the server header show memory as used **of** total (for example "9.0 GB of 16 GB").

### Fixed
- **Test renewal (and Renew now) took minutes**: without a terminal, certbot renew first sleeps a random 0–8 minutes, so Orbital now passes `--no-random-sleep-on-renew`. While another certbot run holds its lock, the certificates page keeps showing the last list with a "certbot is busy" note and checks again every 10 seconds, instead of showing nothing.
- The Test renewal button explains what it's for: a rehearsal against Let's Encrypt's test servers, to make sure automatic renewal will work.

## 1.1.0 (2026-11-17)

### Added
- **See what's happening during long operations.** Renewing or requesting a certificate, copying between servers, uploads, database backups, `docker compose pull`/`up`, container and service actions and the New website wizard now show what Orbital is doing in words ("Asking Let's Encrypt for a certificate for example.com …"), the latest line the tool printed (certbot's "Waiting for verification…"), files and bytes for transfers, and the time so far, with "Details" for everything it printed.
- **Jobs keep running when you move on, and tell you when they're done.** Leave the tab (or the server) while a job runs: when it finishes, a notification says how it went, with **Show** to go back to it; when Orbital is in the background the system notification appears too. The new **Jobs** list in the sidebar shows what is running and what finished on every server, with its current step and time.
- **Send files to another server** from the Files tab ("Send to server…" when a file or folder is selected): any saved SSH server, connected on the spot if needed, with a folder browser for the destination, live files and bytes, and the option to carry on in the background. Data passes through this computer; the servers never connect to each other.
- **Import from another SSH config file**: the import dialog shows which file it reads and can read another one (**Choose file…**), remembered until you switch back to `~/.ssh/config`; Includes inside it still resolve against `~/.ssh`.
- **Command suggestions in the terminal**: under the terminal, commands from the server's shell history (bash, zsh, fish) and this session that start with what you typed, and common next steps after the last command (after `nginx -t`, `systemctl reload nginx`; after `systemctl restart x`, its status and logs; after `git clone`, `cd` into it; and more). **→** at the end of the line or a click types it; Enter is always yours. Text counts as a command only when the shell echoed it, so passwords typed at a prompt are never kept; history lines that look like they hold a password, token or key are left out. Turn it off in Settings.
- **One-line install scripts** with no security warnings: `install.sh` (macOS: the .dmg into /Applications; Linux: the .deb, .rpm or AppImage, whichever fits) and `install.ps1` (Windows: a silent per-user install). Both download the latest release from GitHub and check its SHA-256 checksum first. macOS and Windows only warn about unsigned apps a browser downloaded, so these open normally. The README and every release page show the commands; `.github/workflows/install-scripts.yml` runs them on macOS, Ubuntu, Fedora, AppImage and Windows.

### Fixed
- Settings → About gave the Linux data folder as `~/.local/share/.orbital`; it is `~/.local/share/orbital`.
- Development builds (`npm run tauri dev`) used the installed app's data folder, so servers added while testing appeared in the installed app. They now keep their own (`.orbital-dev` beside `.orbital`; `orbital-dev` on Linux, `Orbital-dev` on Windows); the installed app's data stays as it is.

## 1.0.1 (2026-11-17)

### Added
- **Import from `~/.ssh/config`** (Welcome screen, the sidebar's "+" menu, or ⌘K): every `Host` in the file and the files it includes, with its address, user, port, key and route shown before anything is saved. `ProxyJump` (including chains such as `bastion,admin@10.0.0.5:2200`) and the old `ProxyCommand ssh -W %h:%p` form become jump hosts, created first and reused when they are already saved. Hosts already saved (same host, port and user) are marked and skipped. `Match` blocks, unsupported `ProxyCommand`s, loops and routes with more than three jump hosts are pointed out instead of guessed at. Only the config files are read; keys stay where they are and nothing secret is copied.
- **Uptime monitoring** (sidebar → Uptime, or ⌘K): add websites (HTTP or HTTPS, with the status code to expect and optionally text the page must contain) and ports (TCP `host:port`), checked every 1, 5 or 15 minutes from this Mac while Orbital is open. Each check shows its state, the last 30 results, a latency sparkline, uptime over 24 hours and 7 days and the last error; pause, edit, delete or check now. When a check goes down or comes back, macOS shows a notification, the app a toast, and the activity log records it (new "Uptime" category). A failure is retried once a few seconds later before it counts, and restarting Orbital doesn't repeat an alert. HTTPS checks warn when the certificate expires within 14 days. With a server connected, its nginx sites' domains are offered as one-click checks. Results are kept for 7 days, up/down changes for 90.
- **Monitor: memory details.** How much memory is installed and how much Linux can use, what's in use, what's available, and a breakdown (programs, cache and buffers, free) with programs' own data, shared memory, kernel, file cache, data waiting to be written, memory promised to programs, active/inactive and swap. The Memory card says how much is installed and the share in use.
- **SSH keys tab** on every server: the keys that can sign in to the account (comment, type, fingerprint, restrictions, and which are on this computer); allow a key (one of yours, or one pasted) and remove one. Removing the last key this computer signs in with asks you to type "lock me out". Make new Ed25519 keys on this computer, with an optional passphrase, without ssh-keygen; after allowing one, switch the server's profile to it in one click.
- **Snippets** (sidebar, or ⌘K): saved commands with `{{blanks}}` filled in when you run them, as root or not, on one server or several at once, with each server's output side by side. Values are always passed as one quoted word. Eight examples to start with (disk space, biggest folders, failed services, restart a service, recent errors, …). Every run is in the activity log per server.
- **Earlier versions of config files**: the editor's history button lists the backups safe save keeps (the last 5), compares one with the editor side by side, and restores it through safe save, so it's checked and the current version is kept.
- **New website** (nginx tab, or ⌘K → New website): one domain from nothing to HTTPS in four steps. Orbital creates the A records in Cloudflare when an account there has the domain (or shows the records to add elsewhere), waits until public resolvers answer with the server's address, writes an nginx server block from a template (a folder of static files, or a reverse proxy to a local port) through safe save, enables it on Debian-style layouts, runs `nginx -t` and removes it again if the test fails, reloads nginx, then requests the Let's Encrypt certificate through certbot's nginx plugin. Refuses domains nginx already serves, existing files, system folders and ports 22/80/443. Every step is in the activity log (`website.create`).

- **Linux**: `.deb`, `.rpm` and AppImage packages for x86_64 (pre-release versions such as 1.1.0-beta.1 ship without the `.rpm`: RPM doesn't allow `-` in a version; `scripts/linux-bundles.sh` decides) (WebKitGTK 4.1: Ubuntu 22.04+, Debian 12+, Fedora 38+). Data lives in `~/.local/share/orbital`; the vault is tied to the machine-id. The AppImage updates itself, .deb and .rpm installs update after asking for your password. CI and releases build them, and an install test installs the .deb, starts Orbital headless, checks its data folder and private vault, and removes it again.

### Fixed
- Windows: `Include` lines in `~/.ssh/config` found no files, so hosts from included files were missing from the import; paths are also shown as `~/.ssh/…` there.
- On Linux the vault no longer accepts an empty or "uninitialized" machine-id, which would have given every such machine the same key; WebKitGTK's DMA-BUF renderer is off by default, because it shows a blank window with NVIDIA drivers and in many VMs.
- Running Orbital from a development build showed a blank document as its icon in the Dock, ⌘Tab and Mission Control; it now shows Orbital's icon.

## 1.0.0 (2026-11-17)

The first release, for macOS and Windows.

### Added
- **Domains page** (sidebar, or ⌘K → Domains and DNS): connect Cloudflare with an API token and manage the DNS records of your domains: add, change and delete A, AAAA, CNAME, TXT and MX records, proxied or DNS only, with the connected servers' public addresses one click away. Saved records are checked against public resolvers (Cloudflare and Google, over HTTPS) every 10 seconds until they are live. "Check a domain" checks any name at any provider. Tokens go to the vault, behind the Touch ID gate; every change is in the activity log under DNS.
- **Windows**: a per-user installer (`Orbital_x.y.z_x64-setup.exe`, no admin rights needed) built by the release workflow next to the Mac app, with the same in-app updates. Data lives in `%LOCALAPPDATA%\Orbital`; the vault key uses the Windows machine GUID; SSH-agent sign-in uses the Windows OpenSSH agent or Pageant; shortcuts use Ctrl (Ctrl+K, Ctrl+N, Ctrl+Shift+S) and Alt+← / Alt+→ for back and forward. Touch ID is macOS-only.
- The HTTPS certificate dialog checks that each name already points at the server before asking Let's Encrypt, and links to Domains when one doesn't.
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
