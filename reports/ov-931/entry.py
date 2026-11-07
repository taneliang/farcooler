#!/usr/bin/python3
"""Bind original warm target to this source after normal FIFO admission."""
import hashlib,json,os,subprocess,sys,time
from pathlib import Path
root=Path(__file__).resolve().parents[2]
target=Path(sys.argv[1])
assert os.environ.get('HEAVY_LOCK_HELD')=='1'
prior=json.loads((root/'reports/ov-928/security-green.json').read_text())
assert Path(prior['target'])==target and prior['exit']==0 and prior['source_unchanged']
probe=subprocess.run(['/usr/sbin/lsof','-t',str(target/'debug/.cargo-lock')],capture_output=True,text=True)
assert probe.returncode==1 and not probe.stdout.strip(), 'cargo cache occupied or probe failed'
assert os.getloadavg()[1]<20
# Relative dep-info in a shared target can otherwise accept a newer library
# from a sibling worktree. Mark local roots newer, without mutating any bytes.
paths=[root/'crates'/crate/'src'/name for crate in ['workflow','protocol','core','store','client','daemon','cli'] for name in ['lib.rs','main.rs']]
record={'time':time.time(),'target':str(target),'probe':probe.returncode,'load':os.getloadavg(),'roots':{}}
for path in paths:
    if path.exists():
        digest=hashlib.sha256(path.read_bytes()).hexdigest();os.utime(path,None)
        assert hashlib.sha256(path.read_bytes()).hexdigest()==digest
        record['roots'][str(path.relative_to(root))]=digest
with (root/'reports/ov-931/cache-holders.jsonl').open('a') as out:out.write(json.dumps(record)+'\n')
print('FIFO granted; original warm-target provenance/empty holder verified; local root mtimes bound',flush=True)
os.execv(sys.argv[2],sys.argv[2:])
