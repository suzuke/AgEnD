#!/usr/bin/env python3
"""Replace only the fixture's fixed network URL with its owned bare repo."""
import json
import os
from pathlib import Path
import sys
import subprocess
root = Path(os.environ['G12_GITHUB_FIXTURE'])
config = json.loads((root / 'config.json').read_text())
url = 'https://github.com/' + config['repository']['full_name'] + '.git'
args = [str(root / 'remote') if a == url else a for a in sys.argv[1:]]
deleting = any(a.startswith(':refs/heads/') for a in args)
if deleting:
    with (root / 'delete-calls.jsonl').open('a') as out:
        out.write(json.dumps(args) + '\n')
    result = subprocess.run([config['git'], *args], check=False)
    if result.returncode == 0 and (root / 'recreate-after-delete').exists():
        lease = next(a for a in args if a.startswith('--force-with-lease='))
        ref, head = lease.split('=', 1)[1].split(':', 1)
        subprocess.run([config['git'], '-C', str(root / 'remote'), 'update-ref', ref, head], check=True)
        (root / 'recreate-after-delete').unlink()
        sys.exit(1)
    if (root / 'lose-delete-reply').exists():
        (root / 'lose-delete-reply').unlink()
        sys.exit(1)
    sys.exit(result.returncode)
os.execv(config['git'], [config['git'], *args])
