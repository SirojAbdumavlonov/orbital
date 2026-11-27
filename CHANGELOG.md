# Changelog

All notable changes to Orbital are listed here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow [Semantic Versioning](https://semver.org/).

## Unreleased

## 1.10.0 (2026-11-27)

### Added
- **One date range for all monitoring** (Monitor, Home and Uptime): pick the last hour, 6 hours, a day, 7 days, 30 days or your own dates once, and every chart follows it. History is now kept 30 days (it was 24 hours) and includes the load and network traffic; what Orbital had recorded before is kept.
- **See any moment in history**: point at Monitor's charts to see that minute's numbers, the changes Orbital made then and the busiest programs at the time; click to keep it. Only program names are recorded, never their arguments, which can hold passwords.
- **Changes and restarts on the chart**: service and container actions, config saves, package changes, deploys and server restarts are marked along the chart, so a spike can be matched with what caused it.
- **Growth per disk**: Monitor's disk table shows how fast each disk grows a day over the last week, with a small trend and when it fills at that pace.
- **Compare servers** (Home): one metric (CPU, memory, disk, load or network) for up to six servers on one chart over the chosen dates, with averages and peaks.
- **Network chart**: Monitor's history shows network traffic in and out beside CPU, memory and disk, with averages and peaks for the range.
- **Export history as CSV**: save the chosen range of a server's history to a spreadsheet from Monitor.
- **Disk-full forecast**: at last week's pace, Monitor and the Home card say when the fullest disk fills up (within a month).
- **Restart alerts**: when a connected server restarts, you're told, and so are your Telegram, email or agent channels.
- **Long job alerts**: installs, backups, uploads and updates that ran 2 minutes or more report how they ended to the channels that ask for job alerts.
- **Quiet hours** for alert channels: at night (or any hours you choose) only problems are sent; recoveries, warnings and finished jobs are left out.
- **Copy ssh command**: one click in the sidebar copies the command for a terminal outside Orbital, with port, key and jump hosts.
- **Keyboard shortcuts sheet**: ⌘/ (Ctrl+/ on Windows and Linux) lists every shortcut anywhere in the app.
- **Files opens in your folder of choice** (Settings → Starting Orbital): the home folder, /, the last folder you had open on that server, or a fixed folder such as /var/www.

## 1.9.0 (2026-11-27)

### Added
- **Alerts on Telegram, by email or through the AI agent** (Settings → Monitoring → Also send alerts to): health and uptime alerts go out the moment they happen, to a Telegram bot of your own, any SMTP mailbox, or the AI agent's bot on one of your servers. Pick which alerts each one gets, send a test, and turn one off without deleting it. Tokens and passwords stay encrypted on this computer.

### Fixed
- **Empty space before the sidebar and back/forward buttons in a browser** (the demo): the room for the window buttons is only left in the desktop app on a Mac.
- **Server name stayed indented with the sidebar hidden**: when several servers are open, the name and address now line up with the tabs below them; only the top row leaves room for the window buttons.
- **Changing a process's priority or stopping it asked for a sudo password**: for your own processes it now runs as you, and uses sudo only when the server refuses.
- **The security audit could time out on big disks**: its file scans now skip Docker data, caches and other bulky folders, run side by side and stop after 25 seconds each. The report says when one was cut short.

## 1.8.0 (2026-11-26)

### Added
- **Processes** (a tab on every server): every process as a ranked list or a tree of which started which, refreshed every 5 seconds, with CPU, memory, run time and priority. Pick one to see its threads, open files, listening ports, service and working folder, then stop, kill, pause, resume, reload (HUP) or change its priority; PID 1 and sshd are protected.
- **Disks** (a tab on every server): disks and partitions with sizes, SSD or spinning and SMART health, every filesystem with space and inode use, and what /etc/fstab mounts at boot. Warnings for full disks, running out of inodes, failing disks and fstab entries that aren't mounted.
- **Packages** (a tab on every server): everything installed (apt or dnf) with versions and sizes, a search of the repositories, and install or remove after a dry run lists every package that would change. Removing anything the server needs to boot or let you in (openssh-server, sudo, systemd, the kernel, …) asks you to type "lock me out".
- **Certificate check** (⌘K → Certificate check): any HTTPS, mail or other TLS server's certificate, checked from this computer the way a browser would: trusted or why not, who issued it, which names it covers, days left, key, protocol and cipher, with the whole chain.
- **New timer** (Cron tab): schedule a command as a systemd timer instead of a cron line, with presets (every 15 minutes, daily, weekdays …), systemd checking the schedule and showing the next run as you type, output in the journal and missed runs caught up after a reboot. Timers Orbital made can be removed from the list.


## 1.7.0 (2026-11-26)

### Added
- **SQL console**: run a query against any PostgreSQL or MySQL/MariaDB database, on the server or in one of its containers, from the Query button in the database tab. Results come back as a table you can copy into a spreadsheet; read-only by default, so the database itself refuses changes until you allow them. ⌘↵ runs it, and recent queries are kept per database. The activity log records each query's kind, size and fingerprint, never its text.
- **One-click SSH hardening**: the SSH server tab's Harden button fixes root login, password login, attempts per connection, X11 forwarding and the login time in one step. The settings go into Orbital's own file in sshd_config.d, checked with sshd -t before sshd reloads (open connections stay); changes that would lock this connection out need you to type "lock me out", and one click removes the file again.
- **Security audit** (a tab on every server): a read-only scan for extra root accounts, empty passwords, readable password files, unusual setuid programs, world-writable files and folders, databases listening on every address, passwordless sudo, docker group members and SSH key files others can change, worst first.
- **Kernel tuning** (a tab on every server): the sysctl settings that matter for a server (network and kernel hardening, swappiness, connection queue, file watches, Redis's overcommit), with the value now and the recommended one. Apply them in one step through Orbital's own file in /etc/sysctl.d; if the kernel refuses a value, the previous file is put back. Add any other sysctl key too.
- **Terminal recordings**: turn on "Record terminals" (Settings, or Activity log → Terminal recordings) and every new terminal's output is saved on this computer, never your keystrokes. Play recordings back at 1–8× with a seek bar and long pauses shortened, save them as asciinema .cast files, or delete them; recorded terminals show a REC badge, and each recording is in the activity log.


## 1.6.0 (2026-11-24)

### Added
- **Apps** (Compose stacks → Apps): install Uptime Kuma, Portainer, Vaultwarden, Gitea, n8n, Grafana, WordPress, Nextcloud, Metabase or Adminer in one click. Each one runs as a Compose stack that only the server itself can reach (127.0.0.1). Open opens it in your browser through SSH, and Publish with a domain sets up New website with HTTPS. Passwords are generated for you and shown once. They live only in a private `.env` file on the server, never in Orbital or its activity log. The name, the port and free memory are checked first.
- **More software to install** (the "+" menu): Caddy, PHP-FPM, Node.js, Memcached, Supervisor, chrony time sync, automatic security updates, WireGuard and the Prometheus node exporter, from the server's own package repositories. Each one gets its own tab.
- **Docker comes with Compose**: installing Docker from the "+" menu now also installs the Compose plugin when the system packages it.
- **Logins** (a tab on every server): who is signed in now, recent sign-ins, and failed SSH attempts over the last 24 hours with the busiest addresses and the user names they try. With Fail2ban, ban an address in one click.
- **Search logs** (a tab on every server): find a word or an error across the system journal or the files in /var/log, over the last hour to 7 days, with the matches highlighted and files one click away.
- **Certificates on Home**: a server's card says when one of its HTTPS certificates expires within three weeks (or has expired); click it for the certificates tab.

### Fixed
- **PHP-FPM on Debian and Ubuntu wasn't found**: Orbital only looked for its unit under one name. Versioned names such as php8.2-fpm are now found too.
- **New service with a folder failed** ("WorkingDirectory= path is not absolute"): the folder was written in quotes, which systemd takes literally. Nothing was left behind (the unit check refused it and it was rolled back); now it works.

## 1.5.0 (2026-11-24)

### Added
- **New service** (Services → New service): turn a command into a systemd service that starts at boot and restarts when it stops, with its folder, user and environment. The unit is checked by `systemd-analyze verify` before it's kept; environment variables go into a root-only file (fine for passwords and tokens), never into the unit or the activity log. A name the system already uses is refused, so nothing is overwritten.
- **Compose stacks** (a tab on every server with Docker): every Compose project on the server, wherever it was deployed from, plus new ones made here in /opt/stacks/<name>. A new stack's file is checked by `docker compose config` before it's saved. Deploy, pull and redeploy, restart, stop and read the logs, with live output; edit the file in the editor.
- **Add swap** (Monitor, on servers without swap): a swap file of a suggested size, on now and after reboots, with low swappiness. Nothing changes when swap is already on or the disk hasn't room.
- **Updates on Home**: each connected server's card says how many updates are waiting (security ones counted) and whether it needs a reboot; click it for the Updates tab. Checked when a server connects and every hour.
- **Last 24 hours** (Monitor tab): CPU, memory and the fullest disk, one point a minute, recorded while Orbital is connected and kept on this computer; gaps show when it wasn't. Each server's card on Home shows its CPU trend for the last two hours.
- **Compare a file across servers** (editor → the compare button): the open file next to the same path, or any path, on another connected server, side by side. Spot configuration drift between servers before it bites.
- **Notes** (a tab on every server): what runs where, how to deploy, who to call, saved as you type on this computer. Servers with notes show a note mark on Home.
- **Allow an SSH key on several servers at once** (SSH keys → Allow a key → "Also allow it on"): the same key on other connected servers, each with its own checks and activity-log entry, and a summary of where it worked.
- **When to warn** (Settings → Monitoring): choose the disk, memory and load levels that turn a server orange or red (and alert).

### Fixed
- **nginx broken by any site whose certificate (or file) was deleted** ("cannot load certificate … No such file or directory"). New website and the Nginx tab now name the file that breaks nginx and why, and offer **Turn this site off**: only its link in sites-enabled is removed, the site's file is kept (turn it back on or create the site again later), and if nginx still fails the link is put back. Files in conf.d, which have no link, open in the editor instead. This replaces the default-site-only button.

## 1.4.7 (2026-11-22)

### Added
- **Containers count too**: the health dot and alerts include containers stuck restarting or reported unhealthy by their health check, and favourite actions can restart a Docker container.

### Fixed
- **Backups find databases wherever they run.** The new-backup dialog lists every PostgreSQL, MySQL/MariaDB and MongoDB Orbital finds: installed or running on the server, and in running Docker containers ("MongoDB in the container analytics-mongo"), then the databases of the one you pick. A database in a container is dumped and restored inside it (`docker exec`), with the login its image's variables give (POSTGRES_USER, MYSQL_ROOT_PASSWORD, …); the backup is kept on the server like the others. With none found, "A database" is off and says so. Saving a database backup is refused when its dump tool is missing (on the server or in the container).
- **Network check lost its results when you switched tabs**, including a traceroute still running. Results now stay per server: a check carries on while you're elsewhere and is there when you come back, along with what you typed. The last 30 results, with their output, are kept on this computer between launches (Clear empties the list).

## 1.4.6 (2026-11-22)

### Added
- **Scheduled backups** (Backups tab, on every server): back up a folder (as .tar.gz) or a PostgreSQL, MySQL/MariaDB or MongoDB database every day or every week at a time you choose, and keep the last few. Backups run on the server from cron, so they happen when Orbital is closed too; they're kept in /var/backups/orbital (root only). Run one now, see each job's last run (and why it failed), download a backup, or restore it: a folder into a new copy beside the original (or over it, after typing its name), a database after typing its name.
- **Health alerts**: a notification when a connected server needs attention for a minute or more (a disk nearly full, memory or load high) or a service fails, and when it's back to normal. Mute one server with the bell on its card on Home; turn them all off in Settings → Monitoring.
- **Search everywhere** (⌘K): type two letters to find services, containers, nginx sites, bookmarked folders and open files on every connected server; choosing one opens it with that item selected.
- **Split terminals**: show two of a server's terminals side by side or one above the other (buttons at the right of the terminal tabs). **Type in several terminals**: choose terminals on any servers that get the same keys as you type; each shows a stripe while it's on.

## 1.4.5 (2026-11-22)

### Added
- **nginx routes** (Nginx tab → a site → **Routes**): send a path to an app on a port (`/api` → `127.0.0.1:3000`, with WebSockets), serve a folder (`/static` → `/var/www/…`), or redirect (`/old` → `https://…`, 301/302/307/308). Add, change or remove routes; each change is previewed as the lines that change, written to the site's own file (the real file behind its sites-enabled link) through safe save: a backup, `nginx -t`, and rolled back if nginx rejects it, then nginx reloads. The rest of the file stays exactly as it was. Routes written by hand are listed (and can be removed); nginx's own files (`default`, nginx.conf) are never changed.
- **All your servers on Home**: a card for every saved server, connected ones first, with CPU, memory, the fullest disk and uptime, and what needs attention (a disk nearly full, memory or load high, a failed service). Click a card to open or connect.
- **Health dots in the sidebar**: a connected server's dot turns orange or red when something needs attention, with the reason under its name (for example "Disk / 93%" or "nginx failed"); hover for all of them. Failed services are checked every minute.
- **Reopen where you left off**: at launch, Orbital connects again to the servers that were open, on the same tab, folder and tool tabs, with the same one in front. Servers that need a password or a new host key are left for you (one note says which). Turn it off in Settings → Starting Orbital.
- **Favourite actions**: the ☆ at the end of a server's tabs adds a one-click button under its name: restart or reload a service, open a file or a folder, or run a snippet. Hover a button and click × to remove it.

### Changed
- **A dropped connection reconnects by itself.** When the Mac slept, the network changed or the server restarted, the next action used to fail with "the connection to this server was lost" and a Reconnect button. Now Orbital reconnects in the background (right away, then after 3 and 10 seconds) and finishes what you were doing on the new connection: same tab, folder, open files with unsaved edits, and selections. Terminals end with the connection, as before (tmux sessions carry on). Only when it can't reconnect, or needs you (a password that isn't saved, a changed host key), does it ask.

## 1.4.4 (2026-11-22)

### Fixed
- **Getting an HTTPS certificate could change nginx's `default` site.** certbot writes HTTPS into the server block that serves the name, and into the default site when no block does; deleting that certificate later broke nginx for every change ("cannot load certificate … No such file or directory"). Orbital now lets certbot change only the name's own site file. When no site of its own serves the name (or it's named in `default` or nginx.conf), it gets the certificate alone (`certbot certonly`) and changes no nginx file, and says so.
- **nginx broken by its default site** (it still points at a deleted certificate, so nothing else can change): New website now says so and offers **Turn off nginx's default site**. That removes only its link in `sites-enabled/`; the file itself stays exactly as it is, and if nginx still fails the link is put back. Orbital never edits or deletes nginx's own files (`default`, `default.conf`, `nginx.conf`); "Delete website…" refuses them.

## 1.4.3 (2026-11-22)

### Added
- **Hide the right-hand pane** to give the list the whole width: the editor in Files, the details panel in Services, Docker, Nginx, Users, Cron and the other list pages. Use the button at the right end of the tab row, or ⌘\ (Ctrl+\ on Windows and Linux). Each tab remembers its choice; opening a file shows the editor again.

### Changed
- **Installing from the "+" window shows how far it got**: a ring fills up with the percentage instead of a spinning icon. It follows apt's own download and install progress, or dnf/yum's package count, then the first-time setup and starting the service.

## 1.4.2 (2026-11-22)

### Added
- **Copy and paste in Files** (⌘C / ⌘V on macOS, Ctrl+C / Ctrl+V on Windows and Linux, or the new Paste button):
  - files and folders copied in Finder, Explorer or a Linux file manager are uploaded to the open folder;
  - a file or folder copied in Orbital is pasted into another folder on the same server, or into a folder on another connected server;
  - a copied screenshot or image is uploaded as a PNG.

  The most recent copy wins, so copying something on your computer after copying in Orbital pastes that. A name already in the folder gets " copy" added (like Finder), so nothing is replaced. Copies on the same server keep permissions and times, use sudo when the folder needs it, and are recorded in the activity log.

### Changed
- **Usage statistics never show an error.** Sending already failed silently (no network, the statistics server down); now saving your answer does too.

### Fixed
- **Deleting an HTTPS certificate could break nginx** ("cannot load certificate … No such file or directory"). The check that a certificate isn't in use only searched /etc/nginx and skipped folders reached through links, so a site that nginx loads from elsewhere was missed. It now asks nginx itself for every file it loads (`nginx -T`).

## 1.4.1 (2026-11-22)

### Added
- **What's new**: after an update, a window in the middle of the screen says what changed since the version you had before (every version in between), and the main page shows the new version's changes, each with a short description, until you remove the card with ×. The new **Changes** page in the sidebar lists every version at any time. Each version links to its full notes on GitHub; the notes come with the app, so they work offline.
- **Feedback** (sidebar, and ⌘K "Send feedback…"): report a problem, suggest an idea or say what works, about Orbital in general, one part of it, or one change from the Changes page (each has a **Feedback** link). Only what you type, the Orbital version and your system are sent, to Orbital's own server; an optional contact lets us answer. It doesn't depend on the usage statistics setting.
- **Help** (sidebar): getting started, common tasks, what to do about "Not connected", "Permission denied", cloud firewalls, host key warnings and certificates, keyboard shortcuts, and where your data is kept.
- **Home** at the top of the sidebar (and in ⌘K) goes back to the main page; open servers stay connected.

### Changed
- **The usage statistics question is asked once at all**: not again after an update, a restart or closing it without an answer. Settings → Privacy turns statistics on or off any time.

### Fixed
- **The Orbital logo on the main page could be missing.** In a window shorter than the page, the top of the page (the logo) was cut off instead of the page scrolling; the logo is now also built into the app instead of loaded as a separate file.

## 1.4.0 (2026-11-22)

### Added
- **Test a website** (Nginx tab → a site → **Test**, and by itself after New website creates a site or gets its certificate): nginx's own check (`nginx -t`), a request on the server for that name (is nginx serving it?), and one from this computer over the internet (are DNS and the firewalls right?). A failure says where to look, for example a cloud firewall such as Oracle's security list or Hetzner's firewall.
- **Delete a website** (Nginx tab → a site → **Delete website…**): removes its nginx file and the links that enable it, then reloads nginx. A copy goes to /var/backups/orbital-nginx, and if `nginx -t` fails without the site everything is put back. Optionally also deletes its HTTPS certificate and its folder under /var/www. You type the domain to confirm, and it says when the same file serves other names too.

### Fixed
- **"Not connected (session …)" when the connection dropped** (the Mac slept, the network changed or the server restarted). The message now says so in plain words, any screen that finds the connection gone marks the server as disconnected at once, and **Reconnect** keeps your tab, folder, selections and an open New website dialog with everything filled in (which also has its own Reconnect button now).

### Fixed
- **HTTPS certificate: "points at <your server's address>, not this server".** The check asks two public resolvers and called the name "not this server" unless both confirmed, so one resolver that was slow or hadn't seen a new record yet made a correct record look wrong. Now a name points here when a resolver sees this server's address and none sees a different one (a lagging resolver is mentioned, not held against it); "not this server" appears only for a different address, and names it. New website goes on as soon as the record points here, while the Domains check keeps waiting until every resolver agrees.

### Added
- **Adjust a container for its new server before moving it** (Move to server… → "Adjust for the new server"):
  - **ports**: publish each one on every address (public) or on this server only, on any port number, or not at all; public ports come with a reminder about the cloud firewall (for example Oracle Cloud's security list);
  - **arguments** after the image: edit, remove or add (for example drop `--trusted-origins old.domain`);
  - **environment variables**: change, remove or add; values that look like passwords or tokens stay hidden and are kept unless you type a new one.

  The preview checks the target again as you edit (a port already in use there, …). It also points out what may still tie the container to the old server: domain names in its arguments or variables, and nginx sites on the old server that send a domain to this container's port (their certificate and DNS record stay behind).

### Changed
- **Switching tabs keeps your place.** Coming back to Docker, Services, Nginx, Users, Cron, Ports, Certificates, Fail2ban, Deploy, Updates, SSH server, Files or Monitor shows the same container, service or row selected, with the same search, filters, sort and sub-tab, while Orbital is open (forgotten when you disconnect). An open Move dialog stays open too.

## 1.3.9 (2026-11-21)

### Fixed
- **Files: "permission denied" in folders only root can change, even with sudo allowed.** The Files tab works over SFTP, which always runs as the login user and can't use sudo. Now, when the server refuses and the server's profile allows sudo, every action is done with sudo instead:
  - creating files and folders, renaming, deleting and permissions;
  - saving (in place, so the file keeps its owner and mode);
  - opening and listing folders the login user can't read;
  - downloading files and whole folders;
  - uploads and server-to-server copies into protected folders (sent to a private folder in /tmp first, then put in place with sudo and the temporary copy removed);
  - finding files (now also searches folders only root can read).

  Files and folders made this way belong to root, as with sudo on the command line. Each action is still recorded in the activity log.
- **Sudo when the server refuses, everywhere else too:**
  - **Deploy**: a step refused as the login user (an app folder owned by root, git's "dubious ownership" check, `EACCES` from npm) runs once more with sudo, and the log says so; git trusts the folder for that one run only. The checkout view (commit, changes) reads the same way.
  - **Snippets** run without sudo that the server refuses show **Run again with sudo** (not automatic, since running a command twice isn't always safe).
  - **Software detection** looks as root when sudo works, so config files only root can read are found.

  Reading screens (disk usage, processes, ports, users, cron, logs, nginx, sshd, updates, cleanup, databases) already looked as root when sudo works, and changes to the system (services, Docker, firewall, packages, certificates, users) already used sudo.

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
