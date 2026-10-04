#!/usr/bin/env python3
"""Fail if a command leaves a tmux server running that it started (ov-207).

A test tmux server once outlived its run by 15.5 hours at 92% CPU. Teardown now
ends its server by PID (`farcooler_tmux::reap_server`), but a test that never
calls it, or a new harness that forgets, leaks all the same, and nothing in the
test run would notice. This notices.

It gives the command a private `TMUX_TMPDIR` that it makes, so every tmux
server the command starts keeps its socket there and nowhere else, then looks
only in that directory afterwards. A socket with a live server behind it is a
leak. It never looks at, or signals, any other tmux server: not the owner's,
not another lane's, not one that was already running.

  Outside CI it only reports a leak (exit 1) and names the PID.
  With `CI` set it also ends the leaked servers, by exact PID.

  ./scripts/tmux-leak-check.py -- cargo test -p farcooler-tmux
  ./scripts/tmux-leak-check.py --self-test
"""

import os
import shutil
import signal
import subprocess
import sys
import tempfile
import time

GRACE_SECONDS = 5
# Every private dir `run` made, for the self-test to remove once it has ended
# the servers it deliberately leaked.
BASES = []


def server_pids(base):
    """PIDs of the servers behind the sockets under `base/tmux-<uid>/`."""
    sockets = os.path.join(base, f"tmux-{os.getuid()}")
    pids = set()
    if not os.path.isdir(sockets):
        return pids
    for name in os.listdir(sockets):
        path = os.path.realpath(os.path.join(sockets, name))
        out = subprocess.run(["lsof", "-t", "--", path], capture_output=True, text=True).stdout
        pids.update(int(p) for p in out.split())
    return pids


def leaked(base, grace=GRACE_SECONDS):
    """Servers still behind a socket under `base` after `grace` seconds."""
    deadline = time.time() + grace
    while True:
        pids = server_pids(base)
        if not pids or time.time() >= deadline:
            return pids
        time.sleep(0.25)


def end(pids):
    for sig in (signal.SIGTERM, signal.SIGKILL):
        for pid in pids:
            try:
                os.kill(pid, sig)
            except ProcessLookupError:
                pass
        time.sleep(1)


def run(argv, kill, grace=GRACE_SECONDS):
    """Run `argv` with a private TMUX_TMPDIR; return (exit code, leaked PIDs)."""
    # Short and directly under /tmp: a unix socket path is limited to about
    # 100 bytes, and $TMPDIR on a Mac is already long.
    base = tempfile.mkdtemp(prefix="fc-leak-", dir="/tmp")
    BASES.append(base)
    try:
        status = subprocess.run(argv, env={**os.environ, "TMUX_TMPDIR": base}).returncode
        pids = leaked(base, grace)
        for pid in sorted(pids):
            print(f"tmux-leak-check: LEAKED tmux server {pid} (socket under {base})", file=sys.stderr)
        if kill:
            end(pids)
        return (1 if pids else status), pids
    finally:
        # Kept while a leaked server remains, so it can still be found.
        if not server_pids(base):
            shutil.rmtree(base, ignore_errors=True)


def alive(pid):
    try:
        os.kill(pid, 0)
        return True
    except ProcessLookupError:
        return False


def self_test():
    tmux = shutil.which("tmux")
    if not tmux or not shutil.which("lsof"):
        print("tmux-leak-check: SKIP self-test, no tmux or lsof")
        return 0
    start = "{tmux} -L {name} -f /dev/null new-session -d -s x 'sleep 600'"
    failures = []
    pids = set()

    # A bystander on its own TMUX_TMPDIR: never looked at or touched.
    other = tempfile.mkdtemp(prefix="fc-other-", dir="/tmp")
    env = {**os.environ, "TMUX_TMPDIR": other}
    subprocess.run(["sh", "-c", start.format(tmux=tmux, name="bystander")], env=env, check=True)
    bystanders = server_pids(other)
    try:
        # Outside CI: a leak is reported, the server is left running.
        rc, pids = run(["sh", "-c", start.format(tmux=tmux, name=f"leak{os.getpid()}")], kill=False, grace=1)
        if rc != 1 or len(pids) != 1:
            failures.append(f"a leaked server was not caught (rc={rc}, pids={pids})")
        if not all(alive(p) for p in pids):
            failures.append("a leak was killed outside CI")
        # In CI: a leak is reported and ended.
        rc2, pids2 = run(["sh", "-c", start.format(tmux=tmux, name=f"leakci{os.getpid()}")], kill=True, grace=1)
        if rc2 != 1 or len(pids2) != 1 or any(alive(p) for p in pids2):
            failures.append(f"a leaked server was not ended in CI (rc={rc2}, pids={pids2})")
        # A command that cleans up is not a leak.
        clean = start.format(tmux=tmux, name=f"clean{os.getpid()}") + f"; {tmux} -L clean{os.getpid()} kill-server"
        rc3, pids3 = run(["sh", "-c", clean], kill=False, grace=1)
        if rc3 != 0 or pids3:
            failures.append(f"a clean command was flagged (rc={rc3}, pids={pids3})")
        if not all(alive(p) for p in bystanders):
            failures.append("a server outside the private TMUX_TMPDIR was killed")
    finally:
        end([p for p in pids if alive(p)])
        end(list(bystanders))
        for base in BASES:
            shutil.rmtree(base, ignore_errors=True)
        shutil.rmtree(other, ignore_errors=True)
    if failures:
        for f in failures:
            print(f"tmux-leak-check: self-test FAILED, {f}", file=sys.stderr)
        return 1
    print("tmux-leak-check: self-test ok")
    return 0


if __name__ == "__main__":
    args = sys.argv[1:]
    if args == ["--self-test"]:
        sys.exit(self_test())
    if args[:1] == ["--"] and len(args) > 1:
        sys.exit(run(args[1:], kill=bool(os.environ.get("CI")))[0])
    print(__doc__)
    sys.exit(2)
