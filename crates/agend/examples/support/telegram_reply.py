#!/usr/bin/env python3
"""One owned free-text ask; local dry run or bounded dedicated-chat test."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import tempfile
import time
from telegram_mobile import credentials, db_rows


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--target', required=True, type=Path)
    parser.add_argument('--out', required=True, type=Path)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument('--local', action='store_true')
    mode.add_argument('--credentials', type=Path)
    args = parser.parse_args()
    binary = args.target / 'debug/agend'
    helper = args.target / 'debug/examples/telegram_reply_probe'
    hashes = {p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in (binary, helper)}
    private = {} if args.local else credentials(args.credentials)
    args.out.mkdir(parents=True, exist_ok=False, mode=0o700)
    log = (args.out / 'daemon.log').open('w')
    try:
        home = Path(tempfile.mkdtemp(prefix='agend-g12d-reply-', dir='/private/tmp'))
    except Exception:
        log.close()
        raise
    env = dict(os.environ)
    for key in ('AGEND_INSTANCE', 'TELEGRAM_BOT_TOKEN', 'TELEGRAM_CHAT_ID'):
        env.pop(key, None)
    env.update(private)
    env['AGEND_HOME'] = str(home)
    result = {'success': False, 'scope': 'local' if args.local else 'live private-chat free reply',
              'hashes': hashes, 'home': str(home), 'model_processes': 0,
              'budget': {'asks': 1, 'test_processes': 1, 'seconds': 30 if args.local else 300,
                         'expected_notifications': 0 if args.local else 2, 'cleanup_delete_limit': 0 if args.local else 3}}
    child = None
    removed = False

    def invoke(operation):
        call = subprocess.run([str(helper), operation, str(home)], env=env,
                              capture_output=True, text=True, timeout=150 if operation == 'delete' else 20)
        if call.returncode:
            raise RuntimeError(f'{operation}: {call.stderr.strip()}')
        return json.loads(call.stdout)

    try:
        invoke('seed')
        if not args.local:
            chat = int(private['TELEGRAM_CHAT_ID'])
            (home / 'config.toml').write_text(
                f'[telegram]\nchat_id = {chat}\nallow_user_ids = [{chat}]\n'
                "token = { kind = 'env', value = 'TELEGRAM_BOT_TOKEN' }\n")
        child = subprocess.Popen([str(binary), 'daemon'], env=env, stdout=log, stderr=subprocess.STDOUT)
        deadline = time.monotonic() + result['budget']['seconds']
        saw_open = False
        while time.monotonic() < deadline:
            if child.poll() is not None:
                raise RuntimeError('owned daemon exited')
            if not (home / 'run/daemon.sock').exists():
                time.sleep(.1)
                continue
            state = invoke('observe')
            if state['unexpected_attention']:
                raise RuntimeError('unexpected attention; stopping live probe')
            if state['open'] and not saw_open:
                saw_open = True
                if args.local:
                    invoke('local-answer')
                else:
                    print(json.dumps({'stage': 'waiting_for_reply', 'seconds': 300}), flush=True)
            elif saw_open and not state['open']:
                # Allow the one action-result notification to finish before shutdown.
                time.sleep(0 if args.local else 3)
                break
            time.sleep(.25)
        else:
            raise RuntimeError('bounded reply window expired')
        result['observed_question_then_answer'] = True
    except Exception as error:
        result['error'] = str(error)
    finally:
        if child is not None and child.poll() is None:
            try:
                removed = invoke('remove')['removed']
            except Exception as error:
                result['remove_error'] = str(error)
            child.send_signal(signal.SIGINT)
            try:
                child.wait(timeout=45)
            except subprocess.TimeoutExpired:
                child.kill()
                child.wait(timeout=5)
                result['shutdown_error'] = 'owned daemon required kill'
        log.close()
        if child is None:
            shutil.rmtree(home)
            result['home_removed'] = True
        result['daemon_exit'] = None if child is None else child.returncode
        result['instance_removed'] = removed
        if child is not None:
            try:
                result['inspection'] = invoke('inspect')
                turns = db_rows(home, "SELECT delivered FROM ask_turns WHERE json_extract(turn,'$.entry')='answer'")
                messages = db_rows(home, "SELECT body FROM messages WHERE to_instance='reply-probe'")
                result['delivered_answers'] = turns
                result['inbox_messages'] = [json.loads(row[0]) for row in messages]
                result['updates'] = db_rows(home, 'SELECT update_id,outcome FROM telegram_updates ORDER BY update_id')
                source = 'cli' if args.local else 'telegram'
                answer = result['inspection'].get('answer')
                result['success'] = bool(result.get('observed_question_then_answer') and removed
                    and result['daemon_exit'] == 0 and result['inspection']['exact_answer']
                    and answer.get('source') == source and turns == [(1,)]
                    and result['inbox_messages'] == [answer]
                    and (args.local or [r[1] for r in result['updates'] if r[1] == 'accepted'] == ['accepted']))
                if args.local:
                    result['cleanup'] = {'deleted': 0, 'network_calls': 0}
                else:
                    rows = [json.loads(r[0]) for r in db_rows(home, 'SELECT delivery FROM telegram_outbox')]
                    (home / 'owned-receipts.json').write_text(json.dumps(rows))
                    result['cleanup'] = invoke('delete')
                result['holder_cleanup'] = invoke('holders')
                if not removed or result['daemon_exit'] != 0 or not result['holder_cleanup']['empty']:
                    raise RuntimeError('owned process cleanup unconfirmed; home retained')
                shutil.rmtree(home)
                result['home_removed'] = not home.exists()
            except Exception as error:
                result['success'] = False
                result['cleanup_error'] = str(error)
        (args.out / 'result.json').write_text(json.dumps(result, ensure_ascii=False, indent=2) + '\n')
        print(json.dumps({'success': result['success'], 'evidence': str(args.out / 'result.json'),
                          'home_removed': result.get('home_removed', False)}), flush=True)
    return 0 if result['success'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
