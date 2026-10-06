#!/usr/bin/env python3
"""Zero-model regression: a native terminal appears after the first subscribe."""
import argparse
import json
import os
from pathlib import Path
import sqlite3
import re
import subprocess
import sys
import threading
import time
import uuid
from unittest.mock import patch

import claude_live_smoke as base
import claude_observed_smoke as old
import claude_observed_ready_smoke as fixed


def boundaries():
    cases = []
    for label, batch, code, stdout, stderr, expected in [
        ("later-terminal-loss", 2, 1, "", "startup_frame: instance has no live terminal\n", 1),
        ("foreign-refusal", 1, 1, "", "startup_frame: forbidden\n", 1),
        ("nonempty-refusal", 1, 1, "partial", "startup_frame: instance has no live terminal\n", 1),
        ("wrong-exit", 1, 2, "", "startup_frame: instance has no live terminal\n", 1),
        ("never-registered", 1, 1, "", "startup_frame: instance has no live terminal\n", 20),
    ]:
        run = object.__new__(fixed.ObservedSmoke)
        run.p = {"snapshot": "must-not-execute"};run.home = Path("/");run.out = Path("/")
        run.nonce = "0" * 32;run.env = {};run.trace = [];run.end = 900
        run.left = lambda: 900;run.status = lambda: {}
        now = [0.0]
        reply = subprocess.CompletedProcess([], code, stdout, stderr)
        with patch.object(fixed.subprocess, "run", return_value=reply) as helper, \
             patch.object(fixed.time, "monotonic", lambda: now[0]), \
             patch.object(fixed.time, "sleep", lambda delay: now.__setitem__(0, now[0] + delay)):
            try:
                run.capture(batch)
            except RuntimeError:
                pass
            else:
                raise AssertionError("failure unexpectedly accepted: " + label)
            assert helper.call_count == expected, (label, helper.call_count)
        cases.append({"case": label, "reads": expected})
    run.status = lambda: {}
    now = [0.0]
    def slow_read(*_args, **_kwargs):
        now[0] += 4
        return subprocess.CompletedProcess([], 1, "", "startup_frame: instance has no live terminal\n")
    with patch.object(fixed.subprocess, "run", side_effect=slow_read) as helper, \
         patch.object(fixed.time, "monotonic", lambda: now[0]), \
         patch.object(fixed.time, "sleep", lambda delay: now.__setitem__(0, now[0] + delay)):
        try:
            run.capture(1)
        except RuntimeError as error:
            assert "timed out" in str(error)
        else:
            raise AssertionError("expired registration deadline accepted")
        assert helper.call_count == 3
    cases.append({"case": "deadline-exhaustion", "reads": 3})
    run.status = lambda: {}
    run.end = time.monotonic() + 900
    with patch.object(fixed.subprocess, "run", side_effect=subprocess.TimeoutExpired("helper", 5)) as helper:
        try:
            run.capture(1)
        except subprocess.TimeoutExpired:
            pass
        else:
            raise AssertionError("helper timeout accepted")
        assert helper.call_count == 1
    cases.append({"case": "helper-timeout-no-replay", "reads": 1})
    run.status = lambda: base.require(False, "unexpected attention")
    with patch.object(fixed.subprocess, "run", side_effect=AssertionError("read after attention")):
        try:
            run.capture(1)
        except RuntimeError as error:
            assert "unexpected attention" in str(error)
    cases.append({"case": "attention-before-read", "reads": 0})
    return cases


def native(module, args, evidence):
    nonce = uuid.uuid4().hex
    home = Path("/private/tmp/g12live-" + nonce + "/home")
    out = evidence / ("native-" + module.__name__ + "-" + nonce)
    binary = str(args.agend.resolve())
    spec = {"nonce": nonce, "home": str(home), "out": str(out), "budget": {"seconds": 45},
            "passthrough_environment": base.pass_environment(),
            "environment": {"HOME": str(Path.home()), "PATH": str(args.agend.parent) + ":" + os.environ["PATH"]},
            "instances": list(base.IDS), "version_argv": ["ZERO_MODEL_VERSION_STUB"],
            "daemon_argv": [binary, "daemon"], "status_argv": [binary, "status", "--json"],
            "snapshot": str(args.snapshot.resolve()),
            "cleanup_argv": [str(args.cleanup.resolve()), str(home), nonce, binary],
            "add_argv": [[binary, "instance", "add", base.IDS[0], "claude", "--program", "/bin/sh", "--", "-c", "exec /bin/cat"]]}
    run = module.ObservedSmoke(spec)
    native_run = run.run
    run.run = lambda argv, *a, **kw: "2.1.284 (Claude Code)\n" if argv == spec["version_argv"] else native_run(argv, *a, **kw)
    stop = threading.Event();errors = [];thread = None;failure = None
    def add_later():
        try:
            deadline = time.monotonic() + 15
            while not stop.is_set() and time.monotonic() < deadline:
                if any(row.get("stderr") == "startup_frame: instance has no live terminal\n" for row in run.trace):
                    if stop.wait(.6):
                        return
                    argv = list(spec["add_argv"][0]);argv[3] = base.IDS[1]
                    native_run(argv)
                    return
                stop.wait(.01)
            if not stop.is_set():
                errors.append("no native no_terminal refusal observed")
        except BaseException as error:
            errors.append(str(error))
    try:
        run.start()
        # Only A is added. The daemon itself must refuse a B subscription until
        # our worker adds B after observing the real refusal; no mocked wire.
        if module is fixed:
            for instance in base.IDS:
                slug = re.sub(r"[^a-zA-Z0-9]", "-", str(home / "workspace" / instance))
                scratch = Path(f"/private/tmp/claude-{os.getuid()}") / slug
                assert not os.path.lexists(scratch)
                scratch.mkdir(parents=True, mode=0o700)
                (scratch / "native-proof.txt").write_text("early scratch without SessionStart\n")
        thread = threading.Thread(target=add_later);thread.start()
        try:
            run.capture(1)
        except RuntimeError as error:
            failure = str(error)
        if module is old:
            assert failure == "read-only frame failed; stop without input", failure
        else:
            assert failure is None, failure
            assert all((out / f"initial-frame-1-{i}.json").is_file() for i in base.IDS)
        stop.set();thread.join(timeout=20);assert not thread.is_alive() and not errors, errors
        run.stop()
        with sqlite3.connect(home / "agend.db") as db:
            assert db.execute("SELECT count(*) FROM messages").fetchone()[0] == 0
            assert db.execute("SELECT count(*) FROM driver_events").fetchone()[0] == 0
        refusals = sum(row.get("stderr") == "startup_frame: instance has no live terminal\n" for row in run.trace)
        assert refusals >= 1
        base.write_json(out / "commands.json", run.trace)
        base.write_json(out / "result.json", {"source": module.__name__, "failure": failure,
                        "native_no_terminal_reads": refusals, "persisted_messages": 0,
                        "true_claude_executions": 0, "native_shell_only": True})
    finally:
        stop.set()
        if thread:
            thread.join(timeout=20)
        run.stop()
        if home.exists():
            cleanup = run.cleanup()
            base.write_json(out / "cleanup.json", cleanup)
            if module is fixed:
                assert len(cleanup["early_scratch_cleanup"]) == 2
                assert all(row["absent"] and row["nodes_before_removal"] for row in cleanup["early_scratch_cleanup"])
                assert len(list((out / "session-evidence" / "early-scratch").rglob("native-proof.txt"))) == 2
        assert not os.path.lexists(home.parent), "native home residue"
    return str(out)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--agend", type=Path, required=True)
    parser.add_argument("--snapshot", type=Path, required=True)
    parser.add_argument("--cleanup", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    assert not args.out.exists(), "existing verification evidence"
    args.out.mkdir(mode=0o700)
    results = {"boundaries": boundaries(), "native": [native(m, args, args.out) for m in (old, fixed)]}
    base.write_json(args.out / "result.json", results)
    print(json.dumps({"verdict": "PASS", "true_claude_executions": 0, "evidence": str(args.out)}))


if __name__ == "__main__":
    os.umask(0o077)
    main()
