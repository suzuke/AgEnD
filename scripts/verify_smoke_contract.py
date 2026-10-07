#!/usr/bin/env python3
"""Run generated work in Bash and zsh against native shims, without Claude."""
import argparse
import base64
import copy
import json
import os
from pathlib import Path
import shlex
import subprocess
import tempfile

import claude_live_smoke as smoke


def verify(binary, shell_executable):
    binary = binary.resolve(strict=True)
    with tempfile.TemporaryDirectory(prefix="agend-g12a-contract-") as directory:
        home = Path(directory).resolve()
        work = home / "workspace" / smoke.IDS[0]
        work.mkdir(parents=True)
        (home / "bin").mkdir()
        real = home / "real"
        real.mkdir()
        out = home / "evidence"
        out.mkdir()
        for name in ("git", "kill", "pkill", "killall", "gh"):
            (home / "bin" / name).symlink_to(binary)
        # Only send is stubbed. The generated shell calls the actual gh shim;
        # a real-tool sentinel must never run. No daemon or backend exists.
        sender = home / "bin" / "agend"
        sender.write_text('#!/bin/sh\nprintf "%s\\0" "$@" > "$AGEND_HOME/received-send"\n')
        sender.chmod(0o700)
        gh = real / "gh"
        gh.write_text('#!/bin/sh\nprintf called > "$AGEND_HOME/real-gh-called"\nexit 23\n')
        gh.chmod(0o700)
        nonce = "f" * 32
        prompts = smoke.commands(home, nonce)
        initial = prompts["initial"]
        assert "gh pr merge --help" in initial and "gh pr merge 0" not in initial
        assert "read-only help, with no PR number or merge action" in initial
        shell = initial.split("run exactly this entire command in one foreground call: ", 1)[1].rsplit(
            ". Do not use an absolute gh path", 1)[0]
        environment = {"PATH": f"{home / 'bin'}:{real}:/usr/bin:/bin",
                       "AGEND_HOME": str(home), "AGEND_INSTANCE": smoke.IDS[0]}
        run = object.__new__(smoke.Smoke)
        run.home, run.out = home, out
        # Produce the actual startup refusals, not handwritten audit inputs.
        for instance in smoke.IDS:
            workspace = home / "workspace" / instance
            workspace.mkdir(exist_ok=True)
            for _ in range(2):
                refused = subprocess.run([str(home / "bin" / "gh"), "auth", "token"],
                                         cwd=workspace, env=dict(environment, AGEND_INSTANCE=instance),
                                         capture_output=True, text=True, timeout=5)
                assert refused.returncode == 1 and "agend-shim: refused" in refused.stderr
        run.capture_guard_baseline()
        baseline = run.guard_audit_baseline.decode("utf-8")
        assert len(run.startup_gh_records(baseline)) == 4
        result = subprocess.run([shell_executable, "-c", shell], cwd=work, env=environment,
                                capture_output=True, text=True, timeout=10)
        assert result.returncode == 0, result.stderr
        assert not (home / "real-gh-called").exists(), "refused call executed real gh"
        expected = ["send", smoke.IDS[1], prompts["peer"]]
        assert (home / "received-send").read_bytes() == b"".join(arg.encode() + b"\0" for arg in expected), "peer prompt changed by shell quoting"
        evidence = run.guard_evidence()
        assert evidence["requested_argv"] == ["gh", "pr", "merge", "--help"]
        log = home / "audit" / "shim.jsonl"
        native = log.read_text()
        suffix = native[len(baseline):]
        record = json.loads(suffix)
        empty = object.__new__(smoke.Smoke)
        empty.home, empty.out, empty.guard_audit_baseline = home, out, b""
        log.write_text(suffix)
        assert not empty.guard_evidence()["startup_gh_refusals"]
        log.write_text(native)
        missing = object.__new__(smoke.Smoke)
        missing.home, missing.out = home, out
        try:
            missing.guard_evidence()
        except RuntimeError as error:
            assert str(error) == "startup shim audit baseline missing"
        else:
            raise AssertionError("accepted missing startup baseline")
        mutations = {}
        for field, value in [("code", "shim_loop"), ("event", "bypass"),
                             ("instance", smoke.IDS[1]), ("cwd", str(home)),
                             ("argv", ["pr", "review"])]:
            row = copy.deepcopy(record)
            row[field] = value
            mutations["wrong-" + field] = baseline + json.dumps(row) + "\n"
        mutations["duplicate-refusal"] = native + suffix
        mutations["missing-native-refusal"] = baseline
        mutations["removed-startup-prefix"] = suffix
        mutations["altered-startup-prefix"] = baseline.replace('"gh_token"', '"gh_merge"', 1) + suffix
        mutations["byte-altered-prefix-newlines"] = baseline.replace("\n", "\r\n") + suffix
        mutations["invalid-utf8-observation"] = native.encode("utf-8") + b"\xff\n"
        mutations["late-auth-token"] = native + baseline.splitlines()[0] + "\n"
        mutations["late-unknown-gh"] = native + json.dumps(dict(record, code="unknown")) + "\n"
        rejected = []
        for name, contents in mutations.items():
            encoded = contents.encode("utf-8") if isinstance(contents, str) else contents
            log.write_bytes(encoded)
            try:
                run.guard_evidence()
            except RuntimeError:
                rejected.append(name)
            else:
                raise AssertionError("accepted false native evidence: " + name)
            saved = json.loads((out / "gh-guard-observation.json").read_text())
            assert base64.b64decode(saved["raw_base64"]["audit/shim.jsonl"]) == encoded
        log.write_text(native)
        bad_start = object.__new__(smoke.Smoke)
        bad_start.home, bad_start.out = home, out
        log.write_bytes(b"\xff\n")
        try:
            bad_start.capture_guard_baseline()
        except RuntimeError:
            rejected.append("invalid-utf8-startup")
        else:
            raise AssertionError("accepted invalid UTF-8 startup audit")
        saved = json.loads((out / "gh-guard-baseline.json").read_text())
        assert base64.b64decode(saved["raw_base64"]) == b"\xff\n" and saved["validation"] == "pending"
        log.write_text(native)
        for field, value in [("event", "bypass"), ("code", "gh_merge"),
                             ("argv", ["pr", "merge"]), ("instance", "foreign"),
                             ("cwd", str(home))]:
            row = json.loads(baseline.splitlines()[0])
            row[field] = value
            changed = json.dumps(row) + "\n"
            log.write_text(changed)
            bad_start = object.__new__(smoke.Smoke)
            bad_start.home, bad_start.out = home, out
            try:
                bad_start.capture_guard_baseline()
            except RuntimeError:
                rejected.append("startup-wrong-" + field)
            else:
                raise AssertionError("accepted unknown startup gh record: " + field)
            saved = json.loads((out / "gh-guard-baseline.json").read_text())
            assert saved["audit/shim.jsonl"] == changed and saved["validation"] == "pending"
        log.write_text(native)
        try:
            run.startup_gh_records(baseline.rstrip("\n"))
        except RuntimeError:
            rejected.append("incomplete-startup-audit")
        else:
            raise AssertionError("accepted incomplete startup audit")
        (work / "gh-exit.txt").write_text("0")
        try:
            run.guard_evidence()
        except RuntimeError:
            rejected.append("wrong-exit")
        else:
            raise AssertionError("accepted successful help as guard refusal")
        (work / "gh-exit.txt").write_text("1")
        (work / "shim-paths.txt").write_text("/usr/bin/gh\n")
        try:
            run.guard_evidence()
        except RuntimeError:
            rejected.append("wrong-shim-path")
        else:
            raise AssertionError("accepted PATH bypass")
        observation = json.loads((out / "gh-guard-observation.json").read_text())
        assert observation["shim-paths.txt"] == "/usr/bin/gh\n", "failed path observation was lost"
        assert observation["audit/shim.jsonl"] == native, "failed guard audit was lost"
        (work / "shim-paths.txt").write_text("\n".join(evidence["shim_paths"]) + "\n")
        (work / "gh-guard.txt").unlink()
        try:
            run.guard_evidence()
        except RuntimeError:
            rejected.append("missing-guard-observation")
        else:
            raise AssertionError("accepted missing guard output")
        observation = json.loads((out / "gh-guard-observation.json").read_text())
        assert observation["gh-guard.txt"] is None and observation["audit/shim.jsonl"] == native
        # Each shell parses every generated work request, including the nested peer
        # command; syntax checks execute no body and do not sleep or send.
        for name, prompt in prompts.items():
            if name == "initial":
                command = shell
            elif name == "peer_return":
                command = prompt.split("In Bash run exactly: ", 1)[1].split(". Do not send", 1)[0]
            elif name == "peer":
                command = prompt.split("with timeout 180000: ", 1)[1].split(". Send exactly once", 1)[0]
                tokens = shlex.split(command.split("; agend send ", 1)[1])
                assert tokens == [smoke.IDS[0], prompts["peer_return"]], "return prompt changed by shell quoting"
            elif name in ("busy", "blocking"):
                command = prompt.split("not background: ", 1)[1].split(". Do not shorten", 1)[0]
            else:
                command = prompt.split("Use Bash: ", 1)[1].split(". Do not send", 1)[0]
            parsed = subprocess.run([shell_executable, "-n", "-c", command], capture_output=True, text=True, timeout=5)
            assert parsed.returncode == 0, (name, parsed.stderr)
        report = {"verdict": "PASS", "shell": shell_executable, "native_binary": str(binary), "native_binary_sha256": smoke.digest(binary),
                  "generated_initial_shell_executed": True, "native_gh_refusal": record,
                  "exact_peer_prompt_preserved": True, "seven_shell_commands_parse": True,
                  "four_native_startup_token_refusals": evidence["startup_gh_refusals"],
                  "empty_startup_prefix_passed": True, "missing_baseline_refused": True,
                  "mutations_rejected": rejected, "failed_observation_preserved": True,
                  "Claude_executions": 0, "daemon_executions": 0,
                  "message_operations": 0, "shared_account_writes": 0,
                  "scope": "Native shim and shell contract only; model behavior remains unverified."}
    assert not home.exists(), "native contract fixture residue"
    report["own_fixture_removed"] = True
    return report


if __name__ == "__main__":
    os.umask(0o077)
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--agend", type=Path, required=True)
    args = parser.parse_args()
    print(json.dumps({"verdict": "PASS", "shells": [verify(args.agend, shell)
                     for shell in ("/bin/bash", "/bin/zsh")]}, indent=2))
