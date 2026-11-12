#!/usr/bin/env python3
"""Recompute ov-1110's tables from captured public job receipts and timing HTML."""
import collections
import datetime
import json
import pathlib
import re
import statistics
import zipfile

ROOT = pathlib.Path(__file__).resolve().parent

def timestamp(value):
    return datetime.datetime.fromisoformat(value.replace('Z', '+00:00'))

def tables():
    jobs = collections.defaultdict(list)
    for path in sorted(ROOT.glob('ci-*.json')):
        if not re.fullmatch(r'ci-\d+\.json', path.name):
            continue
        run = json.loads(path.read_text())
        for job in run['jobs']:
            if job['conclusion'] == 'success':
                duration = (timestamp(job['completedAt']) - timestamp(job['startedAt'])).total_seconds()
                assert duration >= 0
                jobs[job['name']].append(duration)
    assert len(jobs) == 14, 'missing CI job family'
    lines = ['| CI job | Runs | Median seconds | Min–max seconds |', '| --- | ---: | ---: | ---: |']
    for name, durations in sorted(jobs.items()):
        lines.append(f'| {name} | {len(durations)} | {statistics.median(durations):.1f} | {min(durations):.0f}–{max(durations):.0f} |')
    gate = collections.defaultdict(int)
    for line in (ROOT / 'historical-gate.txt').read_text().splitlines():
        cells = [cell.strip() for cell in line.split('|')]
        if len(cells) == 7 and cells[4].isdigit():
            gate[cells[1]] += int(cells[4])
    assert sum(gate.values()) == 2806
    lines += ['', '| Historical local job | Sum of step seconds |', '| --- | ---: |']
    lines += [f'| {name} | {seconds} |' for name, seconds in gate.items()]
    lines += [f'| Total executed steps | {sum(gate.values())} |', '']
    units = json.loads(re.search(r'const UNIT_DATA = (\[.*?\]);', zipfile.ZipFile(ROOT / 'timings.zip').read('cold-timing.html').decode(), re.S)[1])
    assert len(units) > 400
    lines += ['| Cold compilation unit | Seconds | Frontend / codegen seconds, when exposed |', '| --- | ---: | --- |']
    for unit in sorted(units, key=lambda row: row['duration'], reverse=True)[:15]:
        parts = '; '.join(f"{name}: {values['end']-values['start']:.2f}" for name, values in unit['sections'] or [])
        lines.append(f"| {unit['name']} {unit['version']}{unit['target']} | {unit['duration']:.2f} | {parts or 'not separately exposed'} |")
    tests = json.loads((ROOT / 'ci-test-times.json').read_text())
    for field, title in [('top20_observed', 'Slowest 20 observed individual tests (Swift and Rust)'), ('top20_rust', 'Slowest 20 Rust test observations')]:
        rows = tests[field]
        assert len(rows) == 20 and rows == sorted(rows, key=lambda row: row['seconds'], reverse=True)
        lines += ['', f'### {title}', '', '| Job | Seconds | Test |', '| --- | ---: | --- |']
        lines += [f"| {row['job']} | {row['seconds']:.3f} | {row['test'].replace('|', '/')} |" for row in rows]
    return '\n'.join(lines) + '\n'

if __name__ == '__main__':
    print(tables(), end='')
