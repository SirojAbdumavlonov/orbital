#!/usr/bin/env python3
"""Orbital's server agent: answers your questions about this server in a
Telegram chat, using Claude or ChatGPT, and messages you when something
breaks.

* Only the Telegram account that paired with the one-time code Orbital
  showed is answered; everyone else is ignored.
* Looking is free: status, logs, services, containers, disk, certificates,
  updates and ports. Anything that changes the server (restarting a service
  or container, running a command) is shown in the chat first and only runs
  after you tap Approve.
* Files that hold secrets (this agent's config, /etc/shadow, private keys)
  are never read, so they never reach the AI provider or Telegram.
* Every question, tool call and decision is written to
  /var/log/orbital-agent.log (never the keys).

Installed by Orbital: /opt/orbital-agent/agent.py, config in
/etc/orbital-agent/config.json (root only), state in /var/lib/orbital-agent.
Telegram is spoken with the standard library; the AI provider through its
official SDK, installed in /opt/orbital-agent/venv.
"""

import json
import os
import re
import shlex
import socket
import subprocess
import sys
import threading
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid

VERSION = "1.3.0"
CONFIG = "/etc/orbital-agent/config.json"
STATE_DIR = "/var/lib/orbital-agent"
STATE = STATE_DIR + "/state.json"
LOG = "/var/log/orbital-agent.log"

CLAUDE_MODEL = "claude-opus-5-5"
MAX_HISTORY_MESSAGES = 60  # a conversation starts over after this many messages
IDLE_RESET_SECONDS = 2 * 3600  # or after two quiet hours
APPROVAL_TTL = 15 * 60
PAIR_ATTEMPTS = 5
TOOL_OUTPUT_LIMIT = 12000
ALERT_EVERY = 300

NAME_RE = re.compile(r"^[A-Za-z0-9@._:+-]{1,128}$")
# Never read (their contents could reach the AI provider and Telegram).
SECRET_PATHS = re.compile(
    r"^(/etc/orbital-agent(/|$)|/etc/shadow|/etc/gshadow|/etc/sudoers|/etc/ssl/private/|/root/\.ssh/|/home/[^/]+/\.ssh/|/etc/ssh/ssh_host_.*_key$|/etc/letsencrypt/(live|archive)/.*(privkey|key))"
    r"|(\.key|\.pem|id_rsa|id_ed25519|id_ecdsa|\.env)$"
)


# ---------------------------------------------------------------- utilities


def log(event, **fields):
    rec = {"time": time.strftime("%Y-%m-%dT%H:%M:%S"), "event": event, **fields}
    try:
        with open(LOG, "a") as f:
            f.write(json.dumps(rec, ensure_ascii=False) + "\n")
    except OSError:
        pass


def load_json(path, default):
    try:
        with open(path) as f:
            return json.load(f)
    except (OSError, ValueError):
        return default


def save_json(path, data, mode=0o600):
    tmp = path + ".tmp"
    fd = os.open(tmp, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, mode)
    with os.fdopen(fd, "w") as f:
        json.dump(data, f, indent=2)
    os.replace(tmp, path)


def clip(text, limit=TOOL_OUTPUT_LIMIT):
    text = text or ""
    return text if len(text) <= limit else text[: limit // 2] + "\n…(cut)…\n" + text[-limit // 2 :]


def run(args, timeout=60, shell=False):
    """Run a command (an argument list, never a shell unless asked) and return its output."""
    try:
        p = subprocess.run(args, shell=shell, capture_output=True, text=True, timeout=timeout)
        out = (p.stdout or "") + (("\n" + p.stderr) if p.stderr.strip() else "")
        return clip(out.strip() or "(no output)") + ("" if p.returncode == 0 else f"\n(exit code {p.returncode})")
    except FileNotFoundError:
        return f"{args[0] if isinstance(args, list) else args.split()[0]} is not installed on this server"
    except subprocess.TimeoutExpired:
        return f"(stopped after {timeout} s without finishing)"


def have(cmd):
    return subprocess.run(["sh", "-c", f"command -v {shlex.quote(cmd)} >/dev/null 2>&1"]).returncode == 0


def valid_name(name):
    if not isinstance(name, str) or not NAME_RE.match(name):
        raise ValueError("names are letters, digits and @ . _ : + - only")
    return name


def lines_arg(n, default=80, top=400):
    try:
        return max(1, min(top, int(n)))
    except (TypeError, ValueError):
        return default


def secret_path(path):
    """The path as written and where it really leads (a symlink can point into a secret folder)."""
    return any(SECRET_PATHS.search(p) for p in {os.path.normpath(path), os.path.realpath(path)})


# ------------------------------------------------------------------- tools

READ_TOOLS = {
    "server_overview": {
        "description": "Hostname, OS, uptime, load, memory, disks, failed services and pending updates. Start here for any general question about the server's health.",
        "properties": {},
    },
    "list_services": {
        "description": "systemd services: the failed ones, the running ones, or all.",
        "properties": {"state": {"type": "string", "enum": ["failed", "running", "all"]}},
    },
    "service_status": {
        "description": "Status of one systemd service (active or not, since when, main PID, memory, last lines).",
        "properties": {"name": {"type": "string", "description": "unit name, e.g. nginx or nginx.service"}},
        "required": ["name"],
    },
    "service_logs": {
        "description": "The last lines of a systemd service's journal.",
        "properties": {
            "name": {"type": "string"},
            "lines": {"type": "integer", "description": "how many lines, up to 400"},
            "since": {"type": "string", "description": "optional, journalctl --since value such as '1 hour ago' or 'today'"},
        },
        "required": ["name"],
    },
    "recent_errors": {
        "description": "Recent error-level messages from the whole system journal.",
        "properties": {"lines": {"type": "integer"}, "since": {"type": "string"}},
    },
    "list_containers": {
        "description": "All Docker containers with their state, image, ports and status.",
        "properties": {},
    },
    "container_logs": {
        "description": "The last lines of a Docker container's logs.",
        "properties": {"name": {"type": "string"}, "lines": {"type": "integer"}},
        "required": ["name"],
    },
    "container_details": {
        "description": "Why a container stopped or restarts: its state, exit code, OOM kill, restart count, health and restart policy.",
        "properties": {"name": {"type": "string"}},
        "required": ["name"],
    },
    "disk_usage": {
        "description": "Disk space per mounted filesystem, and the biggest folders inside a path.",
        "properties": {"path": {"type": "string", "description": "folder to look inside, default /"}},
    },
    "top_processes": {
        "description": "The processes using the most CPU and memory right now.",
        "properties": {"sort": {"type": "string", "enum": ["cpu", "memory"]}},
    },
    "listening_ports": {
        "description": "Which ports are open and which program listens on each, plus the firewall (ufw) status.",
        "properties": {},
    },
    "certificates": {
        "description": "Let's Encrypt certificates (certbot) with their domains and days left.",
        "properties": {},
    },
    "available_updates": {
        "description": "Package updates waiting to be installed (security ones marked where apt says so) and whether a reboot is needed.",
        "properties": {},
    },
    "read_file": {
        "description": "Read a text file such as a log or a config (the last lines for big files). Files holding secrets (keys, passwords, this agent's config) are refused.",
        "properties": {"path": {"type": "string"}, "lines": {"type": "integer", "description": "last N lines, up to 400"}},
        "required": ["path"],
    },
}

ACTION_TOOLS = {
    "service_action": {
        "description": "Start, stop, restart or reload a systemd service. The user is asked to approve it in Telegram first.",
        "properties": {
            "name": {"type": "string"},
            "action": {"type": "string", "enum": ["start", "stop", "restart", "reload"]},
            "reason": {"type": "string", "description": "one short sentence shown to the user"},
        },
        "required": ["name", "action", "reason"],
    },
    "container_action": {
        "description": "Start, stop or restart a Docker container. The user is asked to approve it first.",
        "properties": {
            "name": {"type": "string"},
            "action": {"type": "string", "enum": ["start", "stop", "restart"]},
            "reason": {"type": "string"},
        },
        "required": ["name", "action", "reason"],
    },
    "run_command": {
        "description": "Run a shell command as root, for anything the other tools can't do. Shown to the user exactly as written and run only after they approve. Prefer the read-only tools for looking.",
        "properties": {
            "command": {"type": "string"},
            "reason": {"type": "string", "description": "why, in one short sentence"},
        },
        "required": ["command", "reason"],
    },
}


def tool_schemas():
    """Tool definitions in a provider-neutral form: name, description, JSON schema."""
    out = []
    for name, t in {**READ_TOOLS, **ACTION_TOOLS}.items():
        schema = {"type": "object", "properties": t["properties"], "additionalProperties": False}
        if t.get("required"):
            schema["required"] = t["required"]
        out.append({"name": name, "description": t["description"], "schema": schema})
    return out


OVERVIEW = r"""
echo "host: $(hostname) · $(. /etc/os-release 2>/dev/null; echo "$PRETTY_NAME") · kernel $(uname -r)"
echo "uptime: $(uptime -p 2>/dev/null || uptime)"
echo "load: $(cut -d' ' -f1-3 /proc/loadavg) on $(nproc) cores"
free -h 2>/dev/null | sed -n '1,3p'
df -hT -x tmpfs -x devtmpfs -x squashfs -x overlay 2>/dev/null
f=$(systemctl --failed --no-legend --plain 2>/dev/null | awk '{print $1}' | tr '\n' ' ')
echo "failed services: ${f:-none}"
[ -x /usr/lib/update-notifier/apt-check ] && echo "updates (all;security): $(/usr/lib/update-notifier/apt-check 2>&1)"
[ -f /var/run/reboot-required ] && echo "reboot required: yes"
true
"""


def run_read_tool(name, a):
    if name == "server_overview":
        return run(OVERVIEW, shell=True)
    if name == "list_services":
        state = a.get("state", "failed")
        args = ["systemctl", "list-units", "--type=service", "--no-pager", "--plain", "--no-legend"]
        if state == "failed":
            args.append("--state=failed")
        elif state == "running":
            args.append("--state=running")
        else:
            args.append("--all")
        return run(args)
    if name == "service_status":
        return run(["systemctl", "status", "--no-pager", "-n", "15", "--", valid_name(a["name"])])
    if name == "service_logs":
        args = ["journalctl", "-u", valid_name(a["name"]), "-n", str(lines_arg(a.get("lines"))), "--no-pager"]
        if a.get("since"):
            args += ["--since", str(a["since"])[:40]]
        return run(args)
    if name == "recent_errors":
        args = ["journalctl", "-p", "err", "-n", str(lines_arg(a.get("lines"), 60)), "--no-pager"]
        if a.get("since"):
            args += ["--since", str(a["since"])[:40]]
        return run(args)
    if name == "list_containers":
        return run(["docker", "ps", "-a", "--format", "table {{.Names}}\t{{.State}}\t{{.Status}}\t{{.Image}}\t{{.Ports}}"])
    if name == "container_logs":
        return run(["docker", "logs", "--tail", str(lines_arg(a.get("lines"))), "--", valid_name(a["name"])])
    if name == "container_details":
        fmt = "status={{.State.Status}} exit_code={{.State.ExitCode}} oom_killed={{.State.OOMKilled}} restarts={{.RestartCount}} error={{.State.Error}} finished={{.State.FinishedAt}} health={{if .State.Health}}{{.State.Health.Status}}{{else}}none{{end}} restart_policy={{.HostConfig.RestartPolicy.Name}} memory_limit={{.HostConfig.Memory}} image={{.Config.Image}}"
        return run(["docker", "inspect", "--format", fmt, "--", valid_name(a["name"])])
    if name == "disk_usage":
        path = a.get("path") or "/"
        if not path.startswith("/"):
            return "give an absolute path, like /var"
        df = run(["df", "-hT", "-x", "tmpfs", "-x", "devtmpfs", "-x", "squashfs", "-x", "overlay"])
        du = run(f"du -xh --max-depth=1 -- {shlex.quote(path)} 2>/dev/null | sort -h | tail -20", shell=True, timeout=90)
        return f"{df}\n\nbiggest in {path}:\n{du}"
    if name == "top_processes":
        key = "-%mem" if a.get("sort") == "memory" else "-%cpu"
        return run(f"ps -eo pid,user,%cpu,%mem,rss,etime,comm --sort={key} | head -16", shell=True)
    if name == "listening_ports":
        ports = run(["ss", "-tulpn"]) if have("ss") else run(["netstat", "-tulpn"])
        fw = run(["ufw", "status", "verbose"]) if have("ufw") else "ufw is not installed"
        return f"{ports}\n\nfirewall:\n{fw}"
    if name == "certificates":
        return run(["certbot", "certificates"], timeout=90)
    if name == "available_updates":
        if have("apt"):
            out = run("apt list --upgradable 2>/dev/null | tail -n +2 | head -100", shell=True)
        elif have("dnf"):
            out = run("dnf -q check-update 2>/dev/null | head -100; true", shell=True, timeout=120)
        else:
            out = "no apt or dnf on this server"
        reboot = "\nreboot required: yes" if os.path.exists("/var/run/reboot-required") else ""
        return out + reboot
    if name == "read_file":
        path = a.get("path", "")
        if not path.startswith("/"):
            return "give an absolute path"
        if secret_path(path):
            return "refused: that file can hold secrets (keys, passwords or this agent's config), so it is never read"
        if not os.path.isfile(path):
            return f"{path} is not a file"
        return run(["tail", "-n", str(lines_arg(a.get("lines"), 200)), "--", path])
    return f"unknown tool {name}"


def describe_action(name, a):
    """The action as the user sees it in the approval message."""
    if name == "service_action":
        return f"systemctl {a.get('action')} {a.get('name')}"
    if name == "container_action":
        return f"docker {a.get('action')} {a.get('name')}"
    return a.get("command", "")


def run_action_tool(name, a):
    if name == "service_action":
        if a.get("action") not in ("start", "stop", "restart", "reload"):
            return "unknown action"
        return run(["systemctl", a["action"], "--", valid_name(a["name"])], timeout=120)
    if name == "container_action":
        if a.get("action") not in ("start", "stop", "restart"):
            return "unknown action"
        return run(["docker", a["action"], "--", valid_name(a["name"])], timeout=120)
    if name == "run_command":
        return run(["sh", "-c", str(a.get("command", ""))], timeout=300)
    return f"unknown tool {name}"


SYSTEM_PROMPT = """You are Orbital's assistant for the Linux server "{server}". The person chatting with you in Telegram owns it; they may be new to servers.

Answer their questions by looking at the server with your tools; don't guess what you can check. Keep replies short and plain (it's a phone chat): say what you found and what it means, then what to do. Use Telegram-friendly plain text, no tables or headings.

Tools that only look run at once. service_action, container_action and run_command change the server: they are shown to the person, who approves or rejects each one. Only propose a change that clearly helps with what they asked, give the reason in one short sentence, and prefer the narrowest action (restart one service rather than reboot). If they reject it, accept that and suggest what else they can do. Never try to read secrets (keys, passwords, tokens), and never print one if a tool output happens to contain it."""


# ---------------------------------------------------------------- providers


class Claude:
    """Claude through the official Anthropic SDK (manual tool loop: approvals arrive later)."""

    def __init__(self, key, model):
        import anthropic

        self.anthropic = anthropic
        self.client = anthropic.Anthropic(api_key=key)
        self.model = model or CLAUDE_MODEL
        self.tools = [{"name": t["name"], "description": t["description"], "input_schema": t["schema"]} for t in tool_schemas()]

    def user(self, text):
        return {"role": "user", "content": text}

    def step(self, system, messages):
        """One model turn. Returns (assistant message to append, [(id, name, input)], text, stop)."""
        try:
            r = self.client.beta.messages.create(
                model=self.model,
                max_tokens=16000,
                system=system,
                tools=self.tools,
                messages=messages,
                output_config={"effort": "medium"},
                # If Claude declines, the same request is retried on a fallback model.
                betas=["server-side-fallback-2026-07-01"],
                fallbacks="default",
            )
        except self.anthropic.AuthenticationError:
            raise ProviderError("the Claude API key was refused; set a new one in Orbital")
        except self.anthropic.RateLimitError:
            raise ProviderError("Claude is rate-limiting this key right now; try again in a minute")
        except self.anthropic.APIStatusError as e:
            raise ProviderError(f"Claude API error {e.status_code}: {e.message}")
        except self.anthropic.APIConnectionError:
            raise ProviderError("couldn't reach the Claude API from this server")
        calls = [(b.id, b.name, b.input) for b in r.content if b.type == "tool_use"]
        text = "\n".join(b.text for b in r.content if b.type == "text").strip()
        if r.stop_reason == "refusal":
            text = text or "I can't help with that one."
        # The whole content goes back unchanged on the next request.
        return {"role": "assistant", "content": r.content}, calls, text, r.stop_reason

    def results(self, results):
        return [
            {
                "role": "user",
                "content": [{"type": "tool_result", "tool_use_id": i, "content": out, **({"is_error": True} if err else {})} for i, out, err in results],
            }
        ]


class ChatGPT:
    """ChatGPT through the official OpenAI SDK (Chat Completions with function calling)."""

    def __init__(self, key, model):
        import openai

        self.openai = openai
        self.client = openai.OpenAI(api_key=key)
        self.model = model or "gpt-5"
        self.tools = [{"type": "function", "function": {"name": t["name"], "description": t["description"], "parameters": t["schema"]}} for t in tool_schemas()]

    def user(self, text):
        return {"role": "user", "content": text}

    def step(self, system, messages):
        try:
            r = self.client.chat.completions.create(model=self.model, messages=[{"role": "system", "content": system}, *messages], tools=self.tools)
        except self.openai.AuthenticationError:
            raise ProviderError("the OpenAI API key was refused; set a new one in Orbital")
        except self.openai.RateLimitError:
            raise ProviderError("OpenAI is rate-limiting this key (or it has no credit left)")
        except self.openai.APIStatusError as e:
            raise ProviderError(f"OpenAI API error {e.status_code}: {e.message}")
        except self.openai.APIConnectionError:
            raise ProviderError("couldn't reach the OpenAI API from this server")
        m = r.choices[0].message
        calls = []
        for c in m.tool_calls or []:
            try:
                args = json.loads(c.function.arguments or "{}")
            except ValueError:
                args = {}
            calls.append((c.id, c.function.name, args))
        msg = {"role": "assistant", "content": m.content or ""}
        if m.tool_calls:
            msg["tool_calls"] = [{"id": c.id, "type": "function", "function": {"name": c.function.name, "arguments": c.function.arguments}} for c in m.tool_calls]
        return msg, calls, (m.content or "").strip(), "tool_use" if calls else "end_turn"

    def results(self, results):
        return [{"role": "tool", "tool_call_id": i, "content": out} for i, out, _ in results]


class ProviderError(Exception):
    pass


def make_provider(cfg):
    if cfg.get("provider") == "openai":
        return ChatGPT(cfg["api_key"], cfg.get("model"))
    return Claude(cfg["api_key"], cfg.get("model"))


# ----------------------------------------------------------------- telegram


class Telegram:
    def __init__(self, token):
        self.base = f"https://api.telegram.org/bot{token}/"

    def call(self, method, timeout=30, **params):
        data = json.dumps(params).encode()
        req = urllib.request.Request(self.base + method, data=data, headers={"Content-Type": "application/json"})
        try:
            with urllib.request.urlopen(req, timeout=timeout) as r:
                body = json.loads(r.read())
        except urllib.error.HTTPError as e:
            body = json.loads(e.read() or b"{}")
        if not body.get("ok"):
            raise RuntimeError(f"telegram {method}: {body.get('description', 'failed')}")
        return body["result"]

    def send(self, chat, text, buttons=None):
        """Send text, split to Telegram's 4096-character limit. Returns the last message id."""
        text = text or "…"
        parts = [text[i : i + 4000] for i in range(0, len(text), 4000)]
        last = None
        for n, part in enumerate(parts):
            extra = {}
            if buttons and n == len(parts) - 1:
                extra["reply_markup"] = {"inline_keyboard": buttons}
            last = self.call("sendMessage", chat_id=chat, text=part, disable_web_page_preview=True, **extra)["message_id"]
        return last

    def typing(self, chat):
        try:
            self.call("sendChatAction", chat_id=chat, action="typing", timeout=10)
        except Exception:
            pass


# -------------------------------------------------------------------- agent


class Conversation:
    def __init__(self):
        self.messages = []
        self.last = time.time()
        # Waiting for approvals: the assistant turn, results so far, and the approvals still open.
        self.waiting = None


class Agent:
    def __init__(self):
        self.cfg = {}
        self.cfg_mtime = 0
        self.state = load_json(STATE, {})
        self.tg = None
        self.provider = None
        self.conv = Conversation()
        self.approvals = {}  # id -> (tool_id, name, input, message_id, created)
        self.lock = threading.Lock()
        self.alerted = set(self.state.get("alerted", []))
        self.server = socket.gethostname()

    # -- config and state

    def reload_config(self):
        try:
            m = os.stat(CONFIG).st_mtime
        except OSError:
            return
        if m == self.cfg_mtime:
            return
        self.cfg = load_json(CONFIG, {})
        self.cfg_mtime = m
        self.tg = Telegram(self.cfg["telegram_token"])
        self.server = self.cfg.get("server_name") or socket.gethostname()
        try:
            self.provider = make_provider(self.cfg)
        except ImportError as e:
            self.provider = None
            log("error", error=f"AI library missing: {e}")
        log("config", provider=self.cfg.get("provider"), model=self.cfg.get("model"))

    def save_state(self):
        self.state["alerted"] = sorted(self.alerted)
        self.state["version"] = VERSION
        save_json(STATE, self.state)

    def owner(self):
        return self.state.get("owner_id")

    # -- pairing

    def pair(self, msg, code):
        user = msg.get("from", {})
        want = self.cfg.get("pair_code")
        if not want or time.time() > float(self.cfg.get("pair_expires", 0)):
            self.tg.send(msg["chat"]["id"], "There is no pairing code waiting. Make a new one in Orbital (AI agent tab), then send /start CODE.")
            return
        tries = self.state.get("pair_tries", 0)
        if tries >= PAIR_ATTEMPTS:
            self.tg.send(msg["chat"]["id"], "Too many wrong codes. Make a new one in Orbital.")
            return
        if code.strip().upper() != want.upper():
            self.state["pair_tries"] = tries + 1
            self.save_state()
            log("pair_failed", user_id=user.get("id"))
            self.tg.send(msg["chat"]["id"], "That code doesn't match.")
            return
        self.state.update(owner_id=user["id"], owner_name=user.get("username") or user.get("first_name") or str(user["id"]), paired_at=int(time.time()), pair_tries=0, paired_code=want)
        self.save_state()
        log("paired", user_id=user["id"], name=self.state["owner_name"])
        self.tg.send(
            msg["chat"]["id"],
            f"Paired with {self.server}. Ask me anything about it, for example:\n"
            "• Is everything OK?\n• Why is the site slow?\n• Show nginx errors from the last hour\n• Which container uses the most memory?\n\n"
            "I only look unless you approve a change. /new starts a fresh conversation, /status is a quick check without AI.",
        )

    # -- messages

    def handle_update(self, u):
        if "callback_query" in u:
            return self.handle_callback(u["callback_query"])
        msg = u.get("message") or {}
        text = (msg.get("text") or "").strip()
        user = msg.get("from", {}).get("id")
        chat = msg.get("chat", {})
        if not text or chat.get("type") != "private":
            return
        if text.startswith("/start"):
            if self.owner() and user != self.owner():
                return  # already paired with someone else: stay silent
            code = text[len("/start") :].strip()
            if self.owner() == user and not code:
                self.tg.send(chat["id"], f"You're paired with {self.server}. Ask me anything.")
                return
            return self.pair(msg, code)
        if user != self.owner():
            return  # strangers get no answer at all
        if text == "/new":
            self.conv = Conversation()
            self.tg.send(chat["id"], "Started a fresh conversation.")
            return
        if text == "/status":
            self.tg.send(chat["id"], run_read_tool("server_overview", {}))
            return
        if text == "/help":
            self.tg.send(chat["id"], "Ask about this server in plain words. Changes are only made after you tap Approve. /new forgets the conversation, /status is a quick check.")
            return
        self.ask(chat["id"], text)

    def ask(self, chat, text):
        if not self.provider:
            self.tg.send(chat, "The AI library isn't installed on the server. Update the agent from Orbital.")
            return
        c = self.conv
        if c.waiting:
            self.tg.send(chat, "I'm waiting for you to approve or reject the change above first.")
            return
        if time.time() - c.last > IDLE_RESET_SECONDS or len(c.messages) > MAX_HISTORY_MESSAGES:
            c = self.conv = Conversation()
        c.last = time.time()
        c.messages.append(self.provider.user(text))
        log("question", chars=len(text))
        self.loop(chat)

    def loop(self, chat):
        """Run model turns until it answers, or stops to wait for approvals."""
        c = self.conv
        system = SYSTEM_PROMPT.format(server=self.server)
        for _ in range(20):
            self.tg.typing(chat)
            try:
                assistant, calls, text, stop = self.provider.step(system, c.messages)
            except ProviderError as e:
                log("error", error=str(e))
                # Half a tool exchange can't be sent again, so the conversation starts over.
                self.conv = Conversation()
                self.tg.send(chat, f"⚠️ {e}\nAsk again when it's fixed.")
                return
            c.messages.append(assistant)
            if text and calls:
                self.tg.send(chat, text)
            if not calls:
                self.tg.send(chat, text or "(no answer)")
                return
            results, pending = [], []
            for tid, name, args in calls:
                if name in ACTION_TOOLS:
                    pending.append((tid, name, args))
                    continue
                log("tool", tool=name)
                try:
                    results.append((tid, run_read_tool(name, args or {}), False))
                except (ValueError, KeyError) as e:
                    results.append((tid, f"error: {e}", True))
            if pending:
                c.waiting = {"results": results, "open": {}}
                for tid, name, args in pending:
                    aid = uuid.uuid4().hex[:12]
                    what = describe_action(name, args or {})
                    mid = self.tg.send(
                        chat,
                        f"Approve this change?\n\n{what}\n\nWhy: {(args or {}).get('reason', '')}",
                        buttons=[[{"text": "✅ Approve", "callback_data": f"ok:{aid}"}, {"text": "❌ Reject", "callback_data": f"no:{aid}"}]],
                    )
                    self.approvals[aid] = (tid, name, args or {}, mid, time.time())
                    c.waiting["open"][aid] = tid
                    log("approval_asked", tool=name, action=what)
                return
            c.messages.extend(self.provider.results(results))
        self.tg.send(chat, "That took too many steps; ask me again more specifically.")

    def handle_callback(self, q):
        user = q.get("from", {}).get("id")
        data = q.get("data", "")
        try:
            self.tg.call("answerCallbackQuery", callback_query_id=q["id"])
        except Exception:
            pass
        if user != self.owner() or ":" not in data:
            return
        verdict, aid = data.split(":", 1)
        item = self.approvals.pop(aid, None)
        chat = q["message"]["chat"]["id"]
        c = self.conv
        if not item or not c.waiting or aid not in c.waiting["open"]:
            self.tg.send(chat, "That request is no longer open.")
            return
        tid, name, args, mid, created = item
        what = describe_action(name, args)
        if time.time() - created > APPROVAL_TTL:
            verdict = "expired"
        if verdict == "ok":
            log("approved", tool=name, action=what)
            self.edit(chat, mid, f"✅ Approved: {what}\nRunning…")
            try:
                out = run_action_tool(name, args)
                err = False
            except ValueError as e:
                out, err = f"error: {e}", True
            log("action_done", tool=name, action=what)
            self.edit(chat, mid, f"✅ Done: {what}")
            c.waiting["results"].append((tid, out, err))
        else:
            word = "Rejected" if verdict == "no" else "Expired (not approved in time)"
            log("rejected" if verdict == "no" else "expired", tool=name, action=what)
            self.edit(chat, mid, f"❌ {word}: {what}")
            c.waiting["results"].append((tid, "The user did not approve this; it was not run.", True))
        del c.waiting["open"][aid]
        if not c.waiting["open"]:
            results = c.waiting["results"]
            c.waiting = None
            c.messages.extend(self.provider.results(results))
            self.loop(chat)

    def edit(self, chat, mid, text):
        try:
            self.tg.call("editMessageText", chat_id=chat, message_id=mid, text=text)
        except Exception:
            pass

    # -- alerts

    def check_alerts(self):
        """Message the owner once per new problem, and once when it's fixed."""
        if not self.owner() or not self.cfg.get("alerts", True):
            return
        now = set()
        texts = {}
        failed = subprocess.run(["systemctl", "--failed", "--no-legend", "--plain"], capture_output=True, text=True).stdout.split("\n") if have("systemctl") else []
        for line in failed:
            if line.strip():
                unit = line.split()[0]
                key = f"service:{unit}"
                now.add(key)
                texts[key] = f"🔴 {unit} has failed on {self.server}. Ask me why (for example: why did {unit} fail?)."
        df = subprocess.run(["df", "-P", "-x", "tmpfs", "-x", "devtmpfs", "-x", "squashfs", "-x", "overlay"], capture_output=True, text=True).stdout.split("\n")[1:]
        for line in df:
            f = line.split()
            if len(f) >= 6 and f[4].endswith("%") and int(f[4][:-1]) >= 90:
                key = f"disk:{f[5]}"
                now.add(key)
                texts[key] = f"🟠 Disk {f[5]} on {self.server} is {f[4]} full. Ask me what uses the space."
        if have("docker"):
            ps = subprocess.run(["docker", "ps", "-a", "--filter", "status=restarting", "--format", "{{.Names}}"], capture_output=True, text=True).stdout.split()
            for name in ps:
                key = f"container:{name}"
                now.add(key)
                texts[key] = f"🔴 Container {name} on {self.server} keeps restarting. Ask me why."
        for key in sorted(now - self.alerted):
            self.tg.send(self.owner(), texts[key])
            log("alert", key=key)
        for key in sorted(self.alerted - now):
            self.tg.send(self.owner(), f"🟢 Fixed on {self.server}: {key.split(':', 1)[1]} is back to normal.")
            log("alert_cleared", key=key)
        if now != self.alerted:
            self.alerted = now
            self.save_state()

    # -- main loop

    def run(self):
        os.makedirs(STATE_DIR, mode=0o700, exist_ok=True)
        self.reload_config()
        if not self.tg:
            sys.exit("no config at " + CONFIG)
        log("start", version=VERSION)
        offset = None
        next_alert = time.time() + 30
        while True:
            self.reload_config()
            if time.time() >= next_alert:
                next_alert = time.time() + ALERT_EVERY
                try:
                    self.check_alerts()
                except Exception as e:
                    log("error", error=f"alerts: {e}")
            try:
                params = {"timeout": 25, "allowed_updates": ["message", "callback_query"]}
                if offset is not None:
                    params["offset"] = offset
                updates = self.tg.call("getUpdates", timeout=40, **params)
            except Exception as e:
                log("error", error=f"telegram: {e}")
                time.sleep(5)
                continue
            for u in updates:
                offset = u["update_id"] + 1
                try:
                    self.handle_update(u)
                except Exception as e:
                    log("error", error=f"{type(e).__name__}: {e}")


# ----------------------------------------------------------------- commands


def status():
    """What Orbital shows: never the keys."""
    cfg = load_json(CONFIG, {})
    st = load_json(STATE, {})
    print(json.dumps({
        "version": VERSION,
        "provider": cfg.get("provider"),
        "model": cfg.get("model") or (CLAUDE_MODEL if cfg.get("provider") != "openai" else "gpt-5"),
        "bot": cfg.get("bot_username"),
        "alerts": cfg.get("alerts", True),
        "pairCodeActive": bool(cfg.get("pair_code")) and time.time() < float(cfg.get("pair_expires", 0)) and cfg.get("pair_code") != st.get("paired_code"),
        "owner": st.get("owner_name"),
        "ownerId": st.get("owner_id"),
        "pairedAt": st.get("paired_at"),
        "alerting": st.get("alerted", []),
    }))


def set_pair_code():
    """Read a new pairing code from stdin (never the command line); unpairs the current owner."""
    code = sys.stdin.readline().strip()
    if not re.match(r"^[A-Z2-9]{6,12}$", code):
        sys.exit("bad code")
    cfg = load_json(CONFIG, {})
    cfg["pair_code"] = code
    cfg["pair_expires"] = time.time() + 15 * 60
    save_json(CONFIG, cfg)
    st = load_json(STATE, {})
    for k in ("owner_id", "owner_name", "paired_at", "paired_code"):
        st.pop(k, None)
    st["pair_tries"] = 0
    os.makedirs(STATE_DIR, mode=0o700, exist_ok=True)
    save_json(STATE, st)
    print("ok")


def set_option():
    """Change a non-secret option: alerts on/off. `set alerts on|off`."""
    key, value = sys.argv[2], sys.argv[3]
    cfg = load_json(CONFIG, {})
    if key == "alerts":
        cfg["alerts"] = value == "on"
    else:
        sys.exit("unknown option")
    save_json(CONFIG, cfg)
    print("ok")


def selftest():
    """Checks of the parts that don't need the network (run by Orbital's tests)."""
    assert secret_path("/etc/shadow") and secret_path("/etc/orbital-agent/config.json") and secret_path("/root/.ssh/id_ed25519")
    assert secret_path("/srv/app/.env") and secret_path("/etc/ssl/private/site.key")
    assert not secret_path("/var/log/nginx/error.log") and not secret_path("/etc/nginx/nginx.conf")
    for bad in ["nginx; rm -rf /", "a b", "", "x" * 200, "$(id)"]:
        try:
            valid_name(bad)
            raise AssertionError(f"accepted {bad!r}")
        except ValueError:
            pass
    assert valid_name("php8.3-fpm.service") and valid_name("shop-web-1")
    assert lines_arg("9999") == 400 and lines_arg(None) == 80
    assert describe_action("service_action", {"name": "nginx", "action": "restart"}) == "systemctl restart nginx"
    names = [t["name"] for t in tool_schemas()]
    assert len(names) == len(set(names)) and "run_command" in names
    assert all(t["schema"]["additionalProperties"] is False for t in tool_schemas())
    assert clip("x" * 20000).count("…(cut)…") == 1
    print("selftest ok")


if __name__ == "__main__":
    cmd = sys.argv[1] if len(sys.argv) > 1 else "run"
    {"run": lambda: Agent().run(), "status": status, "pair": set_pair_code, "set": set_option, "selftest": selftest}.get(cmd, lambda: sys.exit(f"unknown command {cmd}"))()
