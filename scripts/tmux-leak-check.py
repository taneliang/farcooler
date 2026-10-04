#!/usr/bin/env python3
"""Fail if a command leaves a tmux server running that it started (ov-207).

A test tmux server once outlived its run by 15.5 hours at 92% CPU. Teardown now
ends its server by PID (`farcooler_tmux::reap_server`), but a test that never
calls it, or a new harness that forgets, leaks all the same, and nothing in the
test run would notice. This notices: it lists every named tmux server
(`tmux -L <socket> ...`) before the command, runs the command, waits a few
seconds, and fails on any server that wasn't there before and still is. Servers
that were already running (the owner's live one) are never touched or counted.
A leak is reported by PID and ended by exact PID, never by pattern.

  ./scripts/tmux-leak-check.py -- cargo test -p farcooler-tmux
  ./scripts/tmux-leak-check.py --self-test
"""

import os
import re
import shutil
import signal
import subprocess
import sys
import time

GRACE_SECONDS = 5


def servers():
    """pid -> command line of every `tmux -L <socket>` process running now."""
    out = subprocess.run(["ps", "-axo", "pid=,command="], capture_output=True, text=True, check=True).stdout
    found = {}
    for line in out.splitlines():
        pid, _, command = line.strip().partition(" ")
        if re.search(r"(^|/)tmux\s+(-\S+\s+)*-L\s+\S+", command) and "new-session" in command:
            found[int(pid)] = command
    return found


def leaked(before, grace=GRACE_SECONDS):
    """Servers started since `before` that are still running after `grace`."""
    deadline = time.time() + grace
    while True:
        new = {p: c for p, c in servers().items() if p not in before}
        if not new or time.time() >= deadline:
            return new
        time.sleep(0.25)


def end(pids):
    for sig in (signal.SIGTERM, signal.SIGKILL):
        for pid in pids:
            try:
                os.kill(pid, sig)
            except ProcessLookupError:
                pass
        time.sleep(1)


def run(argv):
    before = servers()
    status = subprocess.run(argv).returncode
    new = leaked(before)
    for pid, command in new.items():
        print(f"tmux-leak-check: LEAKED server {pid}: {command[:160]}", file=sys.stderr)
    end(list(new))
    if new:
        return 1
    return status


def self_test():
    tmux = shutil.which("tmux")
    if not tmux:
        print("tmux-leak-check: SKIP self-test, no tmux")
        return 0
    socket = f"leakcheck-{os.getpid()}"
    before = servers()
    # A command that leaks: starts a server and walks away.
    leak = [tmux, "-L", socket, "-f", "/dev/null", "new-session", "-d", "-s", "x", "sleep 600"]
    rc = run(leak)
    if rc == 0:
        print("tmux-leak-check: self-test FAILED, a leaked server was not caught", file=sys.stderr)
        return 1
    time.sleep(0.5)
    if leaked(before, grace=0):
        print("tmux-leak-check: self-test FAILED, the leaked server was not ended", file=sys.stderr)
        return 1
    # A command that cleans up after itself is not a leak.
    clean = f"{tmux} -L {socket}b -f /dev/null new-session -d -s x 'sleep 600'; {tmux} -L {socket}b kill-server"
    if run(["sh", "-c", clean]) != 0:
        print("tmux-leak-check: self-test FAILED, a clean command was flagged", file=sys.stderr)
        return 1
    print("tmux-leak-check: self-test ok")
    return 0


if __name__ == "__main__":
    args = sys.argv[1:]
    if args == ["--self-test"]:
        sys.exit(self_test())
    if args[:1] == ["--"] and len(args) > 1:
        sys.exit(run(args[1:]))
    print(__doc__)
    sys.exit(2)
