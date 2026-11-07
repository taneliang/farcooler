#!/usr/bin/python3
"""Reconcile old-source control and final focused metadata evidence."""
import hashlib,json,os,subprocess
from pathlib import Path
root=Path(__file__).resolve().parents[2]
assert os.environ.get('HEAVY_LOCK_HELD')=='1'
p=root/'crates/store/src/engine_cutover/rehearsal.rs';fixed=p.read_bytes()
old=subprocess.check_output(['git','show','e853d9c4ee07fbc07ab8e6d8bc0ce7d1658c57ba:crates/store/src/engine_cutover/rehearsal.rs'],cwd=root)
cargo='/Users/e-liang/.cargo/bin/cargo'
record={}
try:
 p.write_bytes(old)
 command=[cargo,'test','--offline','-p','farcooler-store','--lib','engine_cutover_storage_reconcile_refuses_outside_leaf_before_readback','--','--nocapture']
 result=subprocess.run(command,cwd=root,stdout=subprocess.PIPE,stderr=subprocess.STDOUT)
 (root/'reports/ov-936/storage-red-reconcile.log').write_bytes(result.stdout)
 record=dict(command=command,exit=result.returncode,meaningful=result.returncode==101 and b'reconcile must refuse outside leaf' in result.stdout and b'test result: FAILED' in result.stdout and b'could not compile' not in result.stdout)
finally:
 p.write_bytes(fixed)
 record.update(old=hashlib.sha256(old).hexdigest(),fixed=hashlib.sha256(fixed).hexdigest(),restored=hashlib.sha256(p.read_bytes()).hexdigest(),exact_restoration=p.read_bytes()==fixed)
 (root/'reports/ov-936/storage-reconcile-control.json').write_text(json.dumps(record,indent=2)+'\n')
assert record['meaningful'] and record['exact_restoration']
commands=[
 [cargo,'test','--offline','-p','farcooler-store','--lib','engine_cutover_storage_','--','--nocapture'],
 [cargo,'test','--offline','-p','farcooler-store','--lib','engine_cutover_f12_closed_backup_independent_restore_and_postbackup_receipts','--','--nocapture'],
 [cargo,'clippy','--offline','-p','farcooler-store','--all-targets','--features','testing','--','-D','warnings'],
 ['/usr/bin/python3',str(root/'scripts/migration-lint.py')],
 ['/usr/bin/python3',str(root/'scripts/file-size-budget.py')],
]
records=[]
for command in commands:
 result=subprocess.run(command,cwd=root)
 records.append(dict(command=command,exit=result.returncode));(root/'reports/ov-936/storage-boundary-steps.json').write_text(json.dumps(records,indent=2)+'\n')
 if result.returncode:raise SystemExit(result.returncode)
