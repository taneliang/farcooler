#!/usr/bin/python3
"""Bind original warm target to this source after normal FIFO admission."""
import hashlib,json,os,subprocess,sys,time
from pathlib import Path
root=Path(__file__).resolve().parents[2]
target=Path(sys.argv[1])
assert os.environ.get('HEAVY_LOCK_HELD')=='1'
prior=json.loads((root/'reports/ov-928/security-green.json').read_text())
assert Path(prior['target'])==target and prior['exit']==0 and prior['source_unchanged']
probes={}
for profile in ['debug','release','aarch64-linux-android/debug','wasm32-unknown-unknown/debug']:
    lock=target/profile/'.cargo-lock'
    if not lock.exists(): continue
    result=subprocess.run(['/usr/sbin/lsof','-t',str(lock)],capture_output=True,text=True)
    assert result.returncode==1 and not result.stdout.strip() and not result.stderr.strip(), 'cargo cache occupied or probe failed'
    probes[profile]={'exit':result.returncode,'out':result.stdout,'err':result.stderr}
probe=result
assert os.getloadavg()[1]<20
# Relative dep-info in a shared target can otherwise accept a newer library
# from a sibling worktree. Mark local roots newer, without mutating any bytes.
paths=[root/'crates'/crate/'src'/name for crate in ['workflow','protocol','core','store','client','daemon','cli','tmux'] for name in ['lib.rs','main.rs']]
dep_info={}
for crate in ['protocol','workflow','store','daemon','cli','tmux']:
    for path in sorted((target/'debug/deps').glob('farcooler_'+crate+'-*.d')):
        raw=path.read_bytes()
        dep_info[path.name]={'sha256':hashlib.sha256(raw).hexdigest(),'mtime_ns':path.stat().st_mtime_ns,
            'workflow_config':b'workflow_config.rs' in raw,'service_owner':b'service_owner.rs' in raw,
            'workflow_ownership':b'workflow_ownership.rs' in raw,'relative_src':b' src/' in raw}
record={'dep_info_before':dep_info,'time':time.time(),'target':str(target),'probe':probe.returncode,'profiles':probes,'load':os.getloadavg(),'roots':{}}
for path in paths:
    if path.exists():
        digest=hashlib.sha256(path.read_bytes()).hexdigest();os.utime(path,None)
        assert hashlib.sha256(path.read_bytes()).hexdigest()==digest
        record['roots'][str(path.relative_to(root))]=digest
with (root/'reports/ov-933/cache-holders.jsonl').open('a') as out:out.write(json.dumps(record)+'\n')
print('FIFO granted; original warm-target provenance/empty holder verified; local root mtimes bound',flush=True)
os.execv(sys.argv[2],sys.argv[2:])
