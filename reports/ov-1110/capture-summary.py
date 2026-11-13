#!/usr/bin/env python3
"""Render a retry's measured outcomes, including incomplete/load-rejected runs."""
import collections
import json
import pathlib
import re
import sys
import zipfile

root = pathlib.Path(sys.argv[1])
receipt = json.loads((root / 'results.json').read_text())
with zipfile.ZipFile(root / 'logs.zip') as archive:
    logs = {name[:-4]: archive.read(name).decode() for name in archive.namelist()}
lines = ['# Controlled baseline retry', '', f"Source: `{receipt['head']}`.", '',
         'Outer wall times include up to two seconds of polling latency.',
         'A load-interrupted command is excluded from controlled baseline comparisons.', '',
         '| Command | Outer wall seconds | Exit | Controlled below 20 |',
         '| --- | ---: | ---: | --- |']
for run in receipt['runs']:
    assert 'exit' in run, 'capture still running'
    assert max(run['preflight'][-1]['load_1_5_15']) < receipt['limit']
    samples = run['samples'] + [run['end']]
    assert run['controlled'] == ('blocker' not in run and max(run['end']['load_1_5_15']) < 20)
    if run['controlled']:
        assert all(max(s['load_1_5_15']) < 20 for s in samples)
    lines.append(f"| {run['label']} | {run['seconds']:.2f} | {run['exit']} | {run['controlled']} |")
    if 'blocker' in run:
        assert run['blocker'] == run['samples'][-1]
        assert max(run['blocker']['load_1_5_15']) >= 20
        lines += ['', f"Stopped `{run['label']}` at {run['blocker']['utc']}, load **" +
                  ' / '.join(f'{n:.2f}' for n in run['blocker']['load_1_5_15']) + '**.',
                  'Only the newly created capture process group was signaled.', '']
for label in ['cold', 'warm', 'nextest']:
    if label in logs:
        summaries = re.findall(r'^.*(?:Finished `.*|Summary \[.*).+$', logs[label], re.M)
        lines += ['', f"{label}: " + '; '.join(s.strip() for s in summaries)]
for label in ['cold', 'warm']:
    path = root / (label + '-timing.zip')
    if not path.exists():
        continue
    with zipfile.ZipFile(path) as archive:
        html = archive.read(label + '-timing.html').decode()
    units = json.loads(re.search(r'const UNIT_DATA = (\[.*?\]);', html, re.S)[1])
    lines += ['', f'## {label.title()} build compilation units', '',
              'Durations overlap; final compiler invocations include linking.', '',
              '| Unit | Seconds |', '| --- | ---: |']
    for unit in sorted(units, key=lambda u: -u['duration'])[:10]:
        lines.append(f"| {unit['name']} {unit['version']}{unit.get('target', '')} | {unit['duration']:.2f} |")
if 'nextest' in logs:
    tests = {}
    totals = set()
    for status, seconds, index, total, name in re.findall(
            r'^\s*(PASS|FAIL|TIMEOUT|SIGINT) \[\s*([\d.]+)s\] \(\s*(\d+)/(\d+)\) (.+)$', logs['nextest'], re.M):
        totals.add(int(total))
        tests[int(index)] = (float(seconds), name, status)
    assert len(totals) <= 1
    lines += ['', f"Observed {len(tests)} of {next(iter(totals), 'unknown')} test results.", '',
              '## Slowest 20 observed Rust tests', '',
              '| Test | Seconds | Result |', '| --- | ---: | --- |']
    for seconds, name, status in sorted(tests.values(), key=lambda t: (-t[0], t[1]))[:20]:
        lines.append(f'| `{name}` | {seconds:.3f} | {status} |')
    lines += ['', '## Failed or interrupted tests', '']
    lines += [f'- `{name}` ({seconds:.3f} s; {status})' for seconds, name, status in sorted(tests.values(), key=lambda t: t[1]) if status != 'PASS']
if 'landing-gate' in logs:
    lines += ['', '## Landing gate per-job executed-step wall time', '',
              'Rounded step seconds exclude queue/orchestration. Failed and skipped rows remain in the full log.', '',
              '| Job | Executed step seconds | Step outcomes |', '| --- | ---: | --- |']
    jobs = collections.defaultdict(lambda: [0, collections.Counter()])
    for job, status, seconds in re.findall(r'^\| ([\w-]+) \| .*? \| ([^|]+) \| (\d*) \|', logs['landing-gate'], re.M):
        jobs[job][0] += int(seconds or 0)
        jobs[job][1][status.strip()] += 1
    for job, (seconds, outcomes) in jobs.items():
        lines.append(f"| {job} | {seconds} | " + ', '.join(f'{n} {status}' for status, n in outcomes.items()) + ' |')
    lines += ['', 'See the complete gate log for the planned jobs and CI-only omissions.']
print('\n'.join(lines))
