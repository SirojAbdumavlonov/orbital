#!/usr/bin/env bash
# Runs the live SSH tests' server in Docker on a Mac (or any machine with
# Docker): Ubuntu 24.04 with systemd, sshd on 127.0.0.1:2222, Fail2ban, ufw,
# nginx, Docker and cron, set up like scripts/it-sshd.sh does on Linux.
#   bash scripts/it-docker.sh        # start; prints the variables to export
#   bash scripts/it-docker.sh stop   # remove it
set -euo pipefail
NAME=orbital-it
if [ "${1:-}" = stop ]; then docker rm -f "$NAME" >/dev/null 2>&1 || true; exit 0; fi
DIR=$(cd "$(dirname "$0")" && pwd)
KEYDIR="${KEYDIR:-${TMPDIR:-/tmp}/orbital-it-keys}"
PASS="${ORBITAL_IT_PASS:-s3cret-Pass}"
docker build -q -t orbital-it:24.04 "$DIR/it-docker" >/dev/null
docker rm -f "$NAME" >/dev/null 2>&1 || true
docker run -d --name "$NAME" --privileged --cgroupns=host -v /sys/fs/cgroup:/sys/fs/cgroup:rw \
  --tmpfs /run --tmpfs /run/lock -v /var/lib/docker -v /var/lib/containerd \
  -p 127.0.0.1:2222:2222 orbital-it:24.04 >/dev/null
for _ in $(seq 1 40); do docker exec "$NAME" systemctl is-system-running 2>/dev/null | grep -qE 'running|degraded' && break; sleep 1; done
mkdir -p "$KEYDIR"
[ -f "$KEYDIR/id_ed25519" ] || ssh-keygen -q -t ed25519 -N '' -f "$KEYDIR/id_ed25519"
[ -f "$KEYDIR/id_enc" ] || ssh-keygen -q -t ed25519 -N 'keypass' -f "$KEYDIR/id_enc"
docker exec -i "$NAME" bash -s <<SETUP
set -e
id orbital >/dev/null 2>&1 || useradd -m -s /bin/bash orbital
echo 'orbital:$PASS' | chpasswd
echo 'orbital ALL=(ALL) ALL' > /etc/sudoers.d/orbital && chmod 440 /etc/sudoers.d/orbital
mkdir -p /home/orbital/.ssh
echo '$(cat "$KEYDIR/id_ed25519.pub")' > /home/orbital/.ssh/authorized_keys
echo '$(cat "$KEYDIR/id_enc.pub")' >> /home/orbital/.ssh/authorized_keys
chown -R orbital:orbital /home/orbital/.ssh && chmod 700 /home/orbital/.ssh && chmod 600 /home/orbital/.ssh/authorized_keys
mkdir -p /etc/orbital-it && echo 'mode=ok' > /etc/orbital-it/app.conf && chmod 644 /etc/orbital-it/app.conf
# Port 2222 as in CI (the tests check it from inside the server too); 22 as usual.
printf 'Port 22\nPort 2222\nPasswordAuthentication yes\nKbdInteractiveAuthentication no\n' > /etc/ssh/sshd_config.d/10-orbital-it.conf
# Ubuntu 24.04 starts sshd from ssh.socket, which listens on 22 only: use the service.
systemctl disable --now ssh.socket >/dev/null 2>&1 || true
systemctl enable ssh >/dev/null 2>&1 || true
usermod -aG docker orbital || true
# ufw on, with SSH allowed, as on a real server.
ufw allow 22/tcp >/dev/null && ufw allow 2222/tcp >/dev/null && ufw --force enable >/dev/null || true
systemctl restart ssh fail2ban
SETUP
sleep 2
cat <<ENV
export ORBITAL_IT_HOST=127.0.0.1 ORBITAL_IT_PORT=2222 ORBITAL_IT_USER=orbital ORBITAL_IT_PASS='$PASS'
export ORBITAL_IT_KEY=$KEYDIR/id_ed25519 ORBITAL_IT_ENC_KEY=$KEYDIR/id_enc
export ORBITAL_IT_UFW=1 ORBITAL_IT_FAIL2BAN=1 ORBITAL_IT_USERS=1 ORBITAL_IT_NGINX=1
ENV
