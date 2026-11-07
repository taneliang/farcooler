#!/usr/bin/python3
"""Bind the bounded source delta, resumed gates and retained reports before signing."""
from pathlib import Path
import difflib, hashlib, json, subprocess
root=Path(__file__).resolve().parents[2]
def git(*args): return subprocess.check_output(['git',*args],cwd=root)
def sha(raw): return hashlib.sha256(raw).hexdigest()
def load(name): return json.loads((root/'reports/ov-934'/name).read_text())
parent=git('rev-parse','HEAD').decode().strip()
assert parent=='e69ec2bbad90561f33cb707e374495b0b949772d'
paths=set(git('diff','--name-only',parent,'--','crates').decode().splitlines())
paths.update(git('ls-files','--others','--exclude-standard','--','crates').decode().splitlines())
assert all(p.startswith(('crates/daemon/','crates/store/')) for p in paths)
source={}
for name in sorted(paths):
 raw=(root/name).read_bytes()
 source[name]={'sha256':sha(raw),'blob':git('hash-object',name).decode().strip()}
source_digest=sha(json.dumps(source,sort_keys=True,separators=(',',':')).encode())
phases={}
for p in sorted((root/'reports/ov-934').glob('*.json')):
 d=json.loads(p.read_text())
 if 'command' in d and 'source_unchanged' in d:
  assert d['source_unchanged']
  phases[p.name]={k:d[k] for k in ['exit','stopped','seconds','minimum_free','source_unchanged','command']}
for name in ['affected-tail.json','lint.json']:
 d=load(name);assert d['exit']==0 and not d['stopped'] and d['source_unchanged']
 for path,digest in d['after'].items(): assert sha((root/path).read_bytes())==digest,(name,path)
restored=load('affected-restored.json')
delta=[p for p,h in restored['after'].items() if sha((root/p).read_bytes())!=h]
assert delta==['crates/store/src/workflow_publication.rs'],delta
path=delta[0]; current=(root/path).read_text()
old=current.replace('assert!(serde_json::from_str::<serde_json::Value>(&old).unwrap()[1].get("adapter").is_none());','assert!(!old.contains("adapter"));')
assert sha(old.encode())==restored['after'][path]
assert old.split('#[cfg(test)]')[0]==current.split('#[cfg(test)]')[0]
patch=''.join(difflib.unified_diff(old.splitlines(keepends=True),current.splitlines(keepends=True),fromfile='daemon-gate/'+path,tofile='final/'+path))
(root/'reports/ov-934/post-daemon-test-assertion.patch').write_text(patch)
controls={}
for kind in ['alias','safety','recovery','availability']:
 d=load('mutation-'+kind+'.json');assert d['behavioral_red'] and d['exact_restoration']
 controls[kind]=d
original=load('original931-seal.json')
for path,h in original['files'].items():assert sha((root/path).read_bytes())==h,path
assert (root/'reports/ov-934/product-diff-check.log').read_bytes()==b''
assert 'file-size-budget: ok' in (root/'reports/ov-934/size-budget.log').read_text()
inputs={p:git('rev-parse',p+'^{tree}').decode().strip() for p in [parent,'a39b32d21f1df4db176f868ca65c333c5e9b0027','c40197486572a584f8c85ec2dde423587e0560e6','4e01f8be46e28603a02e7464a984828ab07e54a4','61ea01422a981797060d74c730c91978345fcca7']}
report='/Users/e-liang/Dev/overnight/.claude/agent/reports/ov-931/independent-review.md'
manifest={'parent':parent,'input_trees':inputs,'source_delta':source,'source_delta_sha256':source_digest,
 'signed_pin_transport':'signed commit containing this manifest; exact pin/tree/signature recorded on task ov-934',
 'review_input':{'path':report,'sha256':sha(Path(report).read_bytes())},
 'composition_manifest_sha256':sha((root/'reports/ov-931/composition.json').read_bytes()),
 'later_original931_source_reconciliation':'sole e69→4e01 product delta is equivalent Option::map; applied directly, reports-only61ea imported exactly',
 'affected_distinct_passes':{'daemon_lib':62,'protocol_lib':1,'store_lib':22,'cli_bin':2,'ordinary_service_socket_process_shutdown':10,'command_socket':2,'total':99},
 'resumed_gate_binding':{'daemon_protocol_record':'affected-restored.json','final_tail_record':'affected-tail.json',
  'only_later_source_delta':delta,'delta_patch_sha256':sha(patch.encode()),'non_test_store_prefix_unchanged':True,
  'final_tail_and_lint_all_source_hashes_equal_final':True},
 'negative_controls':controls,'phases':phases,'original931_seal':original,
 'limits':['private bounded R1/R2 proof only','SAME931 final source/evidence audit pending','no actual main executable/owner data/whole release/activation/provider network proof','prior parallel FIFO timeout retained; serial fixtures do not change the read bound']}
manifest['evidence_sha256']={str(p.relative_to(root)):sha(p.read_bytes()) for p in sorted((root/'reports/ov-934').iterdir()) if p.is_file() and p.name!='manifest.json'}
(root/'reports/ov-934/manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
print(json.dumps({'source_files':len(source),'source_delta_sha256':source_digest,'distinct_affected_passes':99,'all_controls_exactly_restored':True}))
