#!/usr/bin/env python3
"""Read actual build-done -> WaitingJoin observations from a workflow SQLite store.

Usage: python3 scripts/workflow-review-timing.py /path/to/farcooler.db
No model stand-ins, writes, or estimated after samples. Compare cohorts only after
publishing a light-review definition and collecting completed candidates.
"""
import argparse
import json
import sqlite3
import statistics
from pathlib import Path


def measure(connection, workspace_filter=None):
    definitions = {pin: json.loads(text) for pin, text in connection.execute(
        'SELECT pin, definition FROM workflow_definitions')}
    effects = {(workspace, effect['id']): effect for workspace, text in connection.execute(
        'SELECT workspace, payload FROM workflow_outbox') for effect in [json.loads(text)]}
    builds = {}
    samples = {'before': [], 'after': []}
    for workspace, revision, text, notices in connection.execute(
            'SELECT workspace, revision, payload, notices FROM workflow_events ORDER BY workspace, revision'):
        if workspace_filter is not None and workspace != workspace_filter:
            continue
        event = json.loads(text)
        observation = event['command'].get('Observe') if isinstance(event['command'], dict) else None
        if not observation:
            continue
        effect = effects.get((workspace, observation['effect']))
        if not effect:
            continue
        key = (workspace, effect['item'])
        if effect['kind'] == {'Execute': 'Build'}:
            builds.pop(key, None)
            if 'Passed' in observation['outcome']:
                builds[key] = (event['now_ms'], observation['outcome']['Passed']['candidate'], effect['run'])
        if f"{effect['item']}:WaitingJoin" not in json.loads(notices) or key not in builds:
            continue
        start, candidate, build_run = builds.pop(key)
        if effect.get('candidate') != candidate:
            continue
        duration = event['now_ms'] - start
        if duration < 0:
            raise ValueError(f'nonmonotonic timing at {workspace}:{revision}')
        definition = definitions[effect['definition_pin']]
        parallel = any(s['kind'] == {'Review': 'Agent'} and len(s['operation']['arguments']) > 2
                       and json.loads(s['operation']['arguments'][2]).get('check') is not None
                       for s in definition['stages'])
        light = any(s['kind'] == 'Route' and 'under_lines' in json.loads(s['operation']['arguments'][1])
                    for s in definition['stages'])
        cohort = 'after' if parallel and light else 'before'
        samples[cohort].append(dict(item=effect['item'], build_run=build_run, join_run=effect['run'],
                                    definition=effect['definition_pin'], revision=revision, ms=duration))
    return {name: dict(count=len(rows), median_ms=statistics.median(r['ms'] for r in rows) if rows else None,
                       samples=rows) for name, rows in samples.items()}


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('database', type=Path)
    parser.add_argument('--workspace', help='Limit to one workspace, excluding historical snapshots')
    args = parser.parse_args()
    with sqlite3.connect(args.database.resolve().as_uri() + '?mode=ro', uri=True) as connection:
        connection.execute('BEGIN')  # One consistent snapshot while the runner continues working.
        print(json.dumps(measure(connection, args.workspace), indent=2))
