"""Run the actual Swift regression with a controlled subprocess scheduling delay.

Usage: /usr/bin/python3 fault-test.py OUTPUT lookup|watcher
Exactly 16 CPU workers; every owned PID is recorded and reaped.
"""
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import time

out = Path(sys.argv[1]).resolve()
out.mkdir(parents=True, exist_ok=True)
mode = sys.argv[2]
assert mode in ('lookup', 'watcher')
wrapper = out / 'bin'
wrapper.mkdir(exist_ok=True)
(wrapper / 'tmux').write_text('''#!/usr/bin/python3
import json, os, pathlib, subprocess, sys, time
out = pathlib.Path(os.environ['OV1323_FAULT'])
args = sys.argv[1:]
parent = subprocess.check_output(['/bin/ps', '-p', str(os.getppid()), '-o', 'command='], text=True)
if 'list-panes' in args and 'terminal send ' in parent and os.environ['OV1323_MODE'] == 'lookup':
    counter = out / 'lookup-count'
    n = int(counter.read_text()) if counter.exists() else 0
    if n < 2:
        counter.write_text(str(n + 1))
        live = subprocess.check_output(['/opt/homebrew/bin/tmux', *args[:args.index('list-panes')], 'list-panes', '-a', '-F', '#{pane_id} #{@farcooler_terminal_id} #{pane_dead} #{pane_current_command} #{pane_tty}'], text=True)
        with (out / 'faults.jsonl').open('a') as log:
            log.write(json.dumps(dict(kind='lookup-delayed', pid=os.getpid(), time=time.time(), parent=parent, live=live)) + '\\n')
        # The runtime's one-second deadline kills and reaps this process.
        time.sleep(10)
if 'send-keys' in args and any('/claude 600' in a for a in args):
    with (out / 'faults.jsonl').open('a') as log:
        log.write(json.dumps(dict(kind='send-keys', pid=os.getpid(), time=time.time(), args=args)) + '\\n')
    if os.environ['OV1323_MODE'] == 'watcher':
        (out / 'arm').touch()
os.execv('/opt/homebrew/bin/tmux', ['tmux', *args])
''')
(wrapper / 'lsof').write_text('''#!/usr/bin/python3
import json, os, pathlib, sys, time
out = pathlib.Path(os.environ['OV1323_FAULT'])
arm = out / 'arm'
if arm.exists():
    arm.unlink()
    (out / 'held-pid').write_text(str(os.getpid()))
    import subprocess
    processes = subprocess.check_output(['/bin/ps', '-axo', 'pid=,ppid=,pgid=,tty=,stat=,args='], text=True)
    rows = [line for line in processes.splitlines() if '/claude 600' in line and '/tmp/fco-' in line]
    with (out / 'faults.jsonl').open('a') as log:
        log.write(json.dumps(dict(kind='watcher-held', pid=os.getpid(), time=time.time(), args=sys.argv[1:], foreground=rows)) + '\\n')
    time.sleep(45)
    with (out / 'faults.jsonl').open('a') as log:
        log.write(json.dumps(dict(kind='watcher-released', pid=os.getpid(), time=time.time())) + '\\n')
os.execv('/usr/sbin/lsof', ['lsof', *sys.argv[1:]])
''')
for path in wrapper.iterdir():
    path.chmod(0o755)
workers = []
try:
    for _ in range(16):
        workers.append(subprocess.Popen(['/usr/bin/yes'], stdout=subprocess.DEVNULL))
        (out / 'load-pids.json').write_text(json.dumps([p.pid for p in workers]))
    started = time.monotonic()
    with (out / 'swift.log').open('w') as log:
        ran = subprocess.run(['swift', 'test', '--skip-build', '--jobs', '4', '--filter',
                              'NativeOfferTests.theRealCLINamesTheRunningAgent'],
                             cwd='apps/macos', stdout=log, stderr=subprocess.STDOUT,
                             env={**os.environ, 'TMPDIR': '/tmp', 'PATH': str(wrapper) + ':/usr/bin:/bin:/usr/sbin:/sbin:/opt/homebrew/bin',
                                  'OV1323_FAULT': str(out), 'OV1323_MODE': mode})
    (out / 'result.json').write_text(json.dumps(dict(exit=ran.returncode,
        seconds=time.monotonic() - started, workers=sum(p.poll() is None for p in workers),
        load=os.getloadavg()), indent=2))
    print((out / 'result.json').read_text(), flush=True)
finally:
    for p in workers:
        if p.poll() is None:
            os.kill(p.pid, signal.SIGTERM)
    for p in workers:
        p.wait()
    (out / 'load-stopped.json').write_text(json.dumps([dict(pid=p.pid, exit=p.returncode) for p in workers], indent=2))
