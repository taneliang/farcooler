#!/usr/bin/python3
import json, os, subprocess, sys
from pathlib import Path
root=Path(__file__).resolve().parents[2]
assert os.environ.get('HEAVY_LOCK_HELD')=='1'
cargo='/Users/e-liang/.cargo/bin/cargo'
commands=[
 ['/usr/bin/python3',str(root/'reports/ov-936/negatives.py')],
 [cargo,'test','--offline','-p','farcooler-store','--lib','engine_cutover_','--','--nocapture'],
 [cargo,'clippy','--offline','-p','farcooler-store','--all-targets','--features','testing','--','-D','warnings'],
 [cargo,'test','--offline','-p','farcooler-daemon','-p','farcooler-cli','workflow_','--','--nocapture'],
]
records=[]
for index,command in enumerate(commands):
 print('RESTORED',index,json.dumps(command),flush=True)
 result=subprocess.run(command,cwd=root)
 records.append(dict(command=command,exit=result.returncode));(root/'reports/ov-936/restored-steps.json').write_text(json.dumps(records,indent=2)+'\n')
 if result.returncode:sys.exit(result.returncode)
