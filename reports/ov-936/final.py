#!/usr/bin/python3
"""Sequential scoped final checks under one normal FIFO lease, no nested target."""
import json, os, subprocess, sys
from pathlib import Path
root=Path(__file__).resolve().parents[2]
assert os.environ.get('HEAVY_LOCK_HELD')=='1'
cargo='/Users/e-liang/.cargo/bin/cargo'
commands=[
 [cargo,'test','--offline','-p','farcooler-store','--lib','engine_cutover_','--','--nocapture'],
 [cargo,'test','--offline','-p','farcooler-store','--lib','--','--nocapture','--skip','plan::tests::no_existing_table_gained_a_column','--skip','workflow_notices::migration_tests::workflow_notice_migration_never_backfills_historical_events_or_rewrites_bytes'],
 [cargo,'clippy','--offline','-p','farcooler-store','--all-targets','--features','testing','--','-D','warnings'],
 ['/usr/bin/python3',str(root/'scripts/migration-lint.py')],
 ['/usr/bin/python3',str(root/'scripts/file-size-budget.py')],
 ['/usr/bin/python3',str(root/'reports/ov-936/cli.py')],
]
records=[]
for index,command in enumerate(commands):
 print('CHECK',index,json.dumps(command),flush=True)
 result=subprocess.run(command,cwd=root)
 records.append(dict(command=command,exit=result.returncode))
 (root/'reports/ov-936/final-steps.json').write_text(json.dumps(records,indent=2)+'\n')
 if result.returncode:sys.exit(result.returncode)
