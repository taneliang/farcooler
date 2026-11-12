#!/usr/bin/env python3
"""Render local observations from ov-1110's resumed capture."""
import json
import collections
import pathlib
import re
import zipfile

root = pathlib.Path(__file__).resolve().parent / 'resume-local'
receipt = json.loads((root / 'results.json').read_text())
with zipfile.ZipFile(root / 'logs.zip') as archive:
    log = archive.read('nextest.log').decode()
    gate_log = archive.read('landing-gate.log').decode()
tests = {}
for status, seconds, index, total, name in re.findall(
        r'^\s*(PASS|FAIL|TIMEOUT) \[\s*([\d.]+)s\] \(\s*(\d+)/(\d+)\) (.+)$', log, re.M):
    assert int(total) == 6928
    tests[int(index)] = {'status': status, 'seconds': float(seconds), 'test': name}
assert len(tests) == 6928
assert sum(row['status'] == 'FAIL' for row in tests.values()) == 12
top = sorted(tests.values(), key=lambda row: (-row['seconds'], row['test']))[:20]
lines = ['# Resumed local measurements', '',
         'Captured 13 November 2026 (Singapore). These are observations of the existing source,',
         'including its failures; they do not establish a passing gate.', '',
         '| Command | Outer wall seconds | Exit | Under-20 controlled |',
         '| --- | ---: | ---: | --- |']
for run in receipt['runs']:
    if 'exit' in run:
        lines.append(f"| {run['label']} | {run['seconds']:.2f} | {run['exit']} | {run['controlled']} |")
lines += ['', 'The outer timer polls every two seconds, so successful-command wall times',
          'include up to two seconds of observation latency. Cargo reports 0.31 s for',
          'the warm build. Nextest reports 1m47s compilation and 700.841 s execution:',
          '6,916 passed, 12 failed, 33 skipped. The wrappers are included in outer wall time.',
          'All 6,928 executed test results are present; repeated failure-summary rows are deduplicated by test index.', '',
          '## Landing gate: incomplete due to load', '']
gate = receipt['runs'][-1]
assert gate['label'] == 'landing-gate' and gate['exit'] == 130 and not gate['controlled']
blocker = gate['blocker']
lines += [f"Stopped at {blocker['utc']}, load **" + ' / '.join(f'{n:.2f}' for n in blocker['load_1_5_15']) + '** (1/5/15 minutes).',
          'Only the capture-created process group was interrupted. No full-gate latency or passing receipt was obtained.', '',
          'Completed-step seconds below are rounded by the gate, exclude orchestration, and are not total gate latency.', '',
          '| Job | Recorded completed-step seconds | Coverage |', '| --- | ---: | --- |']
job_seconds = collections.defaultdict(int)
for job, seconds in re.findall(r'^\| ([\w-]+) \| .*? \| pass \| (\d+) \|', gate_log, re.M):
    job_seconds[job] += int(seconds)
for job in ['wire', 'doc-comments', 'relay', 'tunnel', 'rust', 'swift-tests', 'swift', 'ios', 'android', 'web']:
    coverage = 'completed' if job in ['wire', 'doc-comments', 'relay', 'tunnel'] else ('partial; interrupted in cross-target cargo check' if job == 'rust' else 'not reached')
    lines.append(f"| {job} | {job_seconds[job] if job in job_seconds else 'unmeasured'} | {coverage} |")
lines += ['', '## Fresh cold build: longest compilation units', '',
          'Unit times overlap; frontend/codegen/link interpretation remains in the main investigation.', '',
          '| Unit | Seconds |', '| --- | ---: |']
with zipfile.ZipFile(root / 'cold-timing.zip') as archive:
    timing = archive.read('cold-timing.html').decode()
units = json.loads(re.search(r'const UNIT_DATA = (\[.*?\]);', timing, re.S)[1])
for unit in sorted(units, key=lambda row: -row['duration'])[:10]:
    lines.append(f"| {unit['name']} {unit['version']}{unit.get('target', '')} | {unit['duration']:.2f} |")
lines += ['',
          '## Slowest 20 local Rust tests', '',
          '| Test | Seconds | Result |', '| --- | ---: | --- |']
for row in top:
    lines.append(f"| `{row['test']}` | {row['seconds']:.3f} | {row['status']} |")
lines += ['', '## Failed tests', '']
for row in sorted(tests.values(), key=lambda row: row['test']):
    if row['status'] != 'PASS':
        lines.append(f"- `{row['test']}` ({row['seconds']:.3f} s)")
print('\n'.join(lines))
