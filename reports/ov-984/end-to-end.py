#!/usr/bin/python3
"""ov-984 end to end: a scratch runner on schema 60 starts in recovery, and the
Mac app's RunnerUpgrade (the recovery screen's model) runs Prepare, Commit,
Verify, Release and Finish through the real CLI against the real daemon. The
test process runs as an orphan on a pty of its own: a person outside every
pane, as the app is. Only Touch ID is stood in for. Then the runner starts in
ordinary service on the released data. Needs `apps/macos/test.sh` built first
(this runs `swift test --skip-build`) and target/debug/{farcooler,farcoolerd}."""
import fcntl, os, pty, shutil, subprocess, sys, tempfile, termios, time, json
from pathlib import Path
root = Path(__file__).resolve().parents[2]
cli, daemon = root / 'target/debug/farcooler', root / 'target/debug/farcoolerd'
writer = root / 'target/debug/examples/write_source60'
base = Path(tempfile.mkdtemp(prefix='fc984', dir='/tmp'))
home = base / 'h'; home.mkdir()
for name in ('t', 'c'): (base / name).mkdir()
env = {'PATH': '/usr/bin:/bin:/usr/sbin', 'HOME': str(base), 'FARCOOLER_HOME': str(home), 'FARCOOLER_CONFIG': str(home / 'config.toml'),
       'CLAUDE_CONFIG_DIR': str(base / 'c'), 'TMUX_TMPDIR': str(base / 't'), 'FARCOOLER_TEST_STUB_AGENTS': '1'}
def person(argv, extra=None, cwd=None, limit=600):
    """`argv` as an orphan in a session and on a pty of its own."""
    out = base / 'out'; out.unlink(missing_ok=True)
    pid = os.fork()
    if pid == 0:
        if os.fork(): os._exit(0)
        os.setsid()
        master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSCTTY, 0)
        log = os.open(str(out) + '.tmp', os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
        os.dup2(log, 1); os.dup2(log, 2)
        code = subprocess.Popen(argv, env={**env, **(extra or {})}, cwd=cwd, stdout=1, stderr=2).wait()
        os.write(1, f'\nEXIT {code}\n'.encode())
        os.rename(str(out) + '.tmp', out)
        os._exit(0)
    os.waitpid(pid, 0)
    end = time.time() + limit
    while not out.exists():
        assert time.time() < end, argv
        time.sleep(.05)
    return out.read_text()
def inspect():
    text = person([str(cli), '--json', 'workflow-cutover', 'inspect'])
    assert 'EXIT 0' in text, text
    return json.loads(text.split('\nEXIT')[0].strip().splitlines()[-1])
def up():
    d = subprocess.Popen([str(daemon)], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    end = time.time() + 60
    while not (home / 'farcoolerd.sock').exists():
        assert time.time() < end and d.poll() is None
        time.sleep(.02)
    return d
seen = {}
d = None
try:
    subprocess.run([str(writer), str(home / 'farcooler.db')], check=True)
    d = up()
    seen['before'] = inspect()
    assert seen['before']['ordinary_startup'].startswith('refused'), seen['before']
    swift = person(['/usr/bin/xcrun', 'swift', 'test', '--skip-build', '--filter', 'RunnerUpgradeEndToEnd'],
                   extra={'PATH': os.environ['PATH'], 'HOME': os.environ['HOME'], 'FARCOOLER_BIN': str(cli), 'FARCOOLER_UPGRADE_E2E': '1'},
                   cwd=str(root / 'apps/macos'))
    seen['swift'] = swift[-1500:]
    print(swift[-1500:], flush=True)
    assert 'EXIT 0' in swift and 'theButtonFinishesARealUpgrade' in swift and 'passed' in swift, 'the model run failed'
    code = d.wait(timeout=30)
    assert code == 75, code
    d = up()
    seen['after'] = inspect()
    assert seen['after']['ordinary_startup'] == 'Existing', seen['after']
    assert seen['after']['authority']['phase'] == 'released', seen['after']
    print('ov-984 end to end: schema 60 in recovery, the model ran all five steps, exit 75, ordinary service on released data')
finally:
    if d is not None:
        try: d.terminate(); d.wait(timeout=20)
        except Exception: d.kill()
    install = home / 'install-id'
    if install.exists(): subprocess.run(['/usr/bin/env', 'tmux', '-L', 'farcooler-' + install.read_text().strip(), 'kill-server'], env=env, capture_output=True)
    json.dump(seen, open(root / 'reports/ov-984/end-to-end.json', 'w'), indent=1, default=str)
    shutil.rmtree(base, ignore_errors=True)
