#!/usr/bin/python3
"""Prove the provider-binding regression test fails without its gate, then restore."""
import hashlib
import json
from pathlib import Path
import subprocess
import sys

root = Path(__file__).resolve().parents[2]
source = root / 'crates/daemon/src/workflow_runtime/coding_execute.rs'
original = source.read_bytes()
needle = b'receipt.binding != digest(binding) || receipt.repository != config.ci.repository'
assert original.count(needle) == 1
mutant = original.replace(needle, b'receipt.repository != config.ci.repository')
digest = lambda value: hashlib.sha256(value).hexdigest()
try:
    source.write_bytes(mutant)
    code = subprocess.call(['/usr/bin/python3', str(root/'reports/ov-928/check.py'), 'binding-red'], cwd=root)
finally:
    source.write_bytes(original)
assert source.read_bytes() == original
record = dict(original=digest(original), mutant=digest(mutant), restored=digest(source.read_bytes()), red_exit=code)
(root/'reports/ov-928/binding-mutation.json').write_text(json.dumps(record, indent=2)+'\n')
print(json.dumps(record), flush=True)
sys.exit(0 if code == 101 else 1)
