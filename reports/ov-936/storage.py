#!/usr/bin/python3
"""Focused S1/S2 checks; canonical lock owned by the outer supervisor."""
import json, os, subprocess, sys
from pathlib import Path
root=Path(__file__).resolve().parents[2]
assert os.environ.get('HEAVY_LOCK_HELD')=='1'
cargo='/Users/e-liang/.cargo/bin/cargo'
commands=[
 [cargo,'test','--offline','-p','farcooler-store','--lib','engine_cutover_storage_','--','--nocapture'],
 [cargo,'test','--offline','-p','farcooler-store','--lib','workflow::tests::','--','--nocapture'],
 [cargo,'test','--offline','-p','farcooler-store','--lib','engine_cutover_f09_','--','--nocapture'],
 [cargo,'clippy','--offline','-p','farcooler-store','--all-targets','--features','testing','--','-D','warnings'],
]
records=[]
for command in commands:
 print('STORAGE',json.dumps(command),flush=True)
 result=subprocess.run(command,cwd=root)
 records.append(dict(command=command,exit=result.returncode));(root/'reports/ov-936/storage-steps.json').write_text(json.dumps(records,indent=2)+'\n')
 if result.returncode:sys.exit(result.returncode)
