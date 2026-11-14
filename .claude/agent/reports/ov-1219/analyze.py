#!/usr/bin/env python3
"""Rebuild the report inventory and run the actual ov-1116 selection function.

Static module references are candidates, not a compiler-resolved call graph.
The counterfactual graph is deliberately explicit in proposal.json.
"""
import collections
import importlib.util
import json
from pathlib import Path
import re
import subprocess

SOURCE = '73974f89898d716c36595eade2511ba5f8df7a8f'
OUT = Path(__file__).resolve().parent
ROOT = OUT.parents[3]

def git(*args):
    return subprocess.check_output(['git', *args], cwd=ROOT, text=True).strip()

def load_selector():
    spec = importlib.util.spec_from_file_location('shadow', ROOT / 'scripts/rust-shadow.py')
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod

GROUPS = {
 'git': 'git git_capped git_exclude git_guard git_launch git_lfs git_sandbox beneath change_set file_diff read_only_folders worktree_files fs_watch hostwalk repo_grep stack reconcile',
 'access': 'people person_access person_access_change caller_person caller_seat enrollment first_key step_up step_up_rpc approvals teammate_bind access_migration member_removal runner_key sessions tunnel_gate allowlist rendezvous cutover_gate cutover_admin cutover_delegate cutover_grant code_identity writer_gate writer_join sshd_version registry_binding',
 'agents': 'activity_source agent_supervisor claude_key_exports claude_login_renewal claude_login_wire claude_logins claude_sign_in claude_mailbox claude_registry codex_trust hook_asks hook_ingress hook_install log_join log_watch session_projectors session_projectors_holds session_discovery session_watch skill_install transcript_tail running_agent subagent_rows usage',
 'engine': 'workflow_runtime effect_runner workflow_service workflow_notices workflow_summary workflow_drafts workflow_starter workflow_deciders engine_moves engine_stuck ticket_moves path_routes landing_templates',
 'board': 'answered_for by_hand check_in_target claims lead_change needs_you notice_audience orchestrator people_rulings plan_glance release_sweep rep_gate reps review review_ops task_link task_ops task_starts ci_canary ci_watch pr_stage plan_stage pr_review pr_nudge pr_suggest pr_timing pr_timing_read pr_actions pr_watch landing landing_read charter_draft charter_read charter_write survey survey_ops survey_read board_reads_ops review_agent worker_seen report',
 'service': 'service runtime runtime_selection agent_panes chat_migration fanout foreground layout opening paste_mode pastes paths daemon_log proc_cwd proc_table push resync refusal shared_logins stream_end test_support web_pane workspace_home workspace_ops lfs_ops',
 'rpc': 'rpc rpc_board rpc_people rpc_agent_answer rpc_claude_sign_in rpc_rows rpc_rows_join rpc_pages rpc_plan rpc_agent_stop rpc_rulings rpc_landings wire',
}
OWNERS = {name: group for group, names in GROUPS.items() for name in names.split()}

def owner(path):
    rel = Path(path).relative_to('crates/daemon')
    if rel.parts[0] == 'tests':
        return 'integration'
    parts = rel.parts[1:]
    if parts[0] in ('watch', 'watch.rs'):
        if len(parts) > 1 and (parts[1].startswith('answer_wake') or parts[1].startswith('task_notice')):
            return 'delivery'
        return 'watch'
    stem = parts[0].removesuffix('.rs')
    # Standalone test modules follow the longest owning module name.
    candidates = [name for name in OWNERS if stem == name or stem.startswith(name + '_')]
    return OWNERS[max(candidates, key=len)] if candidates else 'shell'


def main():
    assert not git('diff', SOURCE, '--', 'crates', 'scripts/rust-shadow.py', 'Cargo.toml', 'Cargo.lock'), 'source drift; rebase the report inputs explicitly'
    metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--locked', '--offline',
                       '--format-version', '1', '--all-features'], cwd=ROOT, text=True))
    shadow = load_selector()
    packages, reverse = shadow.graph(metadata)
    names = {key: value['name'] for key, value in packages.items()}
    def closure(start):
        seen = set(start)
        todo = list(start)
        while todo:
            for consumer in reverse[todo.pop()] - seen:
                seen.add(consumer)
                todo.append(consumer)
        return seen
    crate_rows = []
    for key, p in sorted(packages.items(), key=lambda item: item[1]['name']):
        directory = Path(p['manifest_path']).parent
        src = sum(len(f.read_text().splitlines()) for f in (directory/'src').rglob('*.rs'))
        tests = sum(len(f.read_text().splitlines()) for f in (directory/'tests').rglob('*.rs'))
        crate_rows.append({'name': p['name'], 'src_lines': src, 'test_lines': tests,
            'normal_dependencies': sorted(d['name'] for d in p['dependencies'] if d['kind'] is None and d.get('path')),
            'dependencies': sorted(names[d] for d in packages if key in reverse[d]),
            'direct_consumers': sorted(names[d] for d in reverse[key]),
            'transitive_consumers': sorted(names[d] for d in closure([key]) - {key})})
    (OUT/'crates.json').write_text(json.dumps(crate_rows, indent=2)+'\n')
    files = []
    edges = collections.Counter()
    for f in sorted((ROOT/'crates/daemon').rglob('*.rs')):
        path = f.relative_to(ROOT).as_posix()
        text = f.read_text()
        group = owner(path)
        refs = set(re.findall(r'\bcrate::([a-z_][a-z_0-9]*)', text))
        for match in re.finditer(r'\buse crate::\{([^;]+)\};', text, re.S):
            refs.update(re.findall(r'(?:^|,)\s*([a-z_][a-z_0-9]*)', match[1]))
        refs -= {'tests'}
        for ref in refs:
            target = 'watch' if ref == 'watch' else OWNERS.get(ref, 'shell')
            if target != group:
                edges[group, target] += 1
        files.append({'path': path, 'group': group, 'lines': len(text.splitlines()),
                      'crate_references': sorted(refs)})
    (OUT/'modules.json').write_text(json.dumps(files, indent=2)+'\n')
    (OUT/'module-edges.json').write_text(json.dumps([{'from': a, 'to': b, 'files': n}
                    for (a,b),n in sorted(edges.items())], indent=2)+'\n')
    totals = collections.Counter()
    for row in files:
        totals[row['group']] += row['lines']
    print('GROUPS',json.dumps(dict(totals),indent=2))
    for row in crate_rows:
        print(row['name'],row['src_lines'],row['test_lines'],len(row['dependencies']),len(row['transitive_consumers']))
    commits = ['6afab149a', '3c6cb253a', '4d331fabd', '2cf9644fc', '73974f898']
    selections = []
    for commit in commits:
        raw = git('diff-tree', '--no-commit-id', '--no-renames', '--name-status', '-r', commit)
        changes = [line.split('\t', 1) for line in raw.splitlines()]
        selected, reasons, full = shadow.select(ROOT, metadata, changes)
        selections.append({'sha': git('rev-parse',commit), 'subject':git('show','-s','--format=%s',commit),
            'changes':changes,'before':selected,'full':full,'reasons':reasons,
            'daemon_groups':sorted({owner(p) for _,p in changes if p.startswith('crates/daemon/')})})
    (OUT/'selections.json').write_text(json.dumps(selections,indent=2)+'\n')
    # Keep a portable graph: no absolute checkout paths or third-party metadata blob.
    (OUT/'provenance.json').write_text(json.dumps({'sha':SOURCE,
        'origin_main':git('rev-parse','origin/main'),'local_main':git('rev-parse','main'),
        'scope':'locked all-feature, unfiltered-platform metadata; selector adds CLI -> daemon',
        'selector_blob':git('rev-parse',SOURCE+':scripts/rust-shadow.py')},indent=2)+'\n')

if __name__ == '__main__':
    main()
