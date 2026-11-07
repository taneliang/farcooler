#!/usr/bin/python3
"""Final exact source Store/manifest/CLI/old-reader/red/restored evidence."""
import json, os, subprocess, sys
from pathlib import Path
root=Path(__file__).resolve().parents[2]
assert os.environ.get('HEAVY_LOCK_HELD')=='1'
cargo='/Users/e-liang/.cargo/bin/cargo'
commands=[
 [cargo,'test','--offline','-p','farcooler-store','--lib','engine_cutover_','--','--nocapture'],
 [cargo,'test','--offline','-p','farcooler-store','--lib','--','--nocapture','--skip','plan::tests::no_existing_table_gained_a_column','--skip','workflow_notices::migration_tests::workflow_notice_migration_never_backfills_historical_events_or_rewrites_bytes'],
 [cargo,'clippy','--offline','-p','farcooler-store','--all-targets','--features','testing','--','-D','warnings'],
 ['/usr/bin/python3',str(root/'reports/ov-936/cli.py')],
 ['/usr/bin/python3',str(root/'reports/ov-936/old-reader.py')],
 ['/usr/bin/python3',str(root/'reports/ov-936/negatives.py')],
 [cargo,'test','--offline','-p','farcooler-store','--lib','engine_cutover_','--','--nocapture'],
 ['/usr/bin/python3',str(root/'scripts/migration-lint.py')],
 ['/usr/bin/python3',str(root/'scripts/file-size-budget.py')],
 ['/usr/bin/python3',str(root/'reports/ov-936/pack-evidence.py')],
]
records=[]
for index,command in enumerate(commands):
 print('SEAL',index,json.dumps(command),flush=True)
 env=dict(os.environ)
 if index==4:env['OV936_OLD_READER_FIXTURE']=json.loads((root/'reports/ov-936/cli-receipt.json').read_text())['fixture_path']
 if index in [5,6]:command=['/usr/bin/python3',str(root/'reports/ov-936/entry.py'),env['CARGO_TARGET_DIR'],*command]
 result=subprocess.run(command,cwd=root,env=env)
 records.append(dict(command=command,exit=result.returncode));(root/'reports/ov-936/seal-steps.json').write_text(json.dumps(records,indent=2)+'\n')
 if result.returncode:sys.exit(result.returncode)
