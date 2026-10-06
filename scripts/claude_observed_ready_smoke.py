#!/usr/bin/env python3
"""Pinned full smoke with bounded read-only initial terminal registration wait."""
import argparse
import json
import os
from pathlib import Path
import re
import shutil
import stat
import signal
import subprocess
import sys
import time

from claude_live_smoke import IDS, Smoke, digest, plan, require, write_json

CAPTURE = {"phase": "initial idle only", "instances": list(IDS), "rows": 24,
           "interval_seconds": 15, "max_batches": 14, "max_frames": 28,
           "timeout_seconds_per_frame": 5,
           "first_frame_wait_seconds_per_instance": 10,
           "first_frame_max_attempts_per_instance": 20,
           "first_frame_poll_seconds": 0.25, "max_helper_reads": 66,
           "operations": "hello and subscribe_terminal_frames only; no acquire, resize or input"}


class ObservedSmoke(Smoke):
    def status(self, seconds=15):
        data = json.loads(self.run(self.p["status_argv"], seconds=seconds))
        require(not data["attention"], "unexpected attention; stop without pressing keys")
        require(all(i["state"] != "failed" for i in data["instances"]), "instance failed")
        if self.initialized:
            require({i["instance_id"] for i in data["instances"]} == set(self.p.get("instances", IDS)),
                    "instance membership changed")
            require(all(i["state"] != "starting" for i in data["instances"]),
                    "instance restarting; no retry permitted")
        log = self.out / "daemon.log"
        if log.exists():
            require(not re.search(r"g12live-[ab]: (?:restart |.*--resume )",
                                  log.read_text(errors="replace")),
                    "backend restart observed; stop")
        return {i["instance_id"]: i["state"] for i in data["instances"]}

    def capture(self, batch):
        require(1 <= batch <= CAPTURE["max_batches"], "capture budget exhausted")
        for instance in IDS:
            argv = [self.p["snapshot"], str(self.home), self.nonce, instance]
            # Only the first frame may wait for terminal registration. The pinned
            # helper maps the native no_terminal refusal to this exact stderr.
            # No other failure, later lost terminal, or model operation is retried.
            deadline = min(self.end, time.monotonic() + 10)
            attempt = 0
            while True:
                remaining = deadline - time.monotonic()
                require(remaining > 0, "initial terminal registration timed out; no work sent")
                self.status(seconds=remaining)
                remaining = deadline - time.monotonic()
                require(remaining > 0, "initial terminal registration timed out; no work sent")
                attempt += 1
                result = subprocess.run(argv, env=self.env, cwd=self.home, capture_output=True,
                                        text=True, timeout=min(5, self.left(), remaining))
                filename = f"initial-frame-{batch}-{instance}.json"
                self.trace.append({"time_ms": int(time.time()*1000), "argv": argv,
                                   "exit": result.returncode, "stderr": result.stderr,
                                   "evidence_file": filename if result.returncode == 0 else None,
                                   "read_attempt": attempt})
                if result.returncode == 0:
                    break
                not_ready = (batch == 1 and result.returncode == 1 and not result.stdout
                             and result.stderr == "startup_frame: instance has no live terminal\n")
                require(not_ready, "read-only frame failed; stop without input")
                require(attempt < 20, "initial terminal registration read budget exhausted; no work sent")
                remaining = deadline - time.monotonic()
                require(remaining > 0, "initial terminal registration timed out; no work sent")
                time.sleep(min(0.25, remaining))
            frame = json.loads(result.stdout)
            require(frame["data"]["instance_id"] == instance
                    and frame["data"]["request_id"] == "startup-diagnostic",
                    "initial frame identity mismatch")
            write_json(self.out / filename, frame)

    def cleanup(self):
        owner = self.home / ".smoke-owner"
        owned_start = False
        if os.path.lexists(owner):
            info = owner.lstat()
            require(stat.S_ISREG(info.st_mode) and info.st_uid == os.getuid()
                    and info.st_nlink == 1 and owner.read_text() == self.nonce,
                    "home owner changed; preserve early scratch")
            owned_start = True
        result = super().cleanup()
        if not owned_start:
            return result  # A refused preflight never owns existing namespaces.
        # A CLI can allocate scratch before SessionStart. The inherited start
        # checked these nonce namespaces absent before any backend execution.
        # Cleanup runs only after native owned-holder cleanup has succeeded.
        removed = []
        for instance in IDS:
            slug = re.sub(r"[^a-zA-Z0-9]", "-", str(self.home / "workspace" / instance))
            root = Path(f"/private/tmp/claude-{os.getuid()}") / slug
            if not os.path.lexists(root):
                continue
            nodes = [root.parent, root, *root.rglob("*")]
            records = []
            for node in nodes:
                info = node.lstat()
                require(info.st_uid == os.getuid() and not stat.S_ISLNK(info.st_mode)
                        and (stat.S_ISDIR(info.st_mode)
                             or (stat.S_ISREG(info.st_mode) and info.st_nlink == 1)),
                        "early scratch identity changed; preserve namespace")
                records.append({"path": str(node), "uid": info.st_uid,
                                "mode": info.st_mode, "nlink": info.st_nlink,
                                "size": info.st_size})
            if any(node.is_file() for node in nodes):
                destination = self.out / "session-evidence" / "early-scratch" / root.name
                destination.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
                shutil.copytree(root, destination)
            shutil.rmtree(root)
            removed.append({"path": str(root), "nodes_before_removal": records,
                            "absent": not os.path.lexists(root)})
        result["early_scratch_cleanup"] = removed
        return result

    def idle(self):
        if self.initialized:
            return super().idle()
        deadline = min(self.end, time.monotonic() + 180)
        batch, next_capture = 0, time.monotonic()
        while time.monotonic() < deadline:
            self.left()
            if time.monotonic() >= next_capture:
                batch += 1
                self.capture(batch)
                next_capture = time.monotonic() + 15
            states = self.status()
            if all(states.get(instance) == "idle" for instance in IDS):
                batch += 1
                self.capture(batch)
                states = self.status()
                require(all(states.get(instance) == "idle" for instance in IDS),
                        "initial idle changed during read-only capture; no retry")
                self.trace.append({"time_ms": int(time.time()*1000), "observed": "both idle"})
                return
            time.sleep(0.25)
        raise RuntimeError("timed out: both idle; no retry")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("plan", "agend", "cleanup", "snapshot", "out", "execute"):
        parser.add_argument("--" + name, type=Path)
    parser.add_argument("--approved-plan-sha256")
    args = parser.parse_args()
    shared = Path(__file__).with_name("claude_live_smoke.py").resolve()
    if args.plan:
        require(not args.execute and args.agend and args.cleanup and args.snapshot and args.out,
                "plan requires --agend --cleanup --snapshot --out")
        require(not args.plan.exists(), "plan exists; do not overwrite approved bytes")
        spec = plan(args.agend, args.cleanup, args.out)
        spec.update(format="claude-observed-smoke-v2", initial_capture=CAPTURE,
                    runner=str(Path(__file__).resolve()), runner_sha256=digest(__file__),
                    smoke_runner=str(shared), smoke_runner_sha256=digest(shared),
                    snapshot=str(args.snapshot.resolve()), snapshot_sha256=digest(args.snapshot))
        spec["failure"] = "Only the initial no_terminal read may wait within the pinned first-frame bounds. Stop on any other error; no model work replay, backend restart or automatic rerun."
        spec["cleanup_policy"] += " Also remove fresh nonce scratch allocated before SessionStart, after native cleanup, with predelete node metadata and private file preservation."
        spec["snapshot_argv_template"] = [spec["snapshot"], spec["home"], spec["nonce"], "<g12live-a or g12live-b>"]
        write_json(args.plan, spec)
        print(json.dumps({"plan": str(args.plan), "sha256": digest(args.plan), "backend_executions": 0}))
        return
    require(args.execute and os.environ.get("AGEND_REAL_CLAUDE_LIVE") == "1",
            "execution requires --execute and AGEND_REAL_CLAUDE_LIVE=1")
    require(args.approved_plan_sha256 and digest(args.execute) == args.approved_plan_sha256,
            "approved plan hash mismatch")
    spec = json.loads(args.execute.read_text())
    require(spec["format"] == "claude-observed-smoke-v2" and spec["initial_capture"] == CAPTURE,
            "unknown observed smoke scope")
    for name in ("runner", "smoke_runner", "agend", "cleanup", "snapshot", "claude"):
        require(digest(spec[name]) == spec[name + "_sha256"], f"{name} changed; no execution")
    require(spec["runner"] == str(Path(__file__).resolve()) and spec["smoke_runner"] == str(shared),
            "different runner")
    require(spec["budget"]["work_messages"] == 7 and spec["budget"]["harness_sends"] == 5
            and spec["budget"]["model_peer_sends"] == 2 and spec["budget"]["seconds"] == 900
            and spec["budget"]["automatic_retries"] == 0, "unknown work budget")
    require(not Path(spec["out"]).exists() and not Path(spec["home"]).parent.exists(),
            "existing run resources; do not retry")
    smoke = ObservedSmoke(spec)
    error = None
    try:
        smoke.execute()
        smoke.stop()
        write_json(smoke.out / "smoke-pass.json", smoke.audit())
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
                write_json(smoke.out / "cleanup-failure.json", {"error": str(exception),
                                                              "home_preserved": str(smoke.home)})
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
        print(f"claude_observed_ready_smoke: {error}", file=sys.stderr)
        sys.exit(1)
