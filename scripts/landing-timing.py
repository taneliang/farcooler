#!/usr/bin/env python3
"""Approximate landing timings from terminal workflow receipts, without changing them."""
import argparse
import json
import math
import re
import shutil
import sys
import tempfile
from pathlib import Path

STAGES = ('build', 'review', 'wait_join', 'fold', 'gate', 'push', 'ci')
NOTE = ("Approximate seconds: effect creation to terminal receipt mtime includes scheduling and "
        "receipt-writing delay; copied or touched files can change mtimes. Effects are grouped by item across runs; repeated effects are "
        "summed, even if they overlap. Wait to join is the gap from the last review receipt "
        "to the first group effect, matched by ticket and candidate; it includes routing and "
        "scheduling; inconsistent timestamps leave it unknown. Missing stages are unknown, not zero. Group build, review and join sum matched members; group stages are shared, not divided "
        "among tickets. Percentiles use available group-stage totals (nearest rank), including "
        "failed or incomplete landings. Gate step seconds come from the gate's rounded table.")


def read_receipts(directory):
    receipts = []
    for path in sorted(directory.glob('*.json')):
        # Auxiliary process, owner, provider and verb files are not terminal receipts.
        if path.name.count('.') != 1:
            continue
        try:
            data = json.loads(path.read_text())
            effect = data['binding']['effect']
            start = effect['created_ms'] / 1000
            end = path.stat().st_mtime
            if not isinstance(effect['kind'], dict) or not isinstance(start, (int, float)) or end < start:
                continue
            receipts.append((effect, data.get('outcome'), start, end, path))
        except (OSError, ValueError, KeyError, TypeError):
            continue
    return receipts


def gate_steps(path):
    try:
        lines = path.with_suffix('.output').read_text(errors='replace').splitlines()
    except OSError:
        return []
    steps = []
    for line in lines:
        # Pipes escaped in commands must not split the Markdown cell.
        cells = [s.strip().replace('\\|', '|') for s in re.split(r'(?<!\\)\|', line)[1:-1]]
        if len(cells) >= 4:
            try:
                seconds = float(cells[3])
                if math.isfinite(seconds) and seconds >= 0:
                    steps.append({'job': cells[0], 'command': cells[1], 'result': cells[2], 'seconds': seconds})
            except ValueError:
                pass
    return sorted(steps, key=lambda s: -s['seconds'])[:5]


def report(directory):
    groups, tickets = {}, {}
    for e, outcome, start, end, path in read_receipts(directory):
        item, run = e.get('item'), e.get('run')
        if not isinstance(item, str) or not isinstance(run, str):
            continue
        kind = e['kind'].get('Execute')
        if isinstance(kind, dict) and 'Review' in kind:
            kind = 'Review'
        is_group = item.startswith('group-')
        rows = groups if is_group else tickets
        key = item
        row = rows.setdefault(key, {'item': item, 'stages': {}, 'outcomes': [], '_effects': []})
        row['_effects'].append((e, start, end))
        if outcome is not None:
            label = next(iter(outcome), 'unknown') if isinstance(outcome, dict) else str(outcome)
            row['outcomes'].append(label)
        stage = {'Build': 'build', 'Review': 'review', 'Fold': 'fold', 'Run': 'gate',
                 'Push': 'push', 'AwaitCi': 'ci'}.get(kind)
        if stage and ((is_group and stage in ('fold', 'gate', 'push', 'ci')) or
                      (not is_group and stage in ('build', 'review'))):
            row['stages'][stage] = row['stages'].get(stage, 0) + end - start
        if is_group and kind == 'Run':
            row.setdefault('gate_steps', []).extend(gate_steps(path))
    for group in groups.values():
        effects = group['_effects']
        group_start = min(x[1] for x in effects)
        members = {(m.get('item'), m.get('candidate')) for e, _, _ in effects
                   for m in e.get('members', []) if isinstance(m, dict)}
        group['members'] = sorted({m[0] for m in members if m[0]})
        group['gate_steps'] = sorted(group.get('gate_steps', []), key=lambda x: -x['seconds'])[:5]
        for item, candidate in sorted(members, key=lambda x: str(x)):
            # Candidate binds a group member to the reviewed ticket run, not just its key.
            matches = []
            for ticket in tickets.values():
                if ticket['item'] != item:
                    continue
                reviews = [(e, end) for e, _, end in ticket['_effects']
                           if isinstance(e['kind'].get('Execute'), dict)
                           and 'Review' in e['kind']['Execute']]
                if candidate and any(e.get('candidate') == candidate for e, _ in reviews):
                    matches.append((max(end for e, end in reviews if e.get('candidate') == candidate), ticket))
            if matches:
                review_end, ticket = max(matches, key=lambda x: x[0])
                ticket.setdefault('groups', []).append(group['item'])
                if review_end <= group_start:
                    ticket['stages']['wait_join'] = group_start - review_end
                for stage in ('build', 'review', 'wait_join'):
                    if stage in ticket['stages']:
                        group['stages'][stage] = group['stages'].get(stage, 0) + ticket['stages'][stage]
                for stage in ('fold', 'gate', 'push', 'ci'):
                    if stage in group['stages']:
                        ticket['stages'][stage] = group['stages'][stage]
    for row in list(groups.values()) + list(tickets.values()):
        del row['_effects']
        row['stages'] = {k: round(v, 3) for k, v in row['stages'].items()}
    percentiles = {}
    for stage in STAGES:
        source = groups.values()
        vals = sorted(r['stages'][stage] for r in source if stage in r['stages'])
        percentiles[stage] = {'n': len(vals), 'p50': vals[math.ceil(.5 * len(vals)) - 1] if vals else None,
                              'p95': vals[math.ceil(.95 * len(vals)) - 1] if vals else None}
    return {'note': NOTE, 'tickets': sorted(tickets.values(), key=lambda x: x['item']),
            'groups': sorted(groups.values(), key=lambda x: x['item']), 'percentiles': percentiles}


def table(data):
    print(data['note'])
    for name in ('tickets', 'groups'):
        print('\n' + name.title())
        print('| Item | ' + ' | '.join(STAGES) + ' | Outcomes |')
        print('| ' + ' | '.join(['---'] * (len(STAGES) + 2)) + ' |')
        for row in data[name]:
            values = [str(round(row['stages'][s], 1)) if s in row['stages'] else '—' for s in STAGES]
            print('| ' + ' | '.join([row['item']] + values + [', '.join(row['outcomes'])]) + ' |')
            for step in row.get('gate_steps', []):
                print(f"  - {step['seconds']:g}s {step['job']}: {step['command']} ({step['result']})")
    print('\nPercentiles (seconds; nearest rank)')
    print('| Stage | n | p50 | p95 |\n| --- | --- | --- | --- |')
    for stage, p in data['percentiles'].items():
        print(f"| {stage} | {p['n']} | {p['p50'] if p['p50'] is not None else '—'} | {p['p95'] if p['p95'] is not None else '—'} |")


def self_test():
    fixture = Path(__file__).parent / 'fixtures' / 'landing-timing'
    import os
    with tempfile.TemporaryDirectory() as tmp:
        dest = Path(tmp)
        for path in fixture.iterdir():
            if path.name != 'mtimes.json':
                shutil.copyfile(path, dest / path.name)
        for name, ns in json.loads((fixture / 'mtimes.json').read_text()).items():
            os.utime(dest / name, ns=(ns, ns))
        data = report(dest)
        group = next(g for g in data['groups'] if g['item'] == 'group-00000000000000000115-landing')
        assert 'Rejected' in group['outcomes'] and group['stages']['gate'] > 0
        assert group['gate_steps'] and group['gate_steps'][0]['seconds'] > 0
        assert any('Stopped' in t['outcomes'] for t in data['tickets'])
        assert group['members'] == ['ov-1024'] and 'review' in group['stages']
        assert any(t.get('groups') for t in data['tickets'])
        assert data['percentiles']['gate']['n'] == 1
        json.dumps(data)
        # A consistent pair demonstrates the candidate join and nearest-rank stats.
        synthetic = dest / 'synthetic'
        synthetic.mkdir()
        def receipt(name, item, kind, start, end, candidate=None, members=None):
            effect = {'item': item, 'run': name, 'created_ms': start * 1000,
                      'kind': {'Execute': kind}, 'candidate': candidate}
            if members is not None:
                effect['members'] = members
            path = synthetic / (name + '.json')
            path.write_text(json.dumps({'binding': {'effect': effect}, 'outcome': 'Passed'}))
            os.utime(path, (end, end))
        receipt('review', 'ticket-1', {'Review': 'Agent'}, 100, 120, 'abc')
        receipt('fold', 'group-1', 'Fold', 150, 160, members=[{'item': 'ticket-1', 'candidate': 'abc'}])
        result = report(synthetic)
        assert result['tickets'][0]['stages']['wait_join'] == 30
        assert result['groups'][0]['stages']['review'] == 20
        assert result['percentiles']['fold'] == {'n': 1, 'p50': 10.0, 'p95': 10.0}
    print('landing-timing self-test: ok')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('receipts_dir', nargs='?', type=Path)
    parser.add_argument('--json', action='store_true')
    parser.add_argument('--self-test', action='store_true')
    args = parser.parse_args()
    if args.self_test:
        self_test()
    elif not args.receipts_dir or not args.receipts_dir.is_dir():
        parser.error('a receipts directory is required')
    else:
        data = report(args.receipts_dir)
        if args.json:
            json.dump(data, sys.stdout, indent=2)
            print()
        else:
            table(data)


if __name__ == '__main__':
    main()
