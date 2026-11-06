#!/usr/bin/python3
"""Offline adversarial red: remove one session correlation gate, always restore."""
import hashlib, json, subprocess
from pathlib import Path
root = Path(__file__).resolve().parents[2]
source = root/'crates/daemon/src/workflow_runtime/coding_native.rs'
original = source.read_bytes()
gate = b'            if session.as_ref() != Some(sid) { continue; }'
assert original.count(gate) == 1
mutated = original.replace(gate, b'            // deliberate offline mutant: session correlation removed')
sha = lambda b: hashlib.sha256(b).hexdigest()
record = dict(original=sha(original), mutant=sha(mutated))
try:
    source.write_bytes(mutated)
    record['exit'] = subprocess.call(['/usr/bin/python3', str(root/'reports/ov-928/check.py'), 'continuation-native-red'], cwd=root)
finally:
    source.write_bytes(original)
    record['restored'] = sha(source.read_bytes())
    (root/'reports/ov-928/continuation-native-mutation.json').write_text(json.dumps(record, indent=2)+'\n')
assert record['restored'] == record['original']
assert record['exit'] != 0, 'session mutant unexpectedly survived'
