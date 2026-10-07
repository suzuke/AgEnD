#!/usr/bin/env python3
"""Prepare a pinned plan, then run ONE explicitly approved true-model smoke.

No backend is executed by --plan or by argument/authorization validation.
The real CLI and daemon are the producers; SQLite is read only AFTER exit.
"""
import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import re
import shlex
import shutil
import signal
import sqlite3
import stat
import subprocess
import sys
import threading
import time
import uuid

IDS = ("g12live-a", "g12live-b")
CLAUDE = Path("/Users/suzuke/.local/share/claude/versions/2.1.284")
CLAUDE_SHA = "50a14c2f50f56668380fdda490167f1d3630d5cc18fb8aed3073c2c7ea7314fe"
PERSONAL = Path.home() / ".claude"
PASS_ENV = ("HOME", "USER", "LOGNAME", "LANG", "LC_ALL", "LC_CTYPE", "TMPDIR", "TZ")


def pass_environment():
    return {k: os.environ[k] for k in PASS_ENV if k in os.environ}


def require(ok, message):
    if not ok:
        raise RuntimeError(message)


def digest(path):
    with Path(path).open("rb") as file:
        return hashlib.file_digest(file, "sha256").hexdigest()


def audit_text(raw):
    try:
        return raw.decode("utf-8")
    except UnicodeDecodeError as error:
        raise RuntimeError("shim audit is not UTF-8") from error


def completed_startup(row):
    # Production marks the startup key loop halted when Ready is recognized.
    # Halted alone also covers refusal/manual paths; require our three writes.
    keys = json.loads(row["keys"])
    return (row["halted"] == 1 and row["manual"] == 0 and row["generation"]
            and set(keys) == {"trust_no", "trust_yes", "development"}
            and all(v.get("state") == "written" for v in keys.values()))


def write_json(path, value):
    path.write_text(json.dumps(value, indent=2, ensure_ascii=False) + "\n")
    path.chmod(0o600)


def commands(home, nonce):
    def mark(name):
        return f"printf %s {shlex.quote(nonce)} > {shlex.quote(str(home / 'workspace' / IDS[0] / name))}"
    # The Claude Bash tool can use zsh. Resolve external PATH entries without
    # shell-specific builtin options (kill itself can also be a builtin).
    paths = "for t in git kill pkill killall gh; do /usr/bin/which \"$t\"; done > shim-paths.txt"
    guard = "gh pr merge --help > gh-guard.txt 2>&1; printf %s \"$?\" > gh-exit.txt"
    peer_return = (f"SMOKE {nonce} RETURN: ACK this message before work. In Bash run exactly: "
                   f"{mark('peer-complete')}. Do not send any messages. Then reply DONE.")
    ready = shlex.quote(str(home / "workspace" / IDS[1] / "a-ready"))
    peer = (f"SMOKE {nonce} PEER: ACK this message before work. Run only this foreground Bash command "
            f"with timeout 180000: while ! test -f {ready}; do sleep 1; done; "
            f"agend send {IDS[0]} {shlex.quote(peer_return)}. Send exactly once; do not retry. Then reply DONE.")
    initial = (f"SMOKE {nonce} INITIAL: ACK this message before work. "
               "The gh command below requests read-only help, with no PR number or merge action. "
               "The PATH shim deliberately refuses even this help request; record that refusal "
               "without bypassing it, then continue the team message. Using the Bash tool, "
               f"run exactly this entire command in one foreground call: {paths}; {guard}; agend send {IDS[1]} {shlex.quote(peer)}. "
               "Do not use an absolute gh path, do not retry send, and do no other work. Then reply DONE.")
    busy = (f"SMOKE {nonce} BUSY: ACK this message before work. Use one foreground Bash call "
            f"with timeout 90000, not background: {mark('busy-start')}; sleep 45; {mark('busy-end')}. "
            "Do not shorten sleep or send messages. Then reply DONE.")
    queued = (f"SMOKE {nonce} QUEUED: ACK this message before work. Use Bash: {mark('queue-complete')}. "
              "Do not send messages. Then reply DONE.")
    blocking = (f"SMOKE {nonce} BLOCKING: ACK this message before work. Use one foreground Bash call "
                f"with timeout 150000, not background: {mark('interrupt-start')}; sleep 120; "
                f"{mark('interrupt-end')}. Do not shorten sleep or send messages. Then reply DONE.")
    interrupt = (f"SMOKE {nonce} INTERRUPT: ACK this message before work. Cancel the previous sleeping "
                 f"Bash call. Use Bash: {mark('interrupt-complete')}. Do not send messages. Then reply DONE.")
    return dict(initial=initial, peer=peer, peer_return=peer_return, busy=busy,
                queued=queued, blocking=blocking, interrupt=interrupt)


def plan(agend, cleanup, output):
    agend, cleanup, output = (p.resolve() for p in (agend, cleanup, output))
    require(not output.exists(), "evidence directory already exists")
    nonce = uuid.uuid4().hex
    home = Path(f"/private/tmp/g12live-{nonce}/home")
    args = ["--model", "claude-haiku-4-5-20251001", "--effort", "low"]
    require(digest(CLAUDE) == CLAUDE_SHA, "Claude fingerprint differs; no execution")
    cli = str(agend)
    return {
        "format": "claude-live-smoke-v1", "nonce": nonce,
        "source_head": subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip(),
        "runner": str(Path(__file__).resolve()), "runner_sha256": digest(__file__),
        "agend": cli, "agend_sha256": digest(agend),
        "cleanup": str(cleanup), "cleanup_sha256": digest(cleanup),
        "claude": str(CLAUDE), "claude_sha256": CLAUDE_SHA, "version": "2.1.284",
        "home": str(home), "out": str(output), "size": {"columns": 100, "rows": 24},
        "budget": {"work_messages": 7, "harness_sends": 5, "model_peer_sends": 2,
                   "seconds": 900, "automatic_retries": 0,
                   "note": "Seven work requests, not seven API calls. ACK/Bash tools can add model continuations. No monetary or API-call hard cap; wall time is the execution bound."},
        "version_argv": [str(CLAUDE), "--version"],
        "daemon_argv": [cli, "daemon"],
        "add_argv": [[cli, "instance", "add", i, "claude", "--program", str(CLAUDE), "--", *args] for i in IDS],
        "backend_argv": [[str(CLAUDE), *args, "--setting-sources", "project,local", "--settings",
                          str(home / "claude" / i / "settings.json"), "--permission-mode", "bypassPermissions",
                          "--dangerously-load-development-channels", "server:agend", "--session-id", "<daemon UUID v4>"] for i in IDS],
        "status_argv": [cli, "status", "--json"],
        "prompts": commands(home, nonce),
        "send_sequence": [("initial", "queue"), ("busy", "queue"), ("queued", "queue"),
                          ("blocking", "queue"), ("interrupt", "interrupt")],
        "send_argv_template": [cli, "send", IDS[0], "<exact prompts[name]>", "--level", "<send_sequence level>"],
        "send_environment": {"AGEND_INSTANCE": IDS[1], "note": "Five harness seeds use B's agent command identity. Only the two peer prompts require actual model-executed send evidence."},
        "cleanup_argv": [str(cleanup), str(home), nonce, cli],
        "environment": {"AGEND_HOME": str(home), "HOME": str(Path.home()),
                        "PATH": str(agend.parent) + ":" + os.environ.get("PATH", "/usr/bin:/bin"),
                        "AGEND_INSTANCE": "unset for operator", "CLAUDE_CONFIG_DIR": "unset"},
        "passthrough_environment": pass_environment(),
        "expected_resolved_model": "claude-haiku-4-5-20251001",
        "gh_audit_scope": "Capture the append-only native audit before INITIAL. Only refused auth token calls by A/B in their exact workspaces may be startup gh records. After that prefix, require exactly one A gh_merge refusal; reject every additional gh call.",
        "failure": "Stop at first failure. No manual terminal input, daemon restart, automatic rerun, version substitution, or prompt repair.",
        "cleanup_policy": "Stop only the child daemon; native shutdown/sweep only nonce-owned holders. Remove own home and exact fresh session/project artifacts once processes are absent. Preserve foreign data and report leftovers. Keep private evidence outside Git. Retain ~/.claude.json trust entries by user instruction; never write the shared account file or stop foreign Claude sessions.",
    }


class Smoke:
    def __init__(self, spec):
        self.p = spec
        self.home, self.out = Path(spec["home"]), Path(spec["out"])
        self.nonce = spec["nonce"]
        self.end = time.monotonic() + spec["budget"]["seconds"]
        self.daemon = None
        self.sessions = {}
        self.trace = []
        require(pass_environment() == spec["passthrough_environment"], "planned environment changed; no execution")
        require(str(Path.home()) == spec["environment"]["HOME"], "planned HOME changed; no execution")
        self.env = dict(spec["passthrough_environment"])
        self.env.update(AGEND_HOME=str(self.home), PATH=spec["environment"]["PATH"])
        self.initialized = False

    def left(self):
        require(time.monotonic() < self.end, "execution budget exhausted")
        if self.daemon:
            require(self.daemon.poll() is None, "daemon exited; no restart permitted")
        return self.end - time.monotonic()

    def run(self, argv, seconds=15, check=True, sender=None):
        environment = dict(self.env)
        if sender:
            environment["AGEND_INSTANCE"] = sender
        completed = subprocess.run(argv, env=environment, cwd=self.home,
                                   capture_output=True, text=True, timeout=min(seconds, self.left()))
        self.trace.append({"time_ms": int(time.time()*1000), "argv": argv, "caller": sender or "operator",
                           "exit": completed.returncode, "stdout": completed.stdout, "stderr": completed.stderr})
        require(not check or completed.returncode == 0, f"command failed: {argv[:3]}")
        return completed.stdout

    def status(self):
        data = json.loads(self.run(self.p["status_argv"]))
        require(not data["attention"], "unexpected attention; stop without pressing keys")
        require(all(i["state"] != "failed" for i in data["instances"]), "instance failed")
        if self.initialized:
            require({i["instance_id"] for i in data["instances"]} == set(self.p.get("instances", IDS)), "instance membership changed")
            require(all(i["state"] != "starting" for i in data["instances"]), "instance restarting; no retry permitted")
        log = self.out / "daemon.log"
        if log.exists():
            text = log.read_text(errors="replace")
            require(not re.search(r"g12live-[ab]: (?:restart |.*--resume )", text), "backend restart observed; stop")
        return {i["instance_id"]: i["state"] for i in data["instances"]}

    def wait(self, condition, name, seconds=180):
        deadline = min(self.end, time.monotonic() + seconds)
        while time.monotonic() < deadline:
            self.left()
            if condition():
                self.trace.append({"time_ms": int(time.time()*1000), "observed": name})
                return
            time.sleep(0.25)
        raise RuntimeError(f"timed out: {name}; no retry")

    def marker(self, name):
        path = self.home / "workspace" / IDS[0] / name
        return path.exists() and path.read_text() == self.nonce

    def idle(self):
        self.wait(lambda: all(self.status().get(i) == "idle" for i in IDS), "both idle")

    def send(self, name, level="queue"):
        self.run([self.p["agend"], "send", IDS[0], self.p["prompts"][name], "--level", level], seconds=75, sender=IDS[1])

    def start(self):
        # A nonce alone does not prove ownership of a pre-existing artifact.
        # Check every instance before any version query or backend startup.
        scratch_parent = Path(f"/private/tmp/claude-{os.getuid()}")
        if os.path.lexists(scratch_parent):
            info = scratch_parent.lstat()
            require(stat.S_ISDIR(info.st_mode) and info.st_uid == os.getuid()
                    and not scratch_parent.is_symlink(), "foreign scratch parent; preserved")
        for instance in self.p.get("instances", IDS):
            slug = re.sub(r"[^a-zA-Z0-9]", "-", str(self.home / "workspace" / instance))
            for candidate in (PERSONAL / "projects" / slug, scratch_parent / slug):
                require(not os.path.lexists(candidate), "pre-existing personal namespace; preserved")
        self.out.mkdir(mode=0o700)
        self.home.parent.mkdir(mode=0o700)
        self.home.mkdir(mode=0o700)
        (self.home / ".smoke-owner").write_text(self.nonce)
        (self.home / ".smoke-owner").chmod(0o600)
        # A stalled subprocess/read must not keep models running indefinitely.
        # SIGTERM is handled as an exception so the same owned cleanup runs.
        self.watchdog = threading.Timer(max(0, self.end - time.monotonic()),
                                        lambda: os.kill(os.getpid(), signal.SIGTERM))
        self.watchdog.daemon = True
        self.watchdog.start()
        version = self.run(self.p["version_argv"])
        require(re.fullmatch(r"2\.1\.284(?: \(Claude Code\))?\s*", version) is not None, "queried version mismatch")
        log = (self.out / "daemon.log").open("wb")
        self.daemon = subprocess.Popen(self.p["daemon_argv"], env=self.env, cwd=self.home,
                                       stdin=subprocess.DEVNULL, stdout=log, stderr=log)
        log.close()
        self.wait(lambda: (self.home / "run/daemon.sock").exists(), "daemon socket", seconds=20)
        for argv in self.p["add_argv"]:
            output = self.run(argv)
            match = re.search(r"session ([0-9a-f-]{36})", output)
            require(match is not None, "add reply has no session identity")
            self.sessions[argv[3]] = match.group(1)

    def startup_gh_records(self, raw):
        require(not raw or raw.endswith("\n"), "incomplete startup shim audit")
        records = [json.loads(line) for line in raw.splitlines() if line]
        gh = [r for r in records if r.get("tool") == "gh"]
        for row in gh:
            instance = row.get("instance")
            require(instance in IDS and row.get("event") == "refuse"
                    and row.get("code") == "gh_token" and row.get("argv") == ["auth", "token"]
                    and row.get("cwd") == str(self.home / "workspace" / instance),
                    "unexpected startup gh audit record")
        return gh

    def capture_guard_baseline(self):
        require(not hasattr(self, "guard_audit_baseline"), "startup shim audit already captured")
        path = self.home / "audit" / "shim.jsonl"
        raw = path.read_bytes() if path.is_file() else b""
        write_json(self.out / "gh-guard-baseline.json", {
            "audit/shim.jsonl": raw.decode("utf-8", errors="replace"),
            "raw_base64": base64.b64encode(raw).decode("ascii"),
            "captured_before": "INITIAL harness send",
            "validation": "pending",
        })
        gh = self.startup_gh_records(audit_text(raw))
        write_json(self.out / "gh-guard-baseline.json", {
            "audit/shim.jsonl": audit_text(raw), "raw_base64": base64.b64encode(raw).decode("ascii"),
            "gh_records": gh,
            "captured_before": "INITIAL harness send", "validation": "passed",
        })
        self.guard_audit_baseline = raw

    def guard_evidence(self):
        work = self.home / "workspace" / IDS[0]
        raw_files = {name: path.read_bytes() if path.is_file() else None for name, path in (
            ("shim-paths.txt", work / "shim-paths.txt"),
            ("gh-guard.txt", work / "gh-guard.txt"),
            ("gh-exit.txt", work / "gh-exit.txt"),
            ("audit/shim.jsonl", self.home / "audit" / "shim.jsonl"),
        )}
        files = {name: data.decode("utf-8", errors="replace") if data is not None else None
                 for name, data in raw_files.items()}
        # Keep the actual observations even when a later assertion fails and
        # owned workspace cleanup runs; a failure label is not raw evidence.
        write_json(self.out / "gh-guard-observation.json", dict(files, raw_base64={
            name: base64.b64encode(data).decode("ascii") if data is not None else None
            for name, data in raw_files.items()}))
        require(all(value is not None for value in files.values()), "gh guard observation missing")
        files = {name: audit_text(data) for name, data in raw_files.items()}
        paths = files["shim-paths.txt"].splitlines()
        require(paths == [str(self.home / "bin" / t) for t in ("git", "kill", "pkill", "killall", "gh")],
                "external shim PATH observation differs")
        output = files["gh-guard.txt"]
        require(files["gh-exit.txt"] == "1" and "agend-shim: refused `gh pr merge`" in output,
                "gh guard was not observed")
        require(hasattr(self, "guard_audit_baseline"), "startup shim audit baseline missing")
        prefix = self.guard_audit_baseline
        startup_gh = self.startup_gh_records(audit_text(prefix))
        require(raw_files["audit/shim.jsonl"].startswith(prefix), "startup shim audit prefix changed")
        suffix = audit_text(raw_files["audit/shim.jsonl"][len(prefix):])
        records = [json.loads(line) for line in suffix.splitlines() if line]
        gh = [r for r in records if r.get("tool") == "gh"]
        require(len(gh) == 1 and gh[0].get("event") == "refuse" and gh[0].get("code") == "gh_merge"
                and gh[0].get("instance") == IDS[0] and gh[0].get("cwd") == str(work)
                and gh[0].get("argv") == ["pr", "merge"], "native gh refusal identity/count differs")
        evidence = {"requested_argv": ["gh", "pr", "merge", "--help"], "shim_paths": paths,
                    "exit": 1, "output": output, "native_records": gh,
                    "startup_gh_refusals": startup_gh}
        write_json(self.out / "gh-guard-evidence.json", evidence)
        return evidence

    def execute(self):
        self.start()
        self.idle()
        self.capture_guard_baseline()
        self.initialized = True
        self.send("initial")
        work = self.home / "workspace" / IDS[0]
        self.wait(lambda: (work / "shim-paths.txt").is_file() and (work / "gh-exit.txt").is_file()
                  and self.status().get(IDS[0]) == "idle", "A finished initial work before peer reply")
        self.guard_evidence()
        (self.home / "workspace" / IDS[1] / "a-ready").write_text(self.nonce)
        self.wait(lambda: self.marker("peer-complete"), "model peer round trip")
        self.idle()
        self.send("busy")
        self.wait(lambda: self.marker("busy-start") and self.status().get(IDS[0]) == "working", "busy foreground command")
        require(not self.marker("busy-end"), "busy command ended before queue injection")
        self.send("queued")
        require(not self.marker("busy-end"), "busy command ended during queue injection")
        self.wait(lambda: self.marker("busy-end") and self.marker("queue-complete"), "Stop queue work completed")
        self.idle()
        self.send("blocking")
        self.wait(lambda: self.marker("interrupt-start") and self.status().get(IDS[0]) == "working", "interrupt foreground command")
        self.send("interrupt", "interrupt")
        self.wait(lambda: self.marker("interrupt-complete"), "interrupt message work completed", seconds=90)
        require(not self.marker("interrupt-end"), "blocking Bash was not interrupted")
        self.idle()

    def stop(self):
        if hasattr(self, "watchdog"):
            self.watchdog.cancel()
        if self.daemon and self.daemon.poll() is None:
            self.daemon.send_signal(signal.SIGINT)
            try:
                self.daemon.wait(timeout=10)
            except subprocess.TimeoutExpired:
                self.daemon.kill()
                self.daemon.wait(timeout=5)

    def audit(self):
        # SQLite EXCLUSIVE forbids external reads while the daemon runs.
        # Never bypass it with immutable=1 or by copying a changing WAL.
        with sqlite3.connect(f"file:{self.home / 'agend.db'}?mode=ro", uri=True, timeout=0) as db:
            db.row_factory = sqlite3.Row
            rows = [dict(r) for r in db.execute("SELECT m.*,d.delivery_id,d.session_id,d.route,d.started_at_unix_ms,d.sent_at_unix_ms,d.confirmed_at_unix_ms FROM messages m JOIN claude_deliveries d ON d.message_id=m.id ORDER BY m.seq")]
            all_messages = [dict(r) for r in db.execute("SELECT * FROM messages ORDER BY seq")]
            events = [dict(r) for r in db.execute("SELECT * FROM driver_events ORDER BY seq")]
            startup = [dict(r) for r in db.execute("SELECT * FROM claude_startup ORDER BY instance_id")]
            identities = [dict(r) for r in db.execute("SELECT id,session_id,agent_pid,status FROM instances ORDER BY id")]
            write_json(self.out / "native-evidence.json", dict(messages=rows, all_messages=all_messages, events=events, startup=startup, instances=identities))
        require(len(rows) == len(all_messages) == 7 and len(startup) == 2, "unexpected message/startup count")
        require({r["body"] for r in rows} == set(self.p["prompts"].values()), "message bodies differ or duplicate work")
        guard = self.guard_evidence()
        ack_events = [r for r in events if r["kind"] == "AgendAck" and not r["replayed"]]
        for row in rows:
            require(row["state"] == "confirmed" and row["confirmed_at_unix_ms"] is not None, "message lacks native explicit ACK")
            require(row["session_id"] == self.sessions[row["to_instance"]], "delivery session changed")
            ack = [r for r in ack_events if json.loads(r["payload"]).get("message_id") == row["id"]]
            require(len(ack) == 1 and json.loads(ack[0]["payload"])["delivery_id"] == row["delivery_id"] and ack[0]["session_id"] == row["session_id"], "ACK tuple/count differs")
            expected = "stop" if row["body"] == self.p["prompts"]["queued"] else "channel"
            require(row["route"] == expected, f"wrong native route, expected {expected}")
            sender = IDS[0] if row["body"] == self.p["prompts"]["peer"] else IDS[1]
            require(row["from_instance"] == sender, "peer round trip was not sent by the model's identity")
            name = next(k for k, v in self.p["prompts"].items() if v == row["body"])
            needle = {"initial": "shim-paths.txt", "peer": "a-ready", "peer_return": "peer-complete",
                      "busy": "busy-start", "queued": "queue-complete", "blocking": "interrupt-start",
                      "interrupt": "interrupt-complete"}[name]
            work_events = []
            for event in events:
                payload = json.loads(event["payload"])
                command = payload.get("tool_input", {}).get("command", "")
                if (event["instance_id"] == row["to_instance"] and event["session_id"] == row["session_id"]
                        and not event["replayed"] and event["kind"] == "PreToolUse"
                        and payload.get("tool_name") == "Bash" and needle in command
                        and (name != "peer_return" or "agend send" not in command)):
                    work_events.append(event)
            require(work_events and min(e["seq"] for e in work_events) > ack[0]["seq"], "work began before ACK or native Bash evidence missing")
            if name in ("initial", "peer"):
                recipient, prompt = ((IDS[1], "peer") if name == "initial" else (IDS[0], "peer_return"))
                native_send = f"agend send {recipient} {shlex.quote(self.p['prompts'][prompt])}"
                require(any(native_send in json.loads(e["payload"])["tool_input"]["command"]
                            for e in work_events), "model peer send lacks exact native Bash command evidence")
            if name == "initial":
                require(any("gh pr merge --help > gh-guard.txt 2>&1" in json.loads(e["payload"])["tool_input"]["command"]
                            for e in work_events), "read-only gh help lacks native Bash command evidence")
            if name == "queued":
                states = [e for e in events if e["kind"] == "AgendState" and e["session_id"] == row["session_id"] and e["occurred_at_unix_ms"] <= row["created_at_unix_ms"]]
                require(states and json.loads(states[-1]["payload"])["busy"], "queue was not inserted while busy")
                require(row["started_at_unix_ms"] > row["created_at_unix_ms"] + 1000, "queue was dispatched immediately")
                stops = [e for e in events if e["kind"] == "Stop" and e["session_id"] == row["session_id"] and not e["replayed"] and not json.loads(e["payload"]).get("stop_hook_active", False) and e["occurred_at_unix_ms"] >= row["created_at_unix_ms"]]
                require(stops and stops[0]["occurred_at_unix_ms"] <= row["started_at_unix_ms"], "Stop did not precede queued reservation")
            if name == "interrupt":
                states = [e for e in events if e["kind"] == "AgendState" and e["session_id"] == row["session_id"] and e["occurred_at_unix_ms"] <= row["started_at_unix_ms"]]
                require(states and json.loads(states[-1]["payload"])["busy"], "interrupt was not reserved while busy")
        for row in startup:
            require(completed_startup(row), "production startup lacks completed nonmanual key sequence")
        require(len(self.sessions) == 2, "session identities missing")
        for instance, sid in self.sessions.items():
            starts = [e for e in events if e["instance_id"] == instance and e["session_id"] == sid and e["kind"] == "SessionStart"]
            require(len(starts) == 1 and not starts[0]["replayed"], "session restarted or startup event missing")
        usage = {}
        # Header is taken from true CLI's own session record, never a label supplied by the harness.
        for instance, sid in self.sessions.items():
            workspace = self.home / "workspace" / instance
            slug = re.sub(r"[^a-zA-Z0-9]", "-", str(workspace))
            transcript = PERSONAL / "projects" / slug / f"{sid}.jsonl"
            require(transcript.is_file(), "true CLI transcript missing")
            records = [json.loads(line) for line in transcript.read_text().splitlines() if line]
            versions = {r["version"] for r in records if "version" in r}
            require(versions == {self.p["version"]}, "true transcript version differs")
            require(any(r.get("type") == "assistant" for r in records), "no true assistant record")
            assistants = [r["message"] for r in records if r.get("type") == "assistant" and isinstance(r.get("message"), dict)]
            models = {r.get("model") for r in assistants if r.get("model") not in (None, "<synthetic>")}
            require(models == {self.p["expected_resolved_model"]}, "actual model differs; no fallback accepted")
            usage[instance] = [{"id": m.get("id"), "model": m.get("model"), "usage": m.get("usage")} for m in assistants]
            shutil.copyfile(transcript, self.out / f"{instance}-transcript.jsonl")
        return {"verdict": "PASS", "confirmed_messages": len(rows), "routes": {"channel": 6, "stop": 1},
                "true_cli_version": self.p["version"], "new_model_work_requests": 7, "observed_assistant_usage": usage,
                "gh_guard": guard,
                "limitations": "True-backend smoke; native fault matrix remains separately verified. No exact API-call or monetary hard cap."}

    def cleanup(self):
        self.stop()
        if not self.home.exists():
            return {"home_absent": True, "programs": "not started"}
        if (self.home / "agend.db").exists():
            with sqlite3.connect(f"file:{self.home / 'agend.db'}?mode=ro", uri=True, timeout=0) as db:
                for instance, sid in db.execute("SELECT id,session_id FROM instances WHERE backend='claude'"):
                    require(instance in IDS and sid and str(uuid.UUID(sid)) == sid, "unknown session; preserve resources")
                    require(self.sessions.get(instance, sid) == sid, "session changed; preserve resources")
                    self.sessions[instance] = sid
        result = subprocess.run(self.p["cleanup_argv"], capture_output=True, text=True, timeout=45)
        require(result.returncode == 0, "native cleanup failed; preserve home and report leftovers")
        # The user explicitly retains global trust entries. Never write shared
        # account settings or ask foreign CLI sessions to exit for cleanup.
        residues = []
        # The native SessionStart supplies the CLI's exact scratch path. A
        # project can contain another bootstrap UUID; own the nonce namespace,
        # not just the session leaf. Validate every node before any deletion.
        scratch_roots = set()
        if (self.home / "agend.db").exists():
            with sqlite3.connect(f"file:{self.home / 'agend.db'}?mode=ro", uri=True, timeout=0) as db:
                for instance, sid, payload in db.execute("SELECT instance_id,session_id,payload FROM driver_events WHERE kind='SessionStart' AND replayed=0"):
                    require(self.sessions.get(instance) == sid, "foreign scratch session; preserved")
                    event = json.loads(payload)
                    workspace = self.home / "workspace" / instance
                    require(event.get("cwd") == str(workspace) and event.get("session_id") == sid,
                            "scratch event identity mismatch; preserved")
                    scratch = event.get("scratchpad_dir")
                    if scratch is None:
                        continue  # Native fixtures need not produce CLI scratch.
                    slug = re.sub(r"[^a-zA-Z0-9]", "-", str(workspace))
                    root = Path(f"/private/tmp/claude-{os.getuid()}") / slug
                    require(Path(scratch) == root / sid / "scratchpad", "foreign scratch path; preserved")
                    if root.exists() or root.is_symlink():
                        scratch_roots.add(root)
        for root in scratch_roots:
            for directory in (root.parent, root):
                info = directory.lstat()
                require(stat.S_ISDIR(info.st_mode) and info.st_uid == os.getuid()
                        and not directory.is_symlink(), "scratch namespace identity changed; preserved")
            residues.append(str(root))
        for instance, sid in self.sessions.items():
            slug = re.sub(r"[^a-zA-Z0-9]", "-", str(self.home / "workspace" / instance))
            for candidate in [PERSONAL / "projects" / slug, PERSONAL / "debug" / f"{sid}.txt",
                              PERSONAL / "session-env" / sid, PERSONAL / "tasks" / sid,
                              PERSONAL / "todos" / f"{sid}-agent-{sid}.json"]:
                if candidate.exists() or candidate.is_symlink():
                    residues.append(str(candidate))
        # Validate EVERY path before deleting ANY. copytree follows symlinks by
        # default, so reject nested links too; foreign data remains untouched.
        for item in residues:
            path = Path(item)
            require(not path.is_symlink(), "session artifact symlink; preserved")
            for entry in [path, *(path.rglob("*") if path.is_dir() else [])]:
                info = entry.lstat()
                require(info.st_uid == os.getuid() and (stat.S_ISDIR(info.st_mode)
                        or (stat.S_ISREG(info.st_mode) and info.st_nlink == 1)),
                        "artifact type, owner or hardlink mismatch; preserved")
            if path.is_dir():
                require(not any(p.is_symlink() for p in path.rglob("*")), "nested artifact symlink; preserved")
        # Retain own failure records privately before deleting session scratch.
        for item in residues:
            path = Path(item)
            require(not path.is_symlink(), "session artifact symlink; preserved")
            destination = self.out / "session-evidence" / path.parent.name / path.name
            destination.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
            if path.is_dir():
                shutil.copytree(path, destination)
                shutil.rmtree(path)
            else:
                shutil.copyfile(path, destination)
                path.unlink()
        residues = []
        require((self.home / ".smoke-owner").read_text() == self.nonce, "home owner changed; preserved")
        shutil.rmtree(self.home.parent)
        return {"home_absent": not self.home.parent.exists(), "native_cleanup": result.stdout,
                "personal_session_residues": residues,
                "trust_entries_retained": [str(self.home / "workspace" / i) for i in self.sessions],
                "shared_account_writes": 0}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--plan", type=Path)
    parser.add_argument("--agend", type=Path)
    parser.add_argument("--cleanup", type=Path)
    parser.add_argument("--out", type=Path)
    parser.add_argument("--execute", type=Path)
    parser.add_argument("--approved-plan-sha256")
    args = parser.parse_args()
    if args.plan:
        require(not args.execute and args.agend and args.cleanup and args.out, "plan needs --agend --cleanup --out")
        require(not args.plan.exists(), "plan already exists; do not overwrite approved bytes")
        spec = plan(args.agend, args.cleanup, args.out)
        write_json(args.plan, spec)
        print(json.dumps({"plan": str(args.plan), "sha256": digest(args.plan), "backend_executions": 0}))
        return
    require(args.execute and os.environ.get("AGEND_REAL_CLAUDE_LIVE") == "1", "execution needs --execute and AGEND_REAL_CLAUDE_LIVE=1")
    require(args.approved_plan_sha256 and digest(args.execute) == args.approved_plan_sha256, "approved plan hash mismatch")
    spec = json.loads(args.execute.read_text())
    for name in ("runner", "agend", "cleanup", "claude"):
        require(digest(spec[name]) == spec[name + "_sha256"], f"{name} changed; no execution")
    require(spec["runner"] == str(Path(__file__).resolve()), "different runner")
    require(spec["budget"] == {"work_messages": 7, "harness_sends": 5, "model_peer_sends": 2, "seconds": 900, "automatic_retries": 0, "note": "Seven work requests, not seven API calls. ACK/Bash tools can add model continuations. No monetary or API-call hard cap; wall time is the execution bound."}, "unknown budget")
    require(not Path(spec["out"]).exists() and not Path(spec["home"]).parent.exists(), "existing run resources; do not retry")
    smoke = Smoke(spec)
    error = None
    try:
        smoke.execute()
        smoke.stop()
        result = smoke.audit()
        write_json(smoke.out / "smoke-pass.json", result)
    except BaseException as exception:
        error = str(exception)
        smoke.stop()
        if smoke.out.exists():
            write_json(smoke.out / "failure.json", {"error": error, "retry": False})
            try:
                smoke.audit()
            except Exception as audit_error:
                write_json(smoke.out / "partial-audit.json", {"error": str(audit_error)})
    finally:
        if smoke.out.exists():
            write_json(smoke.out / "commands.json", smoke.trace)
            try:
                write_json(smoke.out / "cleanup.json", smoke.cleanup())
            except Exception as exception:
                error = f"{error or ''}; cleanup: {exception}"
                write_json(smoke.out / "cleanup-failure.json", {"error": str(exception), "home_preserved": str(smoke.home)})
    require(error is None, error)
    print(json.dumps({"result": "PASS", "evidence": str(smoke.out)}))


if __name__ == "__main__":
    os.umask(0o077)
    def terminated(_signal, _frame):
        raise RuntimeError("execution terminated; no retry")
    signal.signal(signal.SIGTERM, terminated)
    try:
        main()
    except Exception as error:
        print(f"claude_live_smoke: {error}", file=sys.stderr)
        sys.exit(1)
