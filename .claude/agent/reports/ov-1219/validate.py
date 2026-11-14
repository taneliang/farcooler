#!/usr/bin/env python3
"""Validate measured provenance, exhaustive ownership and conservative consumers."""
import collections
import json
from pathlib import Path
import re
import subprocess
import zipfile
import analyze

OUT, ROOT = analyze.OUT, analyze.ROOT

def read(name):
    return json.loads((OUT/name).read_text())

def main():
    baseline = ROOT/'reports/ov-1110/resume-local'
    receipt = json.loads((baseline/'results.json').read_text())
    assert receipt['head']=='8a7ce8759c277bfe99bc6b5b30015c8a8b16f290'
    main_source='d986f3083730db7cda3b91caf5e536ba321b7c1e'
    subprocess.run(['git','merge-base','--is-ancestor',main_source,analyze.SOURCE],cwd=ROOT,check=True)
    changed=analyze.git('diff','--name-only',main_source,receipt['head']).splitlines()
    assert changed and all(path.startswith('reports/ov-1110/') for path in changed)
    for path in ['crates','Cargo.toml','Cargo.lock','.cargo','.config','scripts','test']:
        assert analyze.git('rev-parse',main_source+':'+path)==analyze.git('rev-parse',receipt['head']+':'+path)
    assert receipt['cold_target_existed'] is False
    for run in receipt['runs'][:2]:
        assert run['exit']==0 and run['controlled']
        assert max(max(s['load_1_5_15']) for s in run['samples']+[run['end']])<20
        assert run['command']==['cargo','build','--workspace','--locked','--offline','--timings']
    with zipfile.ZipFile(baseline/'cold-timing.zip') as z:
        units=json.loads(re.search(r'const UNIT_DATA = (\[.*?\]);',z.read(z.namelist()[0]).decode(),re.S)[1])
    assert units==read('compile-units.json')
    crates=read('crates.json')
    assert len(crates)==18
    daemon=next(c for c in crates if c['name']=='farcooler-daemon')
    assert len(daemon['normal_dependencies'])==12 and len(daemon['dependencies'])==15
    assert set(daemon['transitive_consumers'])=={'farcooler-cli','farcooler-client','farcooler-jni'}
    files=read('modules.json')
    actual={p.relative_to(ROOT).as_posix() for p in (ROOT/'crates/daemon').rglob('*.rs')}
    assert {f['path'] for f in files}==actual and len(files)==len(actual)
    assert all(len((ROOT/f['path']).read_text().splitlines())==f['lines'] for f in files)
    assert sum(f['lines'] for f in files)==195098
    assert sum(f['lines'] for f in files if '/src/' in f['path'])==167406
    assert sum(f['lines'] for f in files if '/tests/' in f['path'])==27623
    counts=collections.Counter()
    for f in files:
        g='shell' if f['group']=='rpc' else f['group']
        counts[g]+=f['lines']
    assert len(counts)==10 and all(10000<=n<=30000 for n in counts.values())
    weights=read('test-weights.json')
    assert weights['count']==6928 and weights['failures']==12
    assert sum(t['count'] for t in weights['old'])==sum(t['count'] for t in weights['new'])==6928
    assert abs(sum(t['observed_case_seconds'] for t in weights['old'])-2091.409)<.001
    scenarios=read('selections.json'); estimates=read('estimates.json')
    assert len(scenarios)==len(estimates)==5
    required={'farcooler-cli','farcooler-client','farcooler-daemon','farcooler-jni'}
    for before,after in zip(scenarios,estimates):
        assert before['sha']==after['sha']
        assert required<=set(before['before'])
        assert required|{'farcooler-daemon-integration'}<=set(after['after'])
        assert after['after_cases']<=after['before_cases']
        for side in ('before','after'):
            assert abs(after[side+'_minutes']*60-after[side+'_build_seconds']-after[side+'_test_seconds'])<1e-6
    assert scenarios[-1]['full'] and estimates[-1]['after_full']
    assert scenarios[-1]['reasons']==['full: M docs/workspaces.md']
    assert all(e['before_cases']==e['after_cases'] for e in estimates[2:])
    proposal=read('proposal.json')
    for name in ['farcooler-daemon-contracts','farcooler-daemon-context']:
        assert not any(d.startswith('farcooler-daemon') and d not in
                       ['farcooler-daemon-contracts','farcooler-daemon-context'] for d in proposal[name])
    # These edges are essential to retaining real consumer failures, not optional performance knobs.
    assert 'farcooler-daemon' in proposal['farcooler-client']
    assert 'farcooler-cli' in proposal['farcooler-daemon']
    assert 'farcooler-daemon' in proposal['farcooler-daemon-integration']
    for p in OUT.glob('*.md'):
        for target in re.findall(r'\]\(([^)]+)\)',p.read_text()):
            assert (p.parent/target.split('#')[0]).exists(), (p,target)
    print('PASS: verified main timing/load provenance; all 18 crates and 195,098 daemon lines;')
    print('6,928 historical cases/12 failures retained; 5 real selector scenarios; consumer edges; links.')

if __name__=='__main__':
    main()
