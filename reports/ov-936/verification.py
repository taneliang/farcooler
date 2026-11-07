#!/usr/bin/python3
"""Actual old-reader/meaningful red/restored checks under one normal FIFO lease."""
import json, os, subprocess, sys
from pathlib import Path
root=Path(__file__).resolve().parents[2]
assert os.environ.get('HEAVY_LOCK_HELD')=='1'
cargo='/Users/e-liang/.cargo/bin/cargo'
fixture=json.loads((root/'reports/ov-936/cli-receipt.json').read_text())['fixture_path']
commands=[
 ['/usr/bin/python3',str(root/'reports/ov-936/old-reader.py')],
 ['/usr/bin/python3',str(root/'reports/ov-936/negatives.py')],
 [cargo,'test','--offline','-p','farcooler-store','--lib','engine_cutover_','--','--nocapture'],
 [cargo,'clippy','--offline','-p','farcooler-store','--all-targets','--features','testing','--','-D','warnings'],
 [cargo,'test','--offline','-p','farcooler-daemon','-p','farcooler-cli','workflow_','--','--nocapture'],
]
records=[]
for index,command in enumerate(commands):
 print('VERIFY',index,json.dumps(command),flush=True)
 bound=command if index==0 else ['/usr/bin/python3',str(root/'reports/ov-936/entry.py'),os.environ['CARGO_TARGET_DIR'],*command]
 result=subprocess.run(bound,cwd=root,env={**os.environ,'OV936_OLD_READER_FIXTURE':fixture} if index==0 else os.environ)
 records.append(dict(command=command,exit=result.returncode))
 (root/'reports/ov-936/verification-steps.json').write_text(json.dumps(records,indent=2)+'\n')
 if result.returncode:sys.exit(result.returncode)
