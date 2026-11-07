#!/usr/bin/python3
"""Meaningful isolated red controls; save originals/mutants and restore exact bytes."""
import hashlib,json,signal,subprocess,sys
from pathlib import Path
root=Path(__file__).resolve().parents[2]
report=root/'reports/ov-939'
source=root/'crates/workflow/src/reducer.rs'
original=source.read_bytes();text=original.decode()
folder=report/'negative-sources';folder.mkdir(exist_ok=True)
(folder/'reducer.original.txt').write_bytes(original)
def interrupted(sig,_):raise SystemExit(128+sig)
for sig in [signal.SIGTERM,signal.SIGINT,signal.SIGHUP]:signal.signal(sig,interrupted)
cases=[
 ('reason','if *passed || !valid_rework_reason(reason)','if *passed','workflow_reasoned_rework_bounds_budget_and_legacy_bytes'),
 ('authority','decision(input, p)?;','let _ = p;','workflow_reasoned_rework_fences_and_exact_retention'),
 ('evidence','if item.last_evidence.as_ref() != Some(evidence) { return Err(Refusal::Evidence); }','','workflow_reasoned_rework_fences_and_exact_retention'),
 ('budget','if item.iteration >= item.definition.max_reworks { return Err(Refusal::Limit); }','','workflow_reasoned_rework_fences_and_exact_retention'),
 ('revision','|| input.expected_revision != state.revision','|| false','workflow_reasoned_rework_fences_and_exact_retention'),
 ('duplicate','if state.seen.contains(&input.id) { return Err(Refusal::Duplicate); }','','workflow_reasoned_rework_fences_and_exact_retention'),
]
results=[]
try:
    for label,old,new,test in cases:
        assert text.count(old)==1,(label,text.count(old))
        mutant=text.replace(old,new,1).encode();(folder/(label+'.mutant.txt')).write_bytes(mutant)
        source.write_bytes(mutant)
        cmd=['/Users/e-liang/.cargo/bin/cargo','test','--offline','--locked','-p','farcooler-workflow','--test','workflow',test,'--','--nocapture']
        try:
            with (report/('red-'+label+'.log')).open('x') as log:
                log.write('COMMAND '+repr(cmd)+'\n');log.flush()
                result=subprocess.run(cmd,cwd=root,stdout=log,stderr=subprocess.STDOUT)
            raw=(report/('red-'+label+'.log')).read_text()
            meaningful=result.returncode==101 and 'panicked at' in raw and 'assertion' in raw and 'test result: FAILED' in raw and test in raw and 'could not compile' not in raw
            results.append(dict(label=label,command=cmd,exit=result.returncode,meaningful=meaningful,
                original_sha256=hashlib.sha256(original).hexdigest(),mutant_sha256=hashlib.sha256(mutant).hexdigest()))
        finally:
            source.write_bytes(original)
            assert source.read_bytes()==original
            (report/'negative-results.json').write_text(json.dumps(results,indent=2)+'\n')
        print('RED',label,result.returncode,'meaningful',meaningful,flush=True)
        assert meaningful,label
finally:
    source.write_bytes(original)
    assert source.read_bytes()==original
cmd=['/Users/e-liang/.cargo/bin/cargo','test','--offline','--locked','-p','farcooler-workflow','--test','workflow','workflow_reasoned','--','--nocapture']
with (report/'restored-pure.log').open('x') as log:
    log.write('COMMAND '+repr(cmd)+'\n');log.flush()
    code=subprocess.call(cmd,cwd=root,stdout=log,stderr=subprocess.STDOUT)
assert code==0
(report/'restoration.json').write_text(json.dumps({'exit':code,'source_sha256':hashlib.sha256(source.read_bytes()).hexdigest(),'matches_original':source.read_bytes()==original},indent=2)+'\n')
