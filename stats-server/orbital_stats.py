#!/usr/bin/env python3
"""Orbital's usage statistics server: yours, on your server, nothing else involved.

Orbital (only where the user said yes) sends small JSON events here:

    POST /v1/events
    {"install": "<uuid>", "session": "<uuid>", "event": "opened" | "active",
     "version": "1.3.4", "os": "macos", "arch": "aarch64",
     "connected": 2, "saved": 7}

Only those fields are kept, each checked; anything else is dropped. No IP
address, no request log, no cookies. Events are kept for 400 days.

    GET /            the dashboard (HTTP Basic auth: user "orbital", the token)
    GET /v1/summary  the same numbers as JSON (same auth)
    GET /healthz     "ok"

Python 3.8+ standard library only. Settings come from the environment:

    ORBITAL_STATS_TOKEN   dashboard password (required to see anything)
    ORBITAL_STATS_DB      SQLite file (default /var/lib/orbital-stats/stats.db)
    ORBITAL_STATS_PORT    port on 127.0.0.1 (default 8787); put nginx with HTTPS in front

`python3 orbital_stats.py selftest` checks it without a network.
"""

import base64
import hmac
import html
import json
import os
import re
import sqlite3
import sys
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

UUID = re.compile(r"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$")
VERSION = re.compile(r"^[0-9A-Za-z.+-]{1,32}$")
WORD = re.compile(r"^[a-z0-9_]{1,16}$")
EVENTS = ("opened", "active")
KEEP_DAYS = 400
MAX_BODY = 4096
DAY = 86400

SCHEMA = """
CREATE TABLE IF NOT EXISTS events (
  at INTEGER NOT NULL,          -- server time, seconds
  install TEXT NOT NULL,
  session TEXT NOT NULL,
  event TEXT NOT NULL,
  version TEXT NOT NULL,
  os TEXT NOT NULL,
  arch TEXT NOT NULL,
  connected INTEGER NOT NULL,
  saved INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS events_at ON events(at);
CREATE INDEX IF NOT EXISTS events_install ON events(install, at);
"""


class Store:
    def __init__(self, path):
        if path != ":memory:":
            os.makedirs(os.path.dirname(path) or ".", exist_ok=True)
        self.db = sqlite3.connect(path, check_same_thread=False)
        self.db.executescript(SCHEMA)
        self.lock = threading.Lock()
        self.pruned = 0

    def add(self, e, now=None):
        now = int(now if now is not None else time.time())
        with self.lock, self.db:
            self.db.execute(
                "INSERT INTO events VALUES (?,?,?,?,?,?,?,?,?)",
                (now, e["install"], e["session"], e["event"], e["version"], e["os"], e["arch"], e["connected"], e["saved"]),
            )
            if now - self.pruned > DAY:
                self.db.execute("DELETE FROM events WHERE at < ?", (now - KEEP_DAYS * DAY,))
                self.pruned = now

    def summary(self, now=None):
        now = int(now if now is not None else time.time())
        q = lambda sql, *a: self.db.execute(sql, a).fetchall()  # noqa: E731
        one = lambda sql, *a: q(sql, *a)[0][0] or 0  # noqa: E731
        active = lambda days: one("SELECT COUNT(DISTINCT install) FROM events WHERE at >= ?", now - days * DAY)  # noqa: E731
        with self.lock:
            firsts = q("SELECT install, MIN(at) FROM events GROUP BY install")
            new_by_day = {}
            for _, first in firsts:
                if first >= now - 14 * DAY:
                    d = time.strftime("%Y-%m-%d", time.gmtime(first))
                    new_by_day[d] = new_by_day.get(d, 0) + 1
            daily = []
            for i in range(13, -1, -1):
                start = (now // DAY - i) * DAY
                n = one("SELECT COUNT(DISTINCT install) FROM events WHERE at >= ? AND at < ?", start, start + DAY)
                d = time.strftime("%Y-%m-%d", time.gmtime(start))
                daily.append({"day": d, "people": n, "new": new_by_day.get(d, 0)})
            # Each "active" is 30 minutes of Orbital being open.
            hours30 = one("SELECT COUNT(*) FROM events WHERE event = 'active' AND at >= ?", now - 30 * DAY) / 2
            latest = "SELECT e.* FROM events e JOIN (SELECT install, MAX(at) m FROM events WHERE at >= ? GROUP BY install) l ON e.install = l.install AND e.at = l.m"
            rows = q(latest, now - 30 * DAY)
            by = lambda col: sorted(  # noqa: E731
                ((k, sum(1 for r in rows if r[col] == k)) for k in {r[col] for r in rows}), key=lambda kv: -kv[1]
            )
            people30 = len(rows)
            return {
                "people": one("SELECT COUNT(DISTINCT install) FROM events"),
                "activeToday": active(1),
                "active7": active(7),
                "active30": people30,
                "hours30": hours30,
                "hoursPerPerson30": round(hours30 / people30, 1) if people30 else 0,
                "avgConnected30": round(one("SELECT AVG(connected) FROM events WHERE at >= ?", now - 30 * DAY), 1),
                "avgSaved30": round(sum(r[8] for r in rows) / people30, 1) if people30 else 0,
                "versions": by(4),
                "os": by(5),
                "daily": daily,
            }


def clean(body):
    """The event as stored, or None if anything is off."""
    try:
        e = json.loads(body)
    except (ValueError, UnicodeDecodeError):
        return None
    if not isinstance(e, dict):
        return None
    try:
        out = {
            "install": str(e["install"]),
            "session": str(e["session"]),
            "event": str(e["event"]),
            "version": str(e["version"]),
            "os": str(e["os"]),
            "arch": str(e["arch"]),
            "connected": e["connected"],
            "saved": e["saved"],
        }
    except KeyError:
        return None
    ok = (
        UUID.match(out["install"])
        and UUID.match(out["session"])
        and out["event"] in EVENTS
        and VERSION.match(out["version"])
        and WORD.match(out["os"])
        and WORD.match(out["arch"])
        and all(isinstance(out[k], int) and not isinstance(out[k], bool) and 0 <= out[k] <= 100000 for k in ("connected", "saved"))
    )
    return out if ok else None


def page(s):
    rows = "".join(
        f"<tr><td>{d['day']}</td><td>{d['people']}</td><td>{d['new']}</td>"
        f"<td><div class=bar style='width:{min(100, d['people'] * 100 // max(1, max(x['people'] for x in s['daily'])))}%'></div></td></tr>"
        for d in s["daily"]
    )
    lst = lambda kv: "".join(f"<li><b>{html.escape(k)}</b> {n}</li>" for k, n in kv) or "<li>—</li>"  # noqa: E731
    card = lambda label, value, hint="": f"<div class=card><small>{label}</small><b>{value}</b><span>{hint}</span></div>"  # noqa: E731
    return f"""<!doctype html><meta charset=utf-8><meta name=viewport content="width=device-width,initial-scale=1">
<title>Orbital usage</title>
<style>
:root{{--bg:#12161d;--card:#1a2029;--ink:#e6e9ef;--dim:#8a93a3;--amber:#f0b44c;--line:#262e3a}}
@media (prefers-color-scheme: light){{:root{{--bg:#f4f5f7;--card:#fff;--ink:#1b2230;--dim:#667085;--line:#e3e6eb}}}}
body{{margin:0;background:var(--bg);color:var(--ink);font:14px/1.5 -apple-system,"IBM Plex Sans",system-ui,sans-serif}}
main{{max-width:960px;margin:0 auto;padding:24px 16px}}h1{{font-size:20px;margin:0 0 4px}}p{{color:var(--dim);margin:0 0 20px}}
.cards{{display:grid;grid-template-columns:repeat(auto-fit,minmax(170px,1fr));gap:10px;margin-bottom:22px}}
.card{{background:var(--card);border:1px solid var(--line);border-radius:10px;padding:12px 14px;display:flex;flex-direction:column}}
.card small{{color:var(--dim)}}.card b{{font-size:26px}}.card span{{color:var(--dim);font-size:12px}}
.cols{{display:grid;grid-template-columns:2fr 1fr;gap:16px}}@media (max-width:700px){{.cols{{grid-template-columns:1fr}}}}
table{{width:100%;border-collapse:collapse}}td,th{{padding:5px 6px;border-bottom:1px solid var(--line);text-align:left;font-variant-numeric:tabular-nums}}
th{{color:var(--dim);font-weight:500}}.bar{{height:8px;background:var(--amber);border-radius:4px;min-width:2px}}
ul{{list-style:none;padding:0;margin:0 0 16px}}li{{padding:3px 0;border-bottom:1px solid var(--line)}}li b{{display:inline-block;min-width:90px}}
h2{{font-size:14px;color:var(--dim);font-weight:500;margin:0 0 8px}}
</style>
<main>
<h1>Orbital usage</h1><p>Anonymous counts from people who agreed to share them. Times in UTC.</p>
<div class=cards>
{card("People, all time", s["people"])}
{card("Active today", s["activeToday"], "last 24 hours")}
{card("Active this week", s["active7"], "last 7 days")}
{card("Active this month", s["active30"], "last 30 days")}
{card("Hours used", f"{s['hours30']:g}", f"last 30 days · {s['hoursPerPerson30']:g} h per person")}
{card("Servers per person", f"{s['avgSaved30']:g}", f"saved · {s['avgConnected30']:g} connected at a time")}
</div>
<div class=cols>
<div><h2>Last 14 days</h2><table><tr><th>Day</th><th>People</th><th>New</th><th></th></tr>{rows}</table></div>
<div><h2>Versions (last 30 days)</h2><ul>{lst(s["versions"])}</ul><h2>Systems</h2><ul>{lst(s["os"])}</ul></div>
</div>
</main>"""


def make_handler(store, token):
    class Handler(BaseHTTPRequestHandler):
        server_version = "orbital-stats"
        sys_version = ""

        def log_message(self, *args):  # no request log: it would hold IP addresses
            pass

        def send(self, code, body=b"", ctype="text/plain; charset=utf-8", extra=()):
            self.send_response(code)
            self.send_header("Content-Type", ctype)
            self.send_header("Content-Length", str(len(body)))
            self.send_header("Cache-Control", "no-store")
            for k, v in extra:
                self.send_header(k, v)
            self.end_headers()
            self.wfile.write(body)

        def authorized(self):
            if not token:
                return False
            got = self.headers.get("Authorization", "")
            want = "Basic " + base64.b64encode(f"orbital:{token}".encode()).decode()
            return hmac.compare_digest(got.encode(), want.encode())

        def do_POST(self):
            if self.path != "/v1/events":
                return self.send(404)
            n = int(self.headers.get("Content-Length") or 0)
            if n <= 0 or n > MAX_BODY:
                return self.send(413 if n > MAX_BODY else 400)
            e = clean(self.rfile.read(n))
            if e is None:
                return self.send(400)
            store.add(e)
            self.send(204)

        def do_GET(self):
            if self.path == "/healthz":
                return self.send(200, b"ok")
            if self.path not in ("/", "/v1/summary"):
                return self.send(404)
            if not self.authorized():
                return self.send(401, b"sign in", extra=[("WWW-Authenticate", 'Basic realm="Orbital usage"')])
            s = store.summary()
            if self.path == "/v1/summary":
                return self.send(200, json.dumps(s).encode(), "application/json")
            self.send(200, page(s).encode(), "text/html; charset=utf-8")

    return Handler


def selftest():
    s = Store(":memory:")
    ev = lambda install, event="active", **kw: {  # noqa: E731
        "install": install, "session": "00000000-0000-4000-8000-000000000001", "event": event,
        "version": "1.3.4", "os": "macos", "arch": "aarch64", "connected": 1, "saved": 3, **kw,
    }
    a, b = "11111111-1111-4111-8111-111111111111", "22222222-2222-4222-8222-222222222222"
    good = json.dumps(ev(a, "opened")).encode()
    assert clean(good)["install"] == a
    # Extra fields are dropped; bad values refused.
    assert "host" not in clean(json.dumps({**ev(a), "host": "db.internal"}).encode())
    for bad in [b"", b"[]", b"{", json.dumps(ev("not-a-uuid")).encode(), json.dumps(ev(a, "login")).encode(),
                json.dumps(ev(a, os="mac os")).encode(), json.dumps(ev(a, connected=-1)).encode(),
                json.dumps(ev(a, saved=True)).encode(), json.dumps(ev(a, version="1.0; drop")).encode()]:
        assert clean(bad) is None, bad
    now = 1_800_000_000
    s.add(clean(good), now - 40 * DAY)
    for i in range(4):
        s.add(clean(json.dumps(ev(a)).encode()), now - i * 60)
    s.add(clean(json.dumps(ev(b, "opened", os="linux", connected=3, saved=9)).encode()), now - 2 * DAY)
    r = s.summary(now)
    assert r["people"] == 2 and r["activeToday"] == 1 and r["active7"] == 2 and r["active30"] == 2, r
    assert r["hours30"] == 2.0 and r["hoursPerPerson30"] == 1.0, r
    assert dict(r["os"]) == {"macos": 1, "linux": 1}, r
    assert len(r["daily"]) == 14 and r["daily"][-1]["people"] == 1, r
    assert "<title>Orbital usage</title>" in page(r)
    # Old events go once a day.
    s.add(clean(json.dumps(ev(b)).encode()), now + KEEP_DAYS * DAY)
    assert s.db.execute("SELECT COUNT(*) FROM events WHERE at < ?", (now,)).fetchone()[0] == 0
    print("selftest ok")


def main():
    if sys.argv[1:] == ["selftest"]:
        return selftest()
    token = os.environ.get("ORBITAL_STATS_TOKEN", "")
    if len(token) < 16:
        sys.exit("set ORBITAL_STATS_TOKEN (16+ characters) for the dashboard")
    store = Store(os.environ.get("ORBITAL_STATS_DB", "/var/lib/orbital-stats/stats.db"))
    port = int(os.environ.get("ORBITAL_STATS_PORT", "8787"))
    srv = ThreadingHTTPServer(("127.0.0.1", port), make_handler(store, token))
    print(f"orbital-stats listening on 127.0.0.1:{port}", flush=True)
    srv.serve_forever()


if __name__ == "__main__":
    main()
