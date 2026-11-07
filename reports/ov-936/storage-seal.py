#!/usr/bin/python3
"""Seal only this S1/S2 delta and its private artifacts; preserve baseline evidence."""
import gzip,hashlib,json,subprocess,tempfile
from pathlib import Path
root=Path(__file__).resolve().parents[2];report=root/'reports/ov-936'
base='e853d9c4ee07fbc07ab8e6d8bc0ce7d1658c57ba'
def binding(data):return {'sha256':hashlib.sha256(data).hexdigest(),'bytes':len(data)}
baseline=json.loads((report/'freeze.json').read_text())
for path,expected in baseline['evidence'].items():assert binding((root/path).read_bytes())==expected,path
fixture_index={}
for label in ['storage-green','storage-restored','storage-final']:
 for path in sorted((report/'fixtures'/label).glob('*')):
    if path.suffix not in ['.sqlite','.json']:continue
    raw=path.read_bytes();destination=path.with_suffix(path.suffix+'.gz')
    packed=gzip.compress(raw,mtime=0)
    if destination.exists():assert destination.read_bytes()==packed
    else:destination.write_bytes(packed)
    assert gzip.decompress(packed)==raw
    fixture_index[str(destination.relative_to(root))]={'raw':binding(raw),'gzip':binding(packed),'raw_local':str(path)}
(report/'storage-fixture-index.json').write_text(json.dumps(fixture_index,indent=2)+'\n')
products=['crates/store/src/engine_cutover.rs','crates/store/src/engine_cutover/rehearsal.rs','crates/store/src/engine_cutover/tests.rs','crates/store/src/workflow_effects.rs','crates/store/src/engine_cutover/tests/storage_safety.rs']
# Every inherited file outside the four modified existing products must match base.
changed=subprocess.check_output(['git','diff','--name-only',base],cwd=root,text=True).splitlines()
assert sorted(changed)==sorted(products[:-1]),changed
for path,expected in baseline['products'].items():
 if path not in products:assert binding((root/path).read_bytes())==expected,path
final=json.loads((report/'storage-final.json').read_text());assert final['exit']==0 and final['source_unchanged'] and final['leader_reaped'] and final['group_release']=='ESRCH'
controls=json.loads((report/'storage-controls.json').read_text());assert controls['exact_restoration'] and all(c['meaningful'] for c in controls['controls'])
reconcile=json.loads((report/'storage-reconcile-control.json').read_text());assert reconcile['meaningful'] and reconcile['exact_restoration']
evidence={str(p.relative_to(root)):binding(p.read_bytes()) for p in sorted(report.glob('storage-*')) if p.is_file() and p.name!='storage-freeze.json'}
evidence['reports/ov-936/storage.py']=binding((report/'storage.py').read_bytes())
evidence.update({p:binding((root/p).read_bytes()) for p in fixture_index})
record={'state':'frozen_S1_S2_delta_for_SAME_audit','base':base,'branch':subprocess.check_output(['git','branch','--show-current'],cwd=root,text=True).strip(),'products':{p:binding((root/p).read_bytes()) for p in products},'baseline_evidence_verified':len(baseline['evidence']),'baseline_freeze':binding((report/'freeze.json').read_bytes()),'unchanged_baseline_products':len(baseline['products'])-4,'canonical_queue':str(Path(tempfile.gettempdir())/'lane-gate.lock'),'native_tmpdir_preserved':True,'meaningful_old_source_controls':6,'new_storage_tests':7,'affected_workflow_tests':13,'existing_f09_tests':2,'normal_closed_restore_tests':1,'owned_group':{k:final[k] for k in ['owned_pgid','leader_reaped','group_release']},'fixture_copies':len(fixture_index),'evidence':evidence,'parent_blockers':'raw60, actual joined writers/resources,937/938 composition/manual messaging,2 full Store exclusions,CI/owner-copy/installed cutover','live_or_whole_release_approval':False}
(report/'storage-freeze.json').write_text(json.dumps(record,indent=2)+'\n')
print(json.dumps({k:record[k] for k in ['baseline_evidence_verified','unchanged_baseline_products','fixture_copies','meaningful_old_source_controls','owned_group','canonical_queue']}))
