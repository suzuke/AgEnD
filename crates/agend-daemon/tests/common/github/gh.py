#!/usr/bin/env python3
"""Recorded GitHub response shapes backed by an owned native bare repository.

This producer is an offline fault fixture, not a replacement GitHub server.
All object ids and merge parents come from real git. No network is used.
"""
import json
import os
from pathlib import Path
import subprocess
import sys

root = Path(os.environ['G12_GITHUB_FIXTURE'])
config = json.loads((root / 'config.json').read_text())
repo = config['repository']
name = repo['full_name']
owner = name.split('/')[0]
bare = root / 'remote'


def git(*args):
    return subprocess.check_output([config['git'], '-c', 'core.hooksPath=/dev/null',
        '-C', str(bare), *args], text=True).strip()


def pull(value):
    result = json.loads((root / 'pull-template.json').read_text())
    result.update(number=value['number'], html_url=f'https://github.com/{name}/pull/{value["number"]}',
        body=value['body'], state=value['state'], merged=value['merged'],
        merge_commit_sha=value['merge_commit'])
    for side, branch in [('head', value['branch']), ('base', value['base'])]:
        result[side].update(ref=branch, label=f'{owner}:{branch}', sha=git('rev-parse', f'refs/heads/{branch}'))
        result[side]['repo'] = repo
    return result


args = sys.argv[1:]
assert args[0] == 'api'
method = args[args.index('--method') + 1]
endpoint = next(a for a in args if a.startswith('repos/'))
fields = dict(args[i+1].split('=', 1) for i, a in enumerate(args) if a == '--raw-field')
with (root / 'calls.jsonl').open('a') as out:
    out.write(json.dumps({'method': method, 'endpoint': endpoint})+'\n')
state_path = root / 'pull-state.json'
states = json.loads(state_path.read_text()) if state_path.exists() else []
parts = endpoint.split('/')
number = int(parts[4]) if len(parts) > 4 and parts[3] == 'pulls' else None
state = next((p for p in states if p['number'] == number), None)
prefix = f'repos/{name}'
status = 200
fault = None
if method == 'GET' and endpoint == prefix:
    result = repo
elif endpoint == prefix + '/pulls' and method == 'GET':
    result = [pull(p) for p in states if f'{owner}:{p["branch"]}' == fields['head'] and p['base'] == fields['base']]
elif endpoint == prefix + '/pulls' and method == 'POST':
    assert not any(p['branch'] == fields['head'] for p in states), 'duplicate PR creation'
    state = dict(number=len(states)+1, body=fields['body'], branch=fields['head'], base=fields['base'],
        state='open', merged=False, merge_commit=None)
    states.append(state)
    state_path.write_text(json.dumps(states))
    result = pull(state)
    status = 201
    fault = 'lose-create-reply'
elif endpoint == prefix + f'/pulls/{number}' and method == 'GET':
    assert state
    result = pull(state)
elif endpoint == prefix + f'/pulls/{number}' and method == 'PATCH':
    assert state and not state['merged'] and fields['state'] == 'closed'
    state['state'] = 'closed'
    state_path.write_text(json.dumps(states))
    result = pull(state)
    fault = 'lose-close-reply'
elif endpoint == prefix + f'/pulls/{number}/merge' and method == 'PUT':
    assert state and not state['merged'], 'duplicate merge'
    if (root / 'unknown-merge').exists():
        sys.exit(1)
    head = git('rev-parse', f'refs/heads/{state["branch"]}')
    base = git('rev-parse', f'refs/heads/{state["base"]}')
    if fields['sha'] != head:
        status, result = 409, {'message': 'Head branch was modified'}
    else:
        assert fields['merge_method'] == 'merge'
        tree = git('merge-tree', '--write-tree', base, head).splitlines()[0]
        merge = git('commit-tree', tree, '-p', base, '-p', head, '-m', 'Fixture PR merge')
        git('update-ref', f'refs/heads/{state["base"]}', merge, base)
        state.update(state='closed', merged=True, merge_commit=merge)
        state_path.write_text(json.dumps(states))
        result = {'merged': True, 'sha': merge}
        fault = 'lose-merge-reply'
elif method == 'GET' and endpoint.startswith(prefix + '/git/commits/'):
    sha = endpoint.rsplit('/', 1)[1]
    result = {'sha': git('rev-parse', sha),
        'parents': [{'sha': p} for p in git('show', '-s', '--format=%P', sha).split()]}
else:
    raise AssertionError((method, endpoint, fields))
if fault and (root / fault).exists():
    (root / fault).unlink()
    sys.exit(1)
header = (root / 'headers.bin').read_bytes()
first, rest = header.split(b'\n', 1)
first = b'HTTP/2.0 ' + str(status).encode() + b' Fixture'
sys.stdout.buffer.write(first+b'\n'+rest+json.dumps(result).encode())
