#!/usr/bin/env bash
# Start throwaway FTP, WebDAV and S3 servers on 127.0.0.1 for the live storage
# tests (crates/orbital-core/tests/live_storage.rs). No root needed.
#
#   bash scripts/it-storage.sh            # prints the variables to export
#   cargo test -p orbital-core --test live_storage -- --test-threads=1
#   bash scripts/it-storage.sh stop
#
# The servers: pyftpdlib (FTP), WsgiDAV (WebDAV), moto (S3). User "orbital",
# password "secret" (any key works on moto).
set -euo pipefail
DIR="${ORBITAL_IT_STORAGE_DIR:-${TMPDIR:-/tmp}/orbital-it-storage}"
if [[ "${1:-}" == "stop" ]]; then
  for f in "$DIR"/*.pid; do [[ -f "$f" ]] && kill "$(cat "$f")" 2>/dev/null || true; rm -f "$f"; done
  exit 0
fi
mkdir -p "$DIR/ftp" "$DIR/dav"
if [[ ! -x "$DIR/venv/bin/python" ]]; then
  python3 -m venv "$DIR/venv"
  "$DIR/venv/bin/pip" install -q pyftpdlib wsgidav cheroot "moto[server]"
fi
PY="$DIR/venv/bin/python"

cat > "$DIR/ftp.py" <<PYEOF
from pyftpdlib.authorizers import DummyAuthorizer
from pyftpdlib.handlers import FTPHandler
from pyftpdlib.servers import FTPServer
a = DummyAuthorizer()
a.add_user("orbital", "secret", "$DIR/ftp", perm="elradfmwMT")
h = FTPHandler
h.authorizer = a
h.passive_ports = range(30000, 30100)
FTPServer(("127.0.0.1", 2121), h).serve_forever()
PYEOF
cat > "$DIR/dav.yaml" <<YEOF
host: 127.0.0.1
port: 8022
provider_mapping:
  "/": "$DIR/dav"
http_authenticator:
  domain_controller: null
  accept_basic: true
  accept_digest: false
  default_to_digest: false
simple_dc:
  user_mapping:
    "*":
      "orbital":
        password: "secret"
verbose: 1
YEOF

"$PY" "$DIR/ftp.py" >"$DIR/ftp.log" 2>&1 & echo $! >"$DIR/ftp.pid"
"$DIR/venv/bin/wsgidav" --config "$DIR/dav.yaml" >"$DIR/dav.log" 2>&1 & echo $! >"$DIR/dav.pid"
"$DIR/venv/bin/moto_server" -H 127.0.0.1 -p 9022 >"$DIR/s3.log" 2>&1 & echo $! >"$DIR/s3.pid"
sleep 3
echo "export ORBITAL_IT_FTP_PORT=2121 ORBITAL_IT_DAV_PORT=8022 ORBITAL_IT_S3_PORT=9022"
