#!/usr/bin/env python3
"""Exercise the delivered executable's cache without a model or development PATH."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time

parser = argparse.ArgumentParser()
parser.add_argument('binary', type=Path)
parser.add_argument('--output', type=Path, required=True)
args = parser.parse_args()
binary = args.binary.resolve()
args.output.parent.mkdir(parents=True, exist_ok=True)
root = Path(tempfile.mkdtemp(prefix='中文 cache verification ', dir=args.output.parent.resolve()))
env = dict(os.environ, PATH='/usr/bin:/bin:/usr/sbin:/sbin', AGENTCHECK_CACHE_DIR=str(root))
for name in ('MODEL_API_KEY', 'OPENAI_API_KEY', 'AGENTCHECK_OMP_PATH', 'AGENTCHECK_GUI_PATH'):
    env.pop(name, None)
checks = []
def verify(name):
    start = time.monotonic()
    result = subprocess.run([str(binary), '--offline-self-test'], env=env, capture_output=True, text=True, timeout=120, check=True)
    data = json.loads(result.stdout)
    assert data['bundled'] and data['model_requests'] == 0
    checks.append({'check': name, 'passed': True, 'elapsed_seconds': round(time.monotonic() - start, 3)})
    return data

data = verify('first_extraction_chinese_and_space_path')
verify('cached_restart')
omp = Path(data['omp'])
expected = hashlib.sha256(omp.read_bytes()).hexdigest()
omp.chmod(0o600)
verify('missing_execute_permission_repaired')
assert omp.stat().st_mode & 0o100
with omp.open('ab') as stream:
    stream.write(b'agentcheck-disposable-cache-corruption')
verify('modified_binary_repaired')
assert hashlib.sha256(omp.read_bytes()).hexdigest() == expected
if data['gui']:
    gui = Path(data['gui'])
    gui.chmod(0o600)
    verify('gui_execute_permission_repaired')
    assert gui.stat().st_mode & 0o100
record = {'binary': str(binary), 'sha256': hashlib.sha256(binary.read_bytes()).hexdigest(), 'size_bytes': binary.stat().st_size, 'omp_version': data['omp_version'], 'model_requests': 0, 'restricted_path': env['PATH'], 'checks': checks, 'cache_directory': str(root), 'scope': 'Local macOS host with development tools excluded from PATH; not a clean-machine or firewall isolation test.'}
args.output.write_text(json.dumps(record, ensure_ascii=False, indent=2) + '\n')
print(json.dumps({key: value for key, value in record.items() if key not in ('cache_directory',)}, ensure_ascii=False, indent=2))
