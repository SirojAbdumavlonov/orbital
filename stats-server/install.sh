#!/bin/sh
# Install (or update) Orbital's usage statistics server on this Linux server.
# Run as root from the folder holding orbital_stats.py:   sudo sh install.sh
#
# It creates the system user orbital-stats, puts the server in /opt/orbital-stats,
# its SQLite file in /var/lib/orbital-stats, the dashboard password in
# /etc/orbital-stats.env (0600, made once), and a systemd service listening on
# 127.0.0.1:8787. Put nginx with HTTPS in front (Orbital → Nginx → New website →
# "Reverse proxy" to port 8787, then Get certificate).
set -eu

[ "$(id -u)" = 0 ] || { echo "run as root (sudo sh install.sh)"; exit 1; }
here=$(cd "$(dirname "$0")" && pwd)
[ -f "$here/orbital_stats.py" ] || { echo "orbital_stats.py must be next to this script"; exit 1; }
command -v python3 >/dev/null 2>&1 || { echo "python3 is needed (apt install python3)"; exit 1; }
python3 -I "$here/orbital_stats.py" selftest

id orbital-stats >/dev/null 2>&1 || useradd --system --home-dir /var/lib/orbital-stats --shell /usr/sbin/nologin orbital-stats
install -d -o root -g root -m 0755 /opt/orbital-stats
install -o root -g root -m 0644 "$here/orbital_stats.py" /opt/orbital-stats/orbital_stats.py
install -d -o orbital-stats -g orbital-stats -m 0700 /var/lib/orbital-stats

if [ ! -f /etc/orbital-stats.env ]; then
  token=$(head -c 24 /dev/urandom | od -An -tx1 | tr -d ' \n')
  umask 077
  printf 'ORBITAL_STATS_TOKEN=%s\nORBITAL_STATS_DB=/var/lib/orbital-stats/stats.db\nORBITAL_STATS_PORT=8787\n' "$token" > /etc/orbital-stats.env
  new=1
fi
chmod 0600 /etc/orbital-stats.env

cat > /etc/systemd/system/orbital-stats.service <<'UNIT'
[Unit]
Description=Orbital usage statistics
After=network.target

[Service]
User=orbital-stats
EnvironmentFile=/etc/orbital-stats.env
ExecStart=/usr/bin/python3 -I /opt/orbital-stats/orbital_stats.py
Restart=on-failure
NoNewPrivileges=yes
ProtectSystem=strict
ProtectHome=yes
PrivateTmp=yes
ReadWritePaths=/var/lib/orbital-stats

[Install]
WantedBy=multi-user.target
UNIT
systemctl daemon-reload
systemctl enable --now orbital-stats >/dev/null 2>&1
systemctl restart orbital-stats
sleep 1
curl -fsS http://127.0.0.1:8787/healthz >/dev/null 2>&1 && echo "orbital-stats is running on 127.0.0.1:8787" || { echo "orbital-stats didn't start:"; journalctl -u orbital-stats -n 20 --no-pager; exit 1; }

if [ "${new:-}" = 1 ]; then
  echo
  echo "Dashboard sign-in: user \"orbital\", password:"
  echo "  $token"
  echo "(kept in /etc/orbital-stats.env; this is the only time it's printed)"
fi
echo
echo "Next: give it a domain with HTTPS (Orbital → Nginx → New website → reverse proxy to port 8787),"
echo "then set the GitHub variable ORBITAL_STATS_URL to https://<that domain>."
