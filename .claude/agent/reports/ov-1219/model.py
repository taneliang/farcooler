#!/usr/bin/env python3
"""Explicit counterfactual, not a benchmark of nonexistent crates."""
import collections
import json
from pathlib import Path
import re
import zipfile
import analyze

OUT, ROOT = analyze.OUT, analyze.ROOT
PREFIX = 'farcooler-daemon-'
GROUPS = ['git','access','agents','engine','board','service','watch','delivery','shell','integration']

def group(path):
    g = analyze.owner(path)
    return 'shell' if g == 'rpc' else g


def main():
    crates = json.loads((OUT/'crates.json').read_text())
    modules = json.loads((OUT/'modules.json').read_text())
    baseline = ROOT/'reports/ov-1110/resume-local'
    with zipfile.ZipFile(baseline/'cold-timing.zip') as z:
        units = json.loads(re.search(r'const UNIT_DATA = (\[.*?\]);',z.read(z.namelist()[0]).decode(),re.S)[1])
    build = collections.Counter()
    for u in units:
        if u['name'].startswith('farcooler-'):
            build[u['name']] += u['duration']
    # Re-use the repository's measured build artifacts; never interpret sums as build wall.
    (OUT/'compile-units.json').write_text(json.dumps(units,indent=2)+'\n')
    receipt = json.loads((baseline/'results.json').read_text())
    total_lines = sum(x['lines'] for x in modules if '/src/' in x['path'])
    lines = collections.Counter()
    for row in modules:
        if '/src/' in row['path']:
            lines[group(row['path'])] += row['lines']
    # Desired acyclic implementation graph. RPC and composition remain in farcooler-daemon.
    deps = {c['name']:set(c['dependencies']) for c in crates}
    deps['farcooler-daemon'].discard('farcooler-cli') # modeled below as an explicit runtime edge
    deps['farcooler-daemon'].discard('farcooler-client') # move daemon client-using integration tests
    deps['farcooler-daemon'].discard('farcooler-vt')
    for g in GROUPS:
        if g not in ('shell','integration'):
            deps[PREFIX+g] = {'farcooler-daemon-contracts','farcooler-daemon-context','farcooler-core',
                              'farcooler-protocol','farcooler-store','farcooler-tmux'}
            deps['farcooler-daemon'].add(PREFIX+g)
    for g in ['access','agents','engine','board','service']:
        deps[PREFIX+g].add(PREFIX+'git')
    deps[PREFIX+'delivery'].add(PREFIX+'agents')
    deps[PREFIX+'watch'].update({PREFIX+'agents',PREFIX+'delivery',PREFIX+'board'})
    deps[PREFIX+'service'].update({PREFIX+'agents',PREFIX+'access'})
    deps[PREFIX+'context'] = {'farcooler-core','farcooler-protocol','farcooler-store','farcooler-tmux',PREFIX+'contracts'}
    deps[PREFIX+'contracts'] = {'farcooler-core','farcooler-protocol','farcooler-workflow'}
    deps[PREFIX+'engine'].add('farcooler-workflow')
    deps[PREFIX+'board'].add('farcooler-review')
    deps[PREFIX+'integration'] = {'farcooler-daemon','farcooler-client','farcooler-cli','farcooler-vt','farcooler-store'}
    # CLI helper migration removes its shipping daemon edge; client real-daemon dev edge stays.
    deps['farcooler-cli'].discard('farcooler-daemon')
    deps['farcooler-cli'].update({PREFIX+'contracts',PREFIX+'context',PREFIX+'engine',PREFIX+'service'})
    # Conservative: current CLI direct execution adapters still reach engine/service until a later thinning.
    deps['farcooler-daemon'].add('farcooler-cli') # selector's runtime consumer edge
    (OUT/'proposal.json').write_text(json.dumps({k:sorted(v) for k,v in sorted(deps.items())},indent=2)+'\n')
    # Feed counterfactual packages/path ownership through the actual selector select(), not a new selector.
    virtual = Path('/tmp/ov1219-counterfactual')
    packages = []
    for name, dependencies in deps.items():
        directory = 'daemon' if name == 'farcooler-daemon' else name.removeprefix('farcooler-')
        packages.append({'id': name,'name':name,'manifest_path':str(virtual/'crates'/directory/'Cargo.toml'),
            'targets':[], 'dependencies':[{'name':d,'path':str(virtual/'crates'/d)} for d in dependencies]})
    metadata = {'workspace_members':list(deps), 'packages':packages,
        'resolve':{'nodes':[{'id':n,'deps':[{'pkg':d} for d in ds]} for n,ds in deps.items()]}}
    # Historical test durations: deduplicate failure-summary repeats by nextest index.
    with zipfile.ZipFile(baseline/'logs.zip') as z:
        log = z.read('nextest.log').decode()
    tests = {}
    for status,sec,index,total,name in re.findall(r'^\s*(PASS|FAIL|TIMEOUT|SIGINT) \[\s*([\d.]+)s\] \(\s*(\d+)/(\d+)\) (.+)$',log,re.M):
        package, test = name.split(' ',1)
        old_package = package.split('::')[0]
        new_package = old_package
        if old_package == 'farcooler-daemon':
            if '::' in package:
                new_package = PREFIX+'integration'
            else:
                test_path = 'crates/daemon/src/' + test.split('::')[0] + '.rs'
                # watch submodules are a distinct ownership seam.
                if test.startswith('watch::'):
                    test_path = 'crates/daemon/src/watch/' + test.split('::')[1] + '.rs'
                g = group(test_path)
                new_package = 'farcooler-daemon' if g == 'shell' else PREFIX+g
        tests[int(index)] = {'old':old_package,'new':new_package,'seconds':float(sec),'status':status}
    assert len(tests)==6928
    measured_test_wall = 700.841
    total_work = sum(t['seconds'] for t in tests.values())
    old_work, new_work = collections.Counter(), collections.Counter()
    old_count, new_count = collections.Counter(), collections.Counter()
    for t in tests.values():
        old_work[t['old']] += t['seconds']; new_work[t['new']] += t['seconds']
        old_count[t['old']] += 1; new_count[t['new']] += 1
    test_rows = [{'old_package':k,'count':old_count[k],'observed_case_seconds':old_work[k]} for k in old_count]
    (OUT/'test-weights.json').write_text(json.dumps({'source':receipt['head'],'count':len(tests),
        'failures':sum(t['status']!='PASS' for t in tests.values()), 'wall_seconds':measured_test_wall,
        'sum_case_seconds':total_work,'old':test_rows,'new':[{'package':k,'count':new_count[k],
        'observed_case_seconds':new_work[k]} for k in sorted(new_count)]},indent=2)+'\n')
    # Build approximation: warm external deps; full current library compiler invocation,
    # plus domain share and 1 second/new package scheduling/metadata allowance. Serial sum.
    # Test-build baseline is measured 107s; weight by observed selected case work.
    daemon_lib = sum(u['duration'] for u in units if u['name']=='farcooler-daemon' and not u['target'])
    daemon_bins = build['farcooler-daemon'] - daemon_lib
    new_build = {n:v for n,v in build.items() if n!='farcooler-daemon'}
    new_build[PREFIX+'contracts'] = 1
    new_build[PREFIX+'context'] = 1
    new_build['farcooler-daemon'] = daemon_lib*lines['shell']/total_lines + daemon_bins + 1
    for g in GROUPS:
        if g not in ['shell','integration']:
            new_build[PREFIX+g] = daemon_lib*lines[g]/total_lines+1
    rows = []
    shadow = analyze.load_selector()
    for selection in json.loads((OUT/'selections.json').read_text()):
        changes=[]
        for status,path in selection['changes']:
            if path.startswith('crates/daemon/'):
                g=group(path)
                if g!='shell':
                    path=path.replace('crates/daemon/',f'crates/daemon-{g}/',1)
            changes.append((status,path))
        selected, reasons, full = shadow.select(virtual,metadata,changes)
        before=selection['before']
        before_work=sum(old_work[k] for k in before)
        after_work=sum(new_work[k] for k in selected)
        before_b=sum(build[k] for k in before)
        after_b=sum(new_build.get(k,0) for k in selected)
        before_t=measured_test_wall*before_work/total_work
        after_t=measured_test_wall*after_work/total_work
        # Estimated test harness compilation apportioned by case work, in addition to product build.
        before_h=107*before_work/total_work;after_h=107*after_work/total_work
        rows.append({'sha':selection['sha'],'after':selected,'after_full':full,'after_reasons':reasons,
            'before_cases':sum(old_count[k] for k in before),'after_cases':sum(new_count[k] for k in selected),
            'before_build_seconds':before_b+before_h,'after_build_seconds':after_b+after_h,
            'before_test_seconds':before_t,'after_test_seconds':after_t,
            'before_minutes':(before_b+before_h+before_t)/60,
            'after_minutes':(after_b+after_h+after_t)/60})
    (OUT/'estimates.json').write_text(json.dumps(rows,indent=2)+'\n')
    for row in rows:
        print(row['sha'][:9],row['before_cases'],row['after_cases'],round(row['before_minutes'],2),round(row['after_minutes'],2),row['after'])
    print('compile',json.dumps(dict(build),indent=2))

if __name__=='__main__':
    main()
