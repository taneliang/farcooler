#!/usr/bin/python3
"""Read-only final provenance seal after the supervised checks have finished."""
import hashlib,json,subprocess,sqlite3
from pathlib import Path
root=Path(__file__).resolve().parents[2]
def sha(path):return hashlib.sha256(path.read_bytes()).hexdigest()
base='fae3f30d0c3d1c475f9dc4463243f293fd88b007'
assert subprocess.check_output(['git','rev-parse','HEAD'],cwd=root,text=True).strip()==base
products=['Cargo.lock','crates/store/Cargo.toml','crates/store/src/lib.rs','crates/store/src/migrate.rs','crates/store/src/store.rs','crates/store/src/workflow.rs','crates/store/src/workflow_effects.rs','crates/store/src/workflow_ownership.rs','crates/store/src/workflow_publication.rs','crates/store/src/engine_cutover.rs','crates/store/fixtures/engine_cutover_all_columns.sql','crates/store/examples/engine_cutover_rehearsal.rs']
products+=sorted(str(p.relative_to(root)) for p in (root/'crates/store/src/engine_cutover').rglob('*.rs'))
assert len(products)==len(set(products))
reserved=['crates/daemon/src/service.rs','crates/daemon/src/main.rs','crates/store/src/workflow_notices.rs']
for path in reserved:assert (root/path).read_bytes()==subprocess.check_output(['git','show',f'{base}:{path}'],cwd=root),path
run='finish-root-fences'
check=json.loads((root/f'reports/ov-936/{run}.json').read_text())
assert check['exit']==0 and check['source_unchanged'] and check['stopped'] is None
controls=json.loads((root/f'reports/ov-936/negative-steps-{run}.json').read_text())
assert len(controls)==6
for item in controls:assert item['meaningful'] and item['restored'] and item['original']==sha(root/item['path'])
readers=json.loads((root/'reports/ov-936/old-reader-receipts.json').read_text())
assert all(item['exit']==0 and item['unchanged'] for item in readers[-2:])
fixture=Path(json.loads((root/'reports/ov-936/cli-receipt.json').read_text())['fixture_path'])
assert str(fixture).startswith('/private/tmp/ov936-old-reader-') and not fixture.is_symlink()
conn=sqlite3.connect(f'file:{fixture}?mode=ro',uri=True)
metadata=dict(conn.execute('SELECT key,value FROM meta'))
assert metadata['schema_version']=='66' and metadata['compatible_down_to']=='66'
authority=conn.execute('SELECT phase,incarnation FROM engine_cutover_authority').fetchone();conn.close()
index=json.loads((root/'reports/ov-936/fixture-index.json').read_text())
for item in index:
 assert sha(root/item['signed_copy'])==item['gzip_sha256']
 assert sha(root/item['path'])==item['sha256']
report_files=[p for p in (root/'reports/ov-936').rglob('*') if p.is_file() and p.name!='freeze.json' and not (p.is_relative_to(root/'reports/ov-936/fixtures') and p.suffix in ['.json','.sqlite'])]
anchors={}
for relative,needles in {
 'crates/store/src/engine_cutover.rs':['pub fn preflight','pub fn engine_quiesce','pub fn engine_commit','pub fn open_for_cutover','fn required_resources'],
 'crates/store/src/engine_cutover/manifest.rs':['pub fn capture','pub fn conserved_by'],
 'crates/store/src/engine_cutover/rehearsal.rs':['pub fn backup','pub fn restore','pub fn recover','pub fn commit_permit','fn private_dir'],
 'crates/store/src/engine_cutover/verify.rs':['pub fn engine_verify','pub fn engine_release'],
}.items():
 lines=(root/relative).read_text().splitlines()
 anchors[relative]={needle:next(n for n,line in enumerate(lines,1) if needle in line) for needle in needles}
record=dict(state='frozen_builder_candidate_for_SAME_independent_review',base=base,map_pin='bdff40f0b2944173c2be5b5fa906dcd2af88f7bf',registry_before=65,registry_after=66,source_tables=87,source_columns=540,implicit_rowids_preserved=True,cutover_schema_objects=103,products={p:dict(sha256=sha(root/p),bytes=(root/p).stat().st_size) for p in products},reserved_unchanged=reserved,final_check=f'reports/ov-936/{run}.json',guard_controls=f'reports/ov-936/negative-steps-{run}.json',old_reader_pins=[item['pin'] for item in readers[-2:]],old_reader_fixture=dict(path=str(fixture),sha256=sha(fixture),metadata=metadata,authority=authority),evidence_files=len(index),raw_evidence_bytes=sum(item['bytes'] for item in index),signed_gzip_bytes=sum(item['gzip_bytes'] for item in index),evidence={str(p.relative_to(root)):dict(sha256=sha(p),bytes=p.stat().st_size) for p in sorted(report_files)},anchors=anchors,known_exclusions=['plan::tests::no_existing_table_gained_a_column (frozen-base ownership trigger counterexample)','workflow_notices::migration_tests::workflow_notice_migration_never_backfills_historical_events_or_rewrites_bytes (reserved test current-1 pin)'],pending=['explicit ov-937 bootstrap integration handoff','explicit ov-938 notice admission/test integration handoff','SAME independent data-safety/security review','reviewed937/938 composition and applicable combined gates','parent main CI/owner-copy/actual backup/installed cutover'],live_owner_work=False,whole_release_approval=False)
(root/'reports/ov-936/freeze.json').write_text(json.dumps(record,indent=2)+'\n')
print(json.dumps(dict(product_files=len(products),evidence_files=len(index),signed_gzip_bytes=record['signed_gzip_bytes'],reserved_unchanged=True,state=record['state'])))
