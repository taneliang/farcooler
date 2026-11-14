#!/usr/bin/env python3
"""Local Mac Rust job reproduction, before/after, on this branch.

Run from the repository with scripts/heavy. Build output stays in target/.
Runner provisioning/download/cache restore/upload are excluded. This is not
GitHub-hosted wall time. Each phase's exit status is retained even on failure.
"""
import hashlib
import json
import os
from pathlib import Path
import platform
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[2]
OUT = Path(__file__).resolve().parent
os.chdir(ROOT)
# A Work session's private socket overrides even an explicit --socket in hook
# tests. Hosted CI has none of these variables; never send test hooks to it.
removed_session_variables = sorted(k for k in os.environ if k.startswith('FARCOOLER_') and k != 'FARCOOLER_TEST_SHELL')
for key in removed_session_variables:
    os.environ.pop(key)
os.environ.update(CARGO_BUILD_JOBS='3', CARGO_PROFILE_DEV_DEBUG='0', NEXTEST_TEST_THREADS='3')
os.environ['PATH'] = f"{Path.home()}/.cargo/bin:/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin"
record = {'source': subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip(),
          'platform': platform.platform(), 'removed_session_variables': removed_session_variables, 'threads': 3, 'target': str(ROOT/'target'),
          'selection_sha256': hashlib.sha256((ROOT/'scripts/macos-rust-tests.json').read_bytes()).hexdigest(),
          'note': 'Local reproduction; runner provisioning and cache restore/upload excluded.', 'runs': []}
common = [['cargo','clippy','--workspace','--all-targets','--','-D','warnings']]
for package, feature in [('tailcat','helper'),('tailcat','linked'),('daemon','tailcat-helper'),
                         ('daemon','tailcat'),('client','tailcat'),('cli','tailcat'),('jni','tailcat')]:
    common.append(['cargo','clippy','-p',f'farcooler-{package}','--features',feature,'--all-targets','--','-D','warnings'])
common += [['cargo','check','-p','farcooler-tailcat','--features','linked'],
           ['cargo','check','-p','farcooler-tailcat','--features','linked','--target','aarch64-unknown-linux-musl'],
           ['cargo','nextest','run','--profile','ci','-p','farcooler-tailcat','--features','helper']]
for target in ['an_empty_allowlist_starts_no_tunnel','revocation_closes_what_it_revoked',
               'a_pairing_gives_the_runner_a_tunnel','a_sweep_rebuilds_the_tunnel_once',
               'a_configured_derp_map_reaches_the_tunnel']:
    common.append(['cargo','nextest','run','--profile','ci','-p','farcooler-daemon','--features','tailcat-helper','--test',target])
common += [['cargo','check','-p','farcooler-daemon','--features','tailcat','--all-targets'],
           ['cargo','build','--workspace','--bins']]
for name in ['farcooler-local','farcooler-preview','farcooler-canary']:
    link = ROOT/'target/debug'/name
    if not link.exists() and not link.is_symlink():
        link.parent.mkdir(parents=True,exist_ok=True)
        link.symlink_to(ROOT/'target/debug/farcooler')
os.environ['PATH'] = str(ROOT/'target/debug')+':'+os.environ['PATH']
tests = [sys.executable,'scripts/test-leak-check.py','--',sys.executable,'scripts/tmux-leak-check.py','--',
         'cargo','nextest','run','--profile','ci','--workspace','--no-fail-fast']


def run(label, commands):
    entry = {'label':label, 'steps':[], 'start_load':os.getloadavg()}
    record['runs'].append(entry)
    start = time.monotonic()
    for index, command in enumerate(commands):
        log = f'{label}-{index:02d}.log'
        now = time.monotonic()
        print(f'{label} {index}: {command[:8]}', flush=True)
        with (OUT/log).open('w') as stream:
            rc = subprocess.run(command,stdout=stream,stderr=subprocess.STDOUT).returncode
        entry['steps'].append({'command':command,'seconds':time.monotonic()-now,'exit':rc,'log':log})
        entry['seconds'] = time.monotonic()-start
        entry['end_load'] = os.getloadavg()
        (OUT/'timings.json').write_text(json.dumps(record,indent=2)+'\n')
    return entry


# First populate all feature/check artifacts. Do not count a cold first pass
# as savings from selection. The before and after runs use the same target.
run('prepare',common)
for label in ['before','after']:
    selection = subprocess.check_output([sys.executable,'scripts/macos-rust-tests.py'],text=True).strip()
    commands = list(common)
    if label == 'after':
        commands += [['cargo','nextest','run','--profile','ci','--workspace','--no-run']]
    commands += [tests + (['-E',selection] if label == 'after' else []),
                 ['cargo','test','--workspace','--doc']]
    run(label,commands)
print('Wrote reports/ov-1160/timings.json',flush=True)
