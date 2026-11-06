#!/usr/bin/python3
"""Verify released cache immediately after normal FIFO allocation, before use."""
import json, subprocess, sys
from pathlib import Path
root=Path(__file__).resolve().parents[2]
target=Path(sys.argv[1])
old=json.loads((root/'reports/ov-927/final-client.json').read_text())
assert Path(old['target'])==target and old['exit']==0 and old['source_unchanged']
probe=subprocess.run(['/usr/sbin/lsof','-t',str(target/'debug/.cargo-lock')],capture_output=True,text=True)
assert not probe.stdout.strip(), 'warm cargo target has not actually released: '+probe.stdout
print('FIFO allocated; reviewed ov-927 cache provenance verified; actual cargo holder empty',flush=True)
sys.exit(subprocess.call(sys.argv[2:]))
