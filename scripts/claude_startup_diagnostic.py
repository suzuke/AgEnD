#!/usr/bin/env python3
"""Pinned, separately approved startup capture; never sends a work message."""
import argparse
import json
import os
from pathlib import Path
import signal
import sqlite3
import subprocess
import sys
import time

from claude_live_smoke import Smoke, digest, plan, require, write_json

BUDGET = {"work_messages": 0, "instances": 1, "seconds": 90,
          "snapshots": 4, "automatic_retries": 0,
          "note": "No model prompts or team messages. True CLI startup and four read-only frames; no API-call or monetary hard cap."}


class Diagnostic(Smoke):
    def execute(self):
        self.start()
        origin = time.monotonic()
        for index in range(1, 5):
            self.wait(lambda: time.monotonic() >= origin + index * 15,
                      "capture interval", seconds=20)
            result = subprocess.run(self.p["snapshot_argv"], env=self.env, cwd=self.home,
                                    capture_output=True, text=True, timeout=min(5, self.left()))
            self.trace.append({"time_ms": int(time.time()*1000), "argv": self.p["snapshot_argv"],
                               "exit": result.returncode, "stderr": result.stderr,
                               "evidence_file": f"frame-{index}.json"})
            require(result.returncode == 0, "read-only frame capture failed; stop without input")
            frame = json.loads(result.stdout)
            require(frame["data"]["instance_id"] == "g12live-a"
                    and frame["data"]["request_id"] == "startup-diagnostic",
                    "diagnostic frame identity mismatch")
            write_json(self.out / f"frame-{index}.json", frame)
            states = self.status()
            self.trace.append({"time_ms": int(time.time()*1000), "fleet_states": states})
            require(set(states) == {"g12live-a"}, "diagnostic instance membership changed")

    def audit(self):
        with sqlite3.connect(f"file:{self.home / 'agend.db'}?mode=ro", uri=True, timeout=0) as db:
            db.row_factory = sqlite3.Row
            messages = [dict(r) for r in db.execute("SELECT * FROM messages ORDER BY seq")]
            events = [dict(r) for r in db.execute("SELECT * FROM driver_events ORDER BY seq")]
            startup = [dict(r) for r in db.execute("SELECT * FROM claude_startup ORDER BY instance_id")]
            instances = [dict(r) for r in db.execute("SELECT id,session_id,agent_pid,status FROM instances ORDER BY id")]
        write_json(self.out / "native-evidence.json", dict(messages=messages, events=events,
                                                         startup=startup, instances=instances))
        require(not messages and len(instances) == 1 and instances[0]["id"] == "g12live-a",
                "diagnostic unexpectedly sent work or changed membership")
        starts = [e for e in events if e["kind"] == "SessionStart"]
        require(len(starts) == 1 and not starts[0]["replayed"]
                and starts[0]["session_id"] == self.sessions["g12live-a"],
                "diagnostic session started more than once or is missing")
        return {"verdict": "CAPTURED", "work_messages": 0, "true_version_query": self.p["version"],
                "frames": len(list(self.out.glob("frame-*.json"))),
                "limit": "Startup observation only; not model conformance or complete Gate 12A. No transcript usage or exact API-call count claimed."}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--plan", type=Path)
    parser.add_argument("--agend", type=Path)
    parser.add_argument("--cleanup", type=Path)
    parser.add_argument("--snapshot", type=Path)
    parser.add_argument("--out", type=Path)
    parser.add_argument("--execute", type=Path)
    parser.add_argument("--approved-plan-sha256")
    args = parser.parse_args()
    shared = Path(__file__).with_name("claude_live_smoke.py").resolve()
    if args.plan:
        require(not args.execute and args.agend and args.cleanup and args.snapshot and args.out,
                "plan requires --agend --cleanup --snapshot --out")
        require(not args.plan.exists(), "plan exists; do not overwrite")
        spec = plan(args.agend, args.cleanup, args.out)
        spec.update(format="claude-startup-diagnostic-v1", budget=BUDGET, instances=["g12live-a"],
                    runner=str(Path(__file__).resolve()), runner_sha256=digest(__file__),
                    smoke_runner=str(shared), smoke_runner_sha256=digest(shared),
                    snapshot=str(args.snapshot.resolve()), snapshot_sha256=digest(args.snapshot))
        for name in ("prompts", "send_sequence", "send_argv_template", "send_environment"):
            spec.pop(name)
        spec["add_argv"] = spec["add_argv"][:1]
        spec["backend_argv"] = spec["backend_argv"][:1]
        spec["snapshot_argv"] = [spec["snapshot"], spec["home"], spec["nonce"], "g12live-a"]
        spec["failure"] = "Stop at first failure; no model prompts/messages, manual keys, automatic retry or version substitution."
        write_json(args.plan, spec)
        print(json.dumps({"plan": str(args.plan), "sha256": digest(args.plan), "backend_executions": 0}))
        return
    require(args.execute and os.environ.get("AGEND_REAL_CLAUDE_STARTUP_DIAGNOSTIC") == "1",
            "execution requires --execute and AGEND_REAL_CLAUDE_STARTUP_DIAGNOSTIC=1")
    require(args.approved_plan_sha256 and digest(args.execute) == args.approved_plan_sha256,
            "approved plan hash mismatch")
    spec = json.loads(args.execute.read_text())
    require(spec["format"] == "claude-startup-diagnostic-v1" and spec["budget"] == BUDGET
            and spec["instances"] == ["g12live-a"], "unknown diagnostic scope")
    for name in ("runner", "smoke_runner", "agend", "cleanup", "snapshot", "claude"):
        require(digest(spec[name]) == spec[name + "_sha256"], f"{name} changed; no execution")
    require(spec["runner"] == str(Path(__file__).resolve()) and spec["smoke_runner"] == str(shared), "different runner")
    require(not Path(spec["home"]).parent.exists() and not Path(spec["out"]).exists(), "existing run resources; no retry")
    run = Diagnostic(spec)
    error = None
    try:
        run.execute()
        run.stop()
        write_json(run.out / "capture-result.json", run.audit())
    except BaseException as exception:
        error = str(exception)
        run.stop()
        if run.out.exists():
            write_json(run.out / "failure.json", {"error": error, "retry": False})
            try:
                run.audit()
            except Exception as audit_error:
                write_json(run.out / "partial-audit.json", {"error": str(audit_error)})
    finally:
        if run.out.exists():
            write_json(run.out / "commands.json", run.trace)
            try:
                write_json(run.out / "cleanup.json", run.cleanup())
            except Exception as exception:
                error = f"{error or ''}; cleanup: {exception}"
                write_json(run.out / "cleanup-failure.json", {"error": str(exception), "home_preserved": str(run.home)})
    require(error is None, error)
    print(json.dumps({"result": "CAPTURED", "evidence": str(run.out)}))


if __name__ == "__main__":
    os.umask(0o077)
    def terminated(_signal, _frame):
        raise RuntimeError("diagnostic terminated; no retry")
    signal.signal(signal.SIGTERM, terminated)
    try:
        main()
    except Exception as error:
        print(f"claude_startup_diagnostic: {error}", file=sys.stderr)
        sys.exit(1)
