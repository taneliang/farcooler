#!/usr/bin/python3
"""Finite meaningful guard removals, exact source restoration, one owned FIFO."""
import hashlib, json, os, subprocess, sys
from pathlib import Path
root=Path(__file__).resolve().parents[2]
assert os.environ.get('HEAVY_LOCK_HELD')=='1'
cargo='/Users/e-liang/.cargo/bin/cargo'
cases=[
 ('namespace','crates/store/src/engine_cutover.rs','if registry==66 && farcooler_workflow::digest(&cutover_objects)!=CUTOVER_SCHEMA_HASH','if false && registry==66 && farcooler_workflow::digest(&cutover_objects)!=CUTOVER_SCHEMA_HASH','engine_cutover_unknown_cutover_prefixed_schema','unknown cutover-prefixed schema must not disappear'),
 ('incarnation','crates/store/src/engine_cutover.rs','SELECT incarnation=?1 AND phase IN','SELECT (?1>=0) AND phase IN','engine_cutover_f09_existing_engine_claims','old Store handle cannot write after transfer'),
 ('legacy-writer','crates/store/src/engine_cutover.rs',"BEGIN SELECT RAISE(ABORT,'legacy execution authority retired'); END;","BEGIN SELECT 1; END;",'engine_cutover_f04_landing_routes','cutover schema objects differ from pinned schema66'),
 ('conservation','crates/store/src/engine_cutover.rs','if !plan.raw.conserved_by(&current)','if false && !plan.raw.conserved_by(&current)','engine_cutover_stale_source_and_per_field_corruption','assertion failed'),
 ('restore-freeze','crates/store/src/engine_cutover/rehearsal.rs',"SET phase='quiesced',incarnation=incarnation+1","SET phase='legacy',incarnation=incarnation+1",'engine_cutover_f11_restored_pending','left: "legacy"'),
 ('restore-quarantine','crates/store/src/engine_cutover/rehearsal.rs','            tx.execute("INSERT INTO engine_cutover_quarantine VALUES(?1,?2) ON CONFLICT(resource) DO NOTHING",rusqlite::params![fact.resource,"post-backup external effect; restored pending is not retry permission"])?;','            let _ = &fact;','engine_cutover_f11_restored_pending','post-backup quarantine must survive forward release'),
]
run=os.environ.get('OV936_RUN_LABEL','unlabelled')
folder=root/'reports/ov-936/negative-sources'/run;folder.mkdir(exist_ok=True)
records=[]
for name,relative,old,new,test,assertion in cases:
 path=root/relative;original=path.read_bytes();text=original.decode();assert text.count(old)==1,(name,text.count(old))
 mutant=text.replace(old,new).encode();(folder/f'{name}.original.txt').write_bytes(original);(folder/f'{name}.mutant.txt').write_bytes(mutant)
 command=[cargo,'test','--offline','-p','farcooler-store','--lib',test,'--','--nocapture']
 try:
  path.write_bytes(mutant)
  result=subprocess.run(command,cwd=root,capture_output=True,text=True,env={**os.environ,"OV936_EVIDENCE_OUTPUT":str(root/"reports/ov-936/fixtures"/f"negative-{run}-{name}")})
  output=result.stdout+result.stderr
  (root/f'reports/ov-936/negative-{run}-{name}.log').write_text(output)
 finally:
  path.write_bytes(original)
 restored=path.read_bytes()==original
 meaningful=result.returncode!=0 and 'test result: FAILED' in output and assertion in output and 'could not compile' not in output
 record=dict(name=name,path=relative,command=command,original=hashlib.sha256(original).hexdigest(),mutant=hashlib.sha256(mutant).hexdigest(),exit=result.returncode,meaningful=meaningful,restored=restored,assertion=assertion)
 records.append(record);(root/f'reports/ov-936/negative-steps-{run}.json').write_text(json.dumps(records,indent=2)+'\n')
 print(json.dumps(record),flush=True)
 if not meaningful or not restored:sys.exit(1)
