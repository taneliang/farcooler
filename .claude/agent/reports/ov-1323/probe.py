"""Controlled scheduling probes, using this checkout's real CLI and daemon.

The wrappers delay real subprocesses; they never synthesize CLI or tmux output.
Run from the repository root: python3 probe.py OUTPUT_DIRECTORY
"""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time

out = Path(sys.argv[1]).resolve()
out.mkdir(parents=True, exist_ok=True)
home = Path('/tmp') / ('fco-probe-' + str(os.getpid()))
home.mkdir()
cli = str(Path('target/debug/farcooler').resolve())
tmux = '/opt/homebrew/bin/tmux'
events = []
start = time.monotonic()


def record(kind, **fields):
    event = dict(seconds=round(time.monotonic() - start, 3), kind=kind, **fields)
    events.append(event)
    (out / 'events.json').write_text(json.dumps(events, indent=2))
    print(json.dumps(event), flush=True)


wrappers = home / 'wrappers'
wrappers.mkdir()
# list-panes has a one-second deadline, and inventory refresh tries twice.
# Delay both reads on ONE explicitly armed CLI invocation.
(wrappers / 'tmux').write_text('''#!/bin/sh
case " $* " in
  *" list-panes "*)
    if [ -f "$PROBE_LOOKUP/remaining" ]; then
      n=$(cat "$PROBE_LOOKUP/remaining")
      if [ "$n" -gt 0 ]; then
        echo $((n - 1)) > "$PROBE_LOOKUP/remaining"
        echo "list-panes delayed at $(date +%s) pid $$" >> "$PROBE_LOOKUP/wrappers.log"
        exec /bin/sleep 10
      fi
    fi ;;
esac
exec /opt/homebrew/bin/tmux "$@"
''')
# Hold a real watcher sample AFTER its initial shell observation. lsof is
# awaited between the process-table read and publishing runningAgent.
(wrappers / 'lsof').write_text('''#!/bin/sh
if [ -f "$PROBE_WATCH/arm" ]; then
  mv "$PROBE_WATCH/arm" "$PROBE_WATCH/held"
  echo "lsof held at $(date +%s) pid $$" >> "$PROBE_WATCH/wrappers.log"
  /bin/sleep 45
  echo "lsof released at $(date +%s) pid $$" >> "$PROBE_WATCH/wrappers.log"
fi
exec /usr/sbin/lsof "$@"
''')
for wrapper in wrappers.iterdir():
    wrapper.chmod(0o755)
env = {**os.environ, 'PATH': str(wrappers) + ':/usr/bin:/bin:/usr/sbin:/sbin:/opt/homebrew/bin',
       'FARCOOLER_HOME': str(home / 'h'), 'FARCOOLER_CONFIG': str(home / 'config.toml'),
       'FARCOOLER_TEST_SHELL': '/bin/sh', 'FARCOOLER_TEST_STUB_AGENTS': '1',
       'PROBE_WATCH': str(out), 'RUST_LOG': 'warn'}


def call(args, extra=None):
    ran = subprocess.run([cli, *args], env={**env, **(extra or {})},
                         capture_output=True, text=True, timeout=65)
    record('cli', args=args, status=ran.returncode, stdout=ran.stdout, stderr=ran.stderr)
    return ran


def obj(args):
    ran = call(['--json', *args])
    assert ran.returncode == 0, ran.stderr
    return json.loads(ran.stdout)


def pane():
    fleet = obj(['worktree', 'list'])
    return next(t for w in fleet['worktrees'] for t in w['terminals'] if t['id'] == terminal)


socket = None
try:
    demo = home / 'repos/demo'
    demo.mkdir(parents=True)
    for args in [['init', '-q'], ['-c', 'user.name=t', '-c', 'user.email=t@t', 'commit', '-q', '--allow-empty', '-m', 'i']]:
        subprocess.run(['/usr/bin/git', '-C', str(demo), *args], check=True)
    os.symlink('/bin/sleep', home / 'claude')
    obj(['daemon', 'ensure'])
    socket = 'farcooler-' + (home / 'h/install-id').read_text().strip()
    assert call(['root', 'add', str(home / 'repos')]).returncode == 0
    assert call(['repo', 'register', str(demo)]).returncode == 0
    worktree = obj(['worktree', 'create', 'demo', 'wt1', '--branch', 'wt1', '--fork-only'])['short']
    terminal = obj(['terminal', 'create', worktree])['id']
    deadline = time.monotonic() + 60
    while pane().get('runningAgent') != '':
        assert time.monotonic() < deadline
        time.sleep(.1)
    # Both read deadlines expire on this CLI alone. The server and shell
    # remain alive. This is the ov-1308 error path, before any write occurs.
    (out / 'remaining').write_text('2')
    lookup = call(['terminal', 'send', terminal, str(home / 'claude') + ' 600\r'],
                  {'PROBE_LOOKUP': str(out)})
    assert lookup.returncode == 1 and 'resource not found' in lookup.stderr
    record('live_inventory_after_failed_lookup', text=subprocess.check_output(
        [tmux, '-L', socket, 'list-panes', '-a', '-F',
         '#{pane_id} #{@farcooler_terminal_id} #{pane_dead} #{pane_current_command} #{pane_tty}'], text=True))
    assert lookup.returncode == 1 and 'resource not found' in lookup.stderr
    record('raw_inventory_after_failed_lookup', text=subprocess.check_output(
        [tmux, '-L', socket, 'list-panes', '-a', '-F', '#{pane_id} #{pane_dead} #{@farcooler_terminal_id} #{pane_current_command}'], text=True))
    # Now slow the watcher, not the shell. The successful send executes once.
    (out / 'arm').touch()
    assert call(['terminal', 'send', terminal, str(home / 'claude') + ' 600\r']).returncode == 0
    deadline = time.monotonic() + 60
    while not (out / 'held').exists():
        assert time.monotonic() < deadline
        time.sleep(.1)
    processes = subprocess.check_output(
        ['/bin/ps', '-axo', 'pid=,ppid=,pgid=,tty=,stat=,args='], text=True)
    record('foreground_while_watcher_held', rows=[
        line for line in processes.splitlines() if str(home / 'claude') in line])
    # The original test's exact detection loop (60 reads, 500 ms sleeps).
    for attempt in range(60):
        last = pane()
        if last.get('runningAgent') == 'claude':
            break
        time.sleep(.5)
    record('old_loop', attempts=attempt + 1, runningAgent=last.get('runningAgent'))
    assert last.get('runningAgent') == '', 'delay did not outlast the old loop'
    # A fresh monotonic condition wait sees the same, still-running process.
    deadline = time.monotonic() + 60
    while pane().get('runningAgent') != 'claude':
        assert time.monotonic() < deadline
        time.sleep(.1)
    record('condition_observed', runningAgent='claude')
finally:
    call(['daemon', 'stop'])
    if socket:
        subprocess.run([tmux, '-L', socket, 'kill-server'], capture_output=True)
    # Keep only evidence; the scratch home is owned by this process.
    shutil.rmtree(home)

# Keep a compact, reviewable extraction alongside the full RPC transcript.
summary = {'events': [e for e in events if e['kind'] != 'cli' or e['args'][:2] == ['terminal', 'send']],
           'list_samples': []}
for event in events:
    if event['kind'] == 'cli' and event['args'] == ['--json', 'worktree', 'list']:
        fleet = json.loads(event['stdout'])
        summary['list_samples'].append(dict(seconds=event['seconds'], terminals=[
            dict(id=t['id'], runningAgent=t.get('runningAgent'))
            for w in fleet['worktrees'] for t in w['terminals']]))
(out / 'summary.json').write_text(json.dumps(summary, indent=2))
