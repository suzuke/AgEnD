#!/usr/bin/env python3
"""Bounded, owned-home mobile callback run; --local exercises without credentials."""
from contextlib import closing
import argparse
import hashlib
import json
import os
from pathlib import Path
import shlex
import shutil
import signal
import sqlite3
import stat
import subprocess
import tempfile
import time


def credentials(path):
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    with os.fdopen(fd) as source:
        info = os.fstat(source.fileno())
        if not stat.S_ISREG(info.st_mode) or info.st_uid != os.getuid() or info.st_mode & 0o077 or info.st_size > 4096:
            raise RuntimeError("credential file must be private, owned and regular")
        pairs = {}
        for line in source:
            line = line.strip().removeprefix("export ")
            if not line or line.startswith("#"):
                continue
            key, value = line.split("=", 1)
            words = shlex.split(value, comments=True)
            if len(words) != 1:
                raise RuntimeError("invalid credential value")
            pairs[key.strip()] = words[0]
    chat = int(pairs["TELEGRAM_CHAT_ID"])
    if not 0 < chat < 2**52:
        raise RuntimeError("this mobile lab requires the dedicated private chat")
    return {"TELEGRAM_BOT_TOKEN": pairs["TELEGRAM_BOT_TOKEN"], "TELEGRAM_CHAT_ID": str(chat)}


def db_rows(home, sql):
    with closing(sqlite3.connect(f"file:{home / 'agend.db'}?mode=ro", uri=True, timeout=2)) as db:
        return db.execute(sql).fetchall()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--target", required=True, type=Path)
    parser.add_argument("--out", required=True, type=Path)
    parser.add_argument("--credentials", type=Path)
    parser.add_argument("--local", action="store_true")
    args = parser.parse_args()
    if not args.local and args.credentials is None:
        parser.error("live mode requires --credentials")
    binary = args.target / "debug/agend"
    helper = args.target / "debug/examples/telegram_mobile_probe"
    binary_hash = hashlib.sha256(binary.read_bytes()).hexdigest()
    helper_hash = hashlib.sha256(helper.read_bytes()).hexdigest()
    args.out.mkdir(parents=True, exist_ok=False, mode=0o700)
    home = Path(tempfile.mkdtemp(prefix="agend-g12d-mobile-", dir="/private/tmp"))
    env = dict(os.environ)
    env.pop("AGEND_INSTANCE", None)
    env.pop("TELEGRAM_BOT_TOKEN", None)
    env.pop("TELEGRAM_CHAT_ID", None)
    env["AGEND_HOME"] = str(home)
    child = None
    log = None
    result = {"scope": "local lifecycle" if args.local else "live private-chat callback to real daemon and TUI",
              "success": False, "home": str(home), "read_seen_in_tui": False, "acknowledged": False,
              "model_processes": 0, "mutation_budget": {"owned_tasks": 1, "notifications": 0 if args.local else 1, "deletions": 0 if args.local else 1},
              "binary_sha256": binary_hash,
              "helper_sha256": helper_hash}
    def invoke(mode):
        completed = subprocess.run([str(helper), mode, str(home)], env=env, capture_output=True, text=True, timeout=80 if mode == "delete" else 15)
        if completed.returncode:
            raise RuntimeError(f"helper {mode} failed: {completed.stderr.strip()}")
        return json.loads(completed.stdout)
    try:
        invoke("seed")
        if not args.local:
            env.update(credentials(args.credentials))
            chat = int(env["TELEGRAM_CHAT_ID"])
            (home / "config.toml").write_text(
                f"[telegram]\nchat_id = {chat}\nallow_user_ids = [{chat}]\n"
                "token = { kind = 'env', value = 'TELEGRAM_BOT_TOKEN' }\n")
        log = (args.out / "daemon.log").open("w")
        child = subprocess.Popen([str(binary), "daemon"], env=env, stdout=log, stderr=subprocess.STDOUT)
        # SQLite intentionally holds an exclusive lock while the daemon runs.
        # Stop after the bounded first-send window before inspecting receipts.
        time.sleep(0.5 if args.local else 5)
        child.send_signal(signal.SIGINT)
        child.wait(timeout=45)
        if child.returncode != 0:
            raise RuntimeError("first daemon boot did not stop cleanly")
        if not args.local:
            rows = db_rows(home, "SELECT delivery FROM telegram_outbox")
            if len(rows) != 1:
                raise RuntimeError("first boot did not produce exactly one notification")
            row = json.loads(rows[0][0])
            if len(row["message_ids"]) != 1 or row["next_part"] != len(row["parts"]) or row["in_flight"]:
                raise RuntimeError("notification receipt incomplete; no replay")
            result["owned_message_id"] = row["message_ids"][0]
        result["first_boot_exit"] = child.returncode
        child = subprocess.Popen([str(binary), "daemon"], env=env, stdout=log, stderr=subprocess.STDOUT)
        deadline = time.monotonic() + (30 if args.local else 300)
        notified = False
        marked = False
        local_acked = False
        while time.monotonic() < deadline:
            if child.poll() is not None:
                raise RuntimeError("owned daemon exited before completion")
            if not (home / "run/daemon.sock").exists():
                time.sleep(0.1)
                continue
            state = invoke("observe")
            if args.local:
                if not marked:
                    invoke("local-read")
                    marked = True
            elif not notified:
                notified = True
                print(json.dumps({"stage": "notification_ready_after_daemon_restart", "home": str(home)}), flush=True)
            if state["read"] and state["tui_read"] and state["open"] and not result["read_seen_in_tui"]:
                result["read_seen_in_tui"] = True
                print(json.dumps({"stage": "read_confirmed_in_tui_item_still_open"}), flush=True)
            if args.local and result["read_seen_in_tui"] and not local_acked:
                invoke("local-ack")
                local_acked = True
            if not state["open"]:
                if not result["read_seen_in_tui"]:
                    raise RuntimeError("item closed before read could be verified in TUI")
                result["acknowledged"] = True
                break
            time.sleep(0.3)
        if not result["acknowledged"]:
            raise RuntimeError("bounded mobile window expired")
        result["success"] = True
    except Exception as error:
        result["error"] = str(error)
    finally:
        if child is not None and child.poll() is None:
            child.send_signal(signal.SIGINT)
            try:
                child.wait(timeout=45)
            except subprocess.TimeoutExpired:
                child.kill()
                child.wait(timeout=5)
                result["success"] = False
                result["shutdown"] = "owned daemon required kill"
        result["daemon_exit"] = None if child is None else child.returncode
        if result["daemon_exit"] != 0:
            result["success"] = False
        if log is not None:
            log.close()
        try:
            result["saved_reads"] = db_rows(home, "SELECT read_key FROM attention_reads")
            result["saved_acknowledgment"] = db_rows(home, "SELECT failure_acknowledged FROM tasks WHERE id='t-mobile'")
            if result["success"] and (result["saved_acknowledgment"] != [(1,)] or result["saved_reads"] != [("task-failed:t-mobile#0",)]):
                raise RuntimeError("final durable state differs from observed completion")
            result["updates"] = db_rows(home, "SELECT update_id,outcome FROM telegram_updates ORDER BY update_id")
            if result["success"] and not args.local:
                outcomes = [row[1] for row in result["updates"] if row[1] in ("read", "accepted")]
                if outcomes != ["read", "accepted"]:
                    raise RuntimeError("live callback outcomes do not prove read then acknowledge")
            if args.local:
                result["cleanup"] = {"deleted": 0, "network_calls": 0}
            else:
                result["cleanup"] = invoke("delete")
            shutil.rmtree(home)
            result["home_removed"] = not home.exists()
        except Exception as error:
            result["success"] = False
            result["cleanup_error"] = str(error)
            result["home_retained"] = str(home)
        (args.out / "result.json").write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n")
        print(json.dumps({"success": result["success"], "evidence": str(args.out / 'result.json'), "home_removed": result.get("home_removed", False)}), flush=True)
    return 0 if result["success"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
