#!/usr/bin/python3
"""Exact old queue source, meaningful assertions, and byte-exact restoration."""
import errno,hashlib,json,os,re,subprocess,sys,time
from pathlib import Path
root=Path(__file__).resolve().parents[2]
source=root/'crates/daemon/src/workflow_notices.rs'
saved=source.read_bytes()
old=subprocess.check_output(['git','show','fae3f30d0c3d1c475f9dc4463243f293fd88b007:crates/daemon/src/workflow_notices.rs'],cwd=root)
out=root/'reports/ov-938/negative-sources'
out.mkdir(exist_ok=True)
(out/'queue.fixed.txt').write_bytes(saved)
(out/'queue.old.txt').write_bytes(old)
sha=lambda b:hashlib.sha256(b).hexdigest()
record={'old_sha256':sha(old),'fixed_sha256':sha(saved),'runs':[]}
try:
    source.write_bytes(old)
    for mode,label,marker in [('unproved','old-unproved','exit0 cannot settle without owned-group disappearance'),('descendant','old-drop','owned descendant group disappeared within bound')]:
        code=subprocess.call(['/usr/bin/python3','reports/ov-938/check.py',mode,label],cwd=root)
        log=(root/f'reports/ov-938/{label}.log').read_text()
        meaningful=code==101 and marker in log and 'test result: FAILED.' in log
        record['runs'].append({'mode':mode,'label':label,'exit':code,'assertion':marker,'meaningful':meaningful})
        assert meaningful, 'old-source run did not reach expected behavioral assertion'
        assert source.read_bytes()==old
        if mode=='descendant':
            ids=[int(x.strip()) for x in re.findall(r'live private receipt mode=drop: \[([^]]+)\]',log)[-1].split(',')]
            # Old-source helper is bounded by its own private 30s lifetime. Do
            # not signal a numeric identity after the old leader was reaped.
            probes=[]
            start=time.monotonic()
            while True:
                state={}
                for pid in [ids[0],ids[2]]:
                    try: os.kill(pid,0); state[str(pid)]='exists'
                    except OSError as e: state[str(pid)]=e.errno
                probes.append({'seconds':time.monotonic()-start,'pids':state})
                if all(v==errno.ESRCH for v in state.values()):break
                assert time.monotonic()-start<35,'bounded private old-source helper did not exit'
                time.sleep(.25)
            record['old_drop_process_receipts']={'ids':ids,'probes':probes,'no_followup_signals':True}
finally:
    source.write_bytes(saved)
    record['restored_sha256']=sha(source.read_bytes())
    record['exact_restoration']=source.read_bytes()==saved
    (root/'reports/ov-938/negative.json').write_text(json.dumps(record,indent=2)+'\n')
assert record['exact_restoration']
print('Both old-source behavioral assertions failed; exact source bytes restored.',flush=True)
