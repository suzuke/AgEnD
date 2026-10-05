#!/usr/bin/env python3
"""Separately approved full smoke with bounded read-only initial UI evidence."""
import argparse
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import time

from claude_live_smoke import IDS, Smoke, digest, plan, require, write_json

CAPTURE = {"phase": "initial idle only", "instances": IDS, "rows": 24,
           "interval_seconds": 15, "max_batches": 14, "max_frames": 28,
           "timeout_seconds_per_frame": 5,
           "operations": "hello and subscribe_terminal_frames only; no acquire, resize or input"}


class ObservedSmoke(Smoke):
    def capture(self, batch):
        require(1 <= batch <= CAPTURE["max_batches"], "capture budget exhausted")
        for instance in IDS:
            argv = [self.p["snapshot"], str(self.home), self.nonce, instance]
            result = subprocess.run(argv, env=self.env, cwd=self.home, capture_output=True,
                                    text=True, timeout=min(5, self.left()))
            filename = f"initial-frame-{batch}-{instance}.json"
            self.trace.append({"time_ms": int(time.time()*1000), "argv": argv,
                               "exit": result.returncode, "stderr": result.stderr,
                               "evidence_file": filename})
            require(result.returncode == 0, "read-only frame failed; stop without input")
            frame = json.loads(result.stdout)
            require(frame["data"]["instance_id"] == instance
                    and frame["data"]["request_id"] == "startup-diagnostic",
                    "initial frame identity mismatch")
            write_json(self.out / filename, frame)

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
        spec.update(format="claude-observed-smoke-v1", initial_capture=CAPTURE,
                    runner=str(Path(__file__).resolve()), runner_sha256=digest(__file__),
                    smoke_runner=str(shared), smoke_runner_sha256=digest(shared),
                    snapshot=str(args.snapshot.resolve()), snapshot_sha256=digest(args.snapshot))
        spec["snapshot_argv_template"] = [spec["snapshot"], spec["home"], spec["nonce"], "<g12live-a or g12live-b>"]
        write_json(args.plan, spec)
        print(json.dumps({"plan": str(args.plan), "sha256": digest(args.plan), "backend_executions": 0}))
        return
    require(args.execute and os.environ.get("AGEND_REAL_CLAUDE_LIVE") == "1",
            "execution requires --execute and AGEND_REAL_CLAUDE_LIVE=1")
    require(args.approved_plan_sha256 and digest(args.execute) == args.approved_plan_sha256,
            "approved plan hash mismatch")
    spec = json.loads(args.execute.read_text())
    require(spec["format"] == "claude-observed-smoke-v1" and spec["initial_capture"] == CAPTURE,
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
        print(f"claude_observed_smoke: {error}", file=sys.stderr)
        sys.exit(1)
