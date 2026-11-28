# A disposable Ubuntu 24.04 server with systemd, for running the live SSH tests
# on a Mac (scripts/it-docker.sh). Not for production.
FROM ubuntu:24.04
ENV DEBIAN_FRONTEND=noninteractive container=docker
RUN apt-get update && apt-get install -y --no-install-recommends \
      systemd systemd-sysv dbus openssh-server sudo ufw fail2ban iptables nftables \
      nginx certbot python3-certbot-nginx python3-venv python3-pip tmux cron rsyslog \
      docker.io docker-compose-v2 curl ca-certificates procps iproute2 lsof less \
      postgresql-client smartmontools apt-utils git \
    && apt-get clean && rm -rf /var/lib/apt/lists/*
# systemd as PID 1; no getty on a container.
RUN systemctl mask getty@.service console-getty.service systemd-logind.service >/dev/null 2>&1 || true \
    && systemctl enable ssh fail2ban docker nginx cron rsyslog
STOPSIGNAL SIGRTMIN+3
CMD ["/sbin/init"]
