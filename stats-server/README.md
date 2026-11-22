# Orbital usage statistics server

Where Orbital's anonymous usage statistics go: your own server, no outside service. One Python file (standard library only) and a SQLite database.

Orbital sends something only when the user said yes (it asks once; Settings → Privacy changes it), and only these fields: a random install id, a per-run session id, the event (`opened`, or `active` every 30 minutes), the app version, OS, processor type, and how many servers are connected and saved. The server keeps exactly those fields (anything else is dropped), stores no IP addresses and writes no request log, and deletes events after 400 days.

## Feedback

The same server receives what people write on Orbital's **Feedback** page (`POST /v1/feedback`), only when they click Send: the kind (problem, idea, praise, other), what it's about, the message (up to 4000 characters), an optional contact for a reply, and the Orbital version and OS. No IP addresses either; at most 60 messages an hour are taken (a spam brake). They're listed, newest first, at the bottom of the dashboard and as JSON at `GET /v1/feedback` (same sign-in). Updating an existing server adds the table on start; nothing else to do.

## Set it up

1. On your server, from a copy of this folder:

   ```bash
   sudo sh install.sh
   ```

   It installs `orbital-stats` as a systemd service on `127.0.0.1:8787` and prints the dashboard password once (it's kept in `/etc/orbital-stats.env`).

2. Give it a domain with HTTPS. In Orbital: the server's **Nginx** tab → **New website** → domain (for example `stats.example.com`) → **Reverse proxy** to port **8787** → **Get certificate**. To keep visitors' IP addresses out of nginx's log too, add `access_log off;` inside that site's `server { … }` block.

3. In GitHub: **Settings → Secrets and variables → Actions → Variables** → add `ORBITAL_STATS_URL` = `https://stats.example.com`. Releases built after that ask users and send here.

## The dashboard

Open `https://stats.example.com/` and sign in as `orbital` with the password from step 1: people all time, active today/this week/this month, hours used, servers per person, the last 14 days, versions and systems, then the feedback. `GET /v1/summary` gives the same numbers as JSON.

Update the server by running `install.sh` again from a newer copy; the password and data are kept.
