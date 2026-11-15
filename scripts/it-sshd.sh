#!/usr/bin/env bash
# Starts a disposable OpenSSH server on 127.0.0.1:2222 for the live integration
# tests (Linux, run as root — this is what CI does). Prints the env vars to export.
set -euo pipefail
USER_NAME=orbital
PASS="${ORBITAL_IT_PASS:-s3cret-Pass}"
KEYDIR="${KEYDIR:-/tmp/orbital-it-keys}"

id "$USER_NAME" >/dev/null 2>&1 || useradd -m -s /bin/bash "$USER_NAME"
echo "$USER_NAME:$PASS" | chpasswd
echo "$USER_NAME ALL=(ALL) ALL" > /etc/sudoers.d/$USER_NAME && chmod 440 /etc/sudoers.d/$USER_NAME
mkdir -p /run/sshd "$KEYDIR" /home/$USER_NAME/.ssh
[ -f "$KEYDIR/id_ed25519" ] || ssh-keygen -q -t ed25519 -N '' -f "$KEYDIR/id_ed25519"
[ -f "$KEYDIR/id_enc" ] || ssh-keygen -q -t ed25519 -N 'keypass' -f "$KEYDIR/id_enc"
cat "$KEYDIR/id_ed25519.pub" "$KEYDIR/id_enc.pub" > /home/$USER_NAME/.ssh/authorized_keys
chown -R $USER_NAME:$USER_NAME /home/$USER_NAME/.ssh
chmod 700 /home/$USER_NAME/.ssh && chmod 600 /home/$USER_NAME/.ssh/authorized_keys
chmod 644 "$KEYDIR"/*.pub && chmod 644 "$KEYDIR/id_ed25519" "$KEYDIR/id_enc"
ssh-keygen -A >/dev/null
mkdir -p /etc/orbital-it && echo 'mode=ok' > /etc/orbital-it/app.conf && chmod 644 /etc/orbital-it/app.conf

cat > /tmp/orbital-sshd.conf <<CONF
Port 2222
ListenAddress 127.0.0.1
HostKey /etc/ssh/ssh_host_ed25519_key
PasswordAuthentication yes
KbdInteractiveAuthentication no
PubkeyAuthentication yes
UsePAM yes
Subsystem sftp internal-sftp
PidFile /tmp/orbital-sshd.pid
CONF
pkill -f orbital-sshd.conf || true
/usr/sbin/sshd -f /tmp/orbital-sshd.conf -E /tmp/orbital-sshd.log
sleep 1
cat <<ENV
export ORBITAL_IT_HOST=127.0.0.1 ORBITAL_IT_PORT=2222 ORBITAL_IT_USER=$USER_NAME ORBITAL_IT_PASS='$PASS'
export ORBITAL_IT_KEY=$KEYDIR/id_ed25519 ORBITAL_IT_ENC_KEY=$KEYDIR/id_enc
ENV
