#!/usr/bin/python3
"""Old e853 source controls, exact restoration, then affected green checks."""
import hashlib,json,os,subprocess,sys
from pathlib import Path
root=Path(__file__).resolve().parents[2]
assert os.environ.get('HEAVY_LOCK_HELD')=='1'
base='e853d9c4ee07fbc07ab8e6d8bc0ce7d1658c57ba'
paths=['crates/store/src/engine_cutover.rs','crates/store/src/engine_cutover/rehearsal.rs','crates/store/src/workflow_effects.rs']
fixed={p:(root/p).read_bytes() for p in paths}
old={p:subprocess.check_output(['git','show',base+':'+p],cwd=root) for p in paths}
hashof=lambda data:hashlib.sha256(data).hexdigest()
record={'base':base,'fixed':{p:hashof(b) for p,b in fixed.items()},'old':{p:hashof(b) for p,b in old.items()},'controls':[]}
cargo='/Users/e-liang/.cargo/bin/cargo'
cases=[
 ('metadata','engine_cutover_storage_metadata_restore_and_reconcile_refuse_unsafe_leaves','restore must refuse symlink'),
 ('journal','engine_cutover_storage_restore_refuses_replaced_journal_and_effect_leaves','restore journal must refuse journal'),
 ('preflight','engine_cutover_storage_preflight_quarantines_pending_lossy_effect','lossy pending Effect requires workspace quarantine'),
 ('claim','engine_cutover_storage_unknown_effects_preserve_raw_and_never_claim_or_admit','claim must refuse pending/effect'),
 ('admit','engine_cutover_storage_admit_rechecks_payload_after_valid_claim','admit must recheck immutable source payload'),
]
try:
 for p,data in old.items():(root/p).write_bytes(data)
 for label,test,assertion in cases:
    command=[cargo,'test','--offline','-p','farcooler-store','--lib',test,'--','--nocapture']
    result=subprocess.run(command,cwd=root,stdout=subprocess.PIPE,stderr=subprocess.STDOUT)
    (root/f'reports/ov-936/storage-red-{label}.log').write_bytes(result.stdout)
    meaningful=result.returncode==101 and assertion.encode() in result.stdout and b'test result: FAILED' in result.stdout and b'could not compile' not in result.stdout
    record['controls'].append({'name':label,'command':command,'exit':result.returncode,'assertion':assertion,'meaningful':meaningful})
    print('OLD SOURCE',label,'exit',result.returncode,'meaningful',meaningful,flush=True)
    if not meaningful:raise RuntimeError('control failed at wrong boundary: '+label)
finally:
 for p,data in fixed.items():(root/p).write_bytes(data)
 record['restored']={p:hashof((root/p).read_bytes()) for p in paths}
 record['exact_restoration']=record['restored']==record['fixed']
 (root/'reports/ov-936/storage-controls.json').write_text(json.dumps(record,indent=2)+'\n')
assert record['exact_restoration']
# Rebuild restored sources normally; all positive controls must still pass.
subprocess.run(['/usr/bin/python3',str(root/'reports/ov-936/storage.py')],cwd=root,check=True)
