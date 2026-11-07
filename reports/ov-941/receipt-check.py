#!/usr/bin/python3
"""Run real producer and private real receipt fixture sequentially under FIFO."""
import os, subprocess, tempfile, json, hashlib
from pathlib import Path
root=Path(__file__).resolve().parents[2]
fixture=Path(tempfile.mkdtemp(prefix='ov941-receipt-',dir='/private/tmp'))
env={**os.environ,'FARCOOLER_OV941_FIXTURE':str(fixture)}
cargo='/Users/e-liang/.cargo/bin/cargo'
daemon=[cargo,'test','--offline','--locked','-p','farcooler-daemon','--lib','workflow_cli_payload_receipt_boundary','--','--ignored','--nocapture']
cli=[cargo,'test','--offline','--locked','-p','farcooler-cli','--bin','farcooler','workflow_cli_emit_receipt_fixture','--','--ignored','--nocapture']
for phase,command in [('seed',daemon),('emit',cli),('consume',daemon)]:
    print('PHASE',phase,'fixture',fixture,flush=True)
    subprocess.run(command,cwd=root,env={**env,'FARCOOLER_OV941_PHASE':phase},check=True)
record={'fixture':str(fixture),'sha256':{p.name:hashlib.sha256(p.read_bytes()).hexdigest() for p in fixture.glob('*.json')}}
print(json.dumps(record),flush=True)
