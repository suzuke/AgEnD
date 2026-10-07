#!/usr/bin/env python3
"""Replace only the fixture's fixed network URL with its owned bare repo."""
import json
import os
from pathlib import Path
import sys
root = Path(os.environ['G12_GITHUB_FIXTURE'])
config = json.loads((root / 'config.json').read_text())
url = 'https://github.com/' + config['repository']['full_name'] + '.git'
args = [str(root / 'remote') if a == url else a for a in sys.argv[1:]]
os.execv(config['git'], [config['git'], *args])
