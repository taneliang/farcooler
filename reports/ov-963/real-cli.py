#!/usr/bin/python3
"""The real `farcooler` CLI and a production-feature `farcoolerd`, from a fresh
scratch home, Prepare to ordinary service. Each command runs as an orphan on its
own pty, which is a person to the recovery seat. FARCOOLER_TEST_CUTOVER_KILL_AT
is set throughout: a shipped build must ignore it."""
import fcntl, json, os, pty, shutil, signal, subprocess, sys, tempfile, termios, time
from pathlib import Path
root = Path(__file__).resolve().parents[2]
cli, daemon = root / 'target/debug/farcooler', root / 'target/debug/farcoolerd'
base = Path(tempfile.mkdtemp(prefix='fc963cli', dir='/tmp'))
home = base / 'h'; home.mkdir()
for name in ('t', 'c'): (base / name).mkdir()
env = {'PATH': '/usr/bin:/bin', 'HOME': str(base), 'FARCOOLER_HOME': str(home), 'FARCOOLER_CONFIG': str(home / 'config.toml'), 'CLAUDE_CONFIG_DIR': str(base / 'c'),
       'TMUX_TMPDIR': str(base / 't'), 'FARCOOLER_TEST_STUB_AGENTS': '1', 'FARCOOLER_TEST_CUTOVER_KILL_AT': 'commit.in-mapping'}
def person(*args):
    out = base / 'out'; out.unlink(missing_ok=True)
    pid = os.fork()
    if pid == 0:
        os.setsid()
        if os.fork(): os._exit(0)
        master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSCTTY, 0)
        log = os.open(str(out) + '.tmp', os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
        os.dup2(log, 1); os.dup2(log, 2)
        child = subprocess.Popen([str(cli), 'workflow-cutover', *args, '--json'] if False else [str(cli), '--json', 'workflow-cutover', *args], env=env, stdout=1, stderr=2)
        code = child.wait()
        os.write(1, f'\nEXIT {code}\n'.encode())
        os.rename(str(out) + '.tmp', out)
        os._exit(0)
    os.waitpid(pid, 0)
    end = time.time() + 60
    while not out.exists():
        assert time.time() < end, args
        time.sleep(.02)
    text = out.read_text()
    print(args, text.strip()[-400:], flush=True)
    return text
def up():
    d = subprocess.Popen([str(daemon)], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    end = time.time() + 60
    while not (home / 'farcoolerd.sock').exists():
        assert time.time() < end and d.poll() is None
        time.sleep(.02)
    return d
steps = {}
try:
    d = up()
    steps['inspect-live'] = person('inspect')
    steps['prepare-live'] = person('prepare', '--attempt', 'real')
    assert d.wait(timeout=20) == 0
    d = up()
    for op in (['prepare', '--attempt', 'real'], ['commit'], ['verify'], ['release']):
        steps[op[0]] = person(*op)
        assert 'EXIT 0' in steps[op[0]], op
        assert d.poll() is None, op
    steps['finish'] = person('finish')
    assert 'EXIT 0' in steps['finish']
    code = d.wait(timeout=20)
    assert code == 75, code
    d = up()
    steps['inspect-after'] = person('inspect')
    assert '"ordinary_startup":"Existing"' in steps['inspect-after'].replace(' ', '')
    assert '"phase":"released"' in steps['inspect-after'].replace(' ', '')
    d.terminate(); d.wait(timeout=20)
    print('real CLI: Prepare to ordinary service, kill variable ignored, exit 75 seen')
finally:
    try: d.kill()
    except Exception: pass
    install = home / 'install-id'
    if install.exists(): subprocess.run(['/usr/bin/env', 'tmux', '-L', 'farcooler-' + install.read_text().strip(), 'kill-server'], env=env, capture_output=True)
    json.dump({k: v[-600:] for k, v in steps.items()}, open(root / 'reports/ov-963/real-cli.json', 'w'), indent=1)
    shutil.rmtree(base, ignore_errors=True)
