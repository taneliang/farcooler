#!/usr/bin/python3
import json, os, subprocess, uuid
from pathlib import Path
root=Path(__file__).resolve().parents[2]
assert os.environ.get('HEAVY_LOCK_HELD')=='1'
name='ov936-old-reader-'+str(uuid.uuid4())
command=['/Users/e-liang/.cargo/bin/cargo','run','--offline','-p','farcooler-store','--features','testing','--example','engine_cutover_rehearsal','--','/private/tmp',name]
result=subprocess.run(command,cwd=root,capture_output=True,text=True)
print(result.stdout,flush=True);print(result.stderr,flush=True)
record=dict(command=command,exit=result.returncode,fixture_path=str(Path('/private/tmp')/name/'db.sqlite'))
(root/'reports/ov-936/cli-receipt.json').write_text(json.dumps(record,indent=2)+'\n')
raise SystemExit(result.returncode)
