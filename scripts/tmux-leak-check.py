#!/usr/bin/env python3
"""Fail if a command leaves a tmux server or a farcoolerd running that it started (ov-207, ov-566).

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

A `farcoolerd` is a leak too (ov-566). The CLI starts a daemon on demand,
detached, so a test that runs `farcooler <command>` under a scratch home and
never stops the daemon leaves it running after the run, parented to launchd:
four were found after full Mac runs. A daemon the command started inherits the
private `TMUX_TMPDIR`, which is how this tells it from the owner's app, another
lane's daemon, or a developer's, none of which carries it; `ps eww` shows that
environment for a process that is not a platform binary. A daemon still alive
after the grace period is named, and ended in CI.

  ./scripts/tmux-leak-check.py -- cargo test -p farcooler-tmux
  ./scripts/tmux-leak-check.py --self-test
"""

from contextlib import ExitStack
import os
from pathlib import Path
import re
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import time
import process_guard

GRACE_SECONDS = 5


def server_pids(base):
    """PIDs of the servers behind the sockets under `base/tmux-<uid>/`."""
    sockets = os.path.join(base, f"tmux-{os.getuid()}")
    if not os.path.isdir(sockets):
        return set()
    paths = [os.path.realpath(os.path.join(sockets, name)) for name in os.listdir(sockets)]
    if not paths:
        return set()
    # One lsof for every socket: it ORs its file arguments, and each separate
    # run costs a scan of the whole file table (seconds on a busy Mac).
    out = subprocess.run(["lsof", "-t", "--", *paths], capture_output=True, text=True).stdout
    return {int(p) for p in out.split()}


# `.../farcoolerd`, `farcoolerd-local`: the daemon and its channel builds, in any mode.
DAEMON = re.compile(r"(^|/)farcoolerd(-[a-z]+)?( |$)")


def daemon_pids(base):
    """PIDs of the `farcoolerd` processes that carry `TMUX_TMPDIR=<base>` in their environment."""
    listing = subprocess.run(["ps", "-axo", "pid=,command="], capture_output=True, text=True).stdout
    candidates = []
    for line in listing.splitlines():
        pid, _, command = line.strip().partition(" ")
        if pid.isdigit() and int(pid) != os.getpid() and DAEMON.search(command.lstrip().split(" --", 1)[0] + " "):
            candidates.append(pid)
    if not candidates:
        return set()
    # `ps eww` prints each process's environment after its command line.
    shown = subprocess.run(["ps", "eww", "-p", ",".join(candidates)], capture_output=True, text=True).stdout
    marker = re.compile(r"(^|\s)TMUX_TMPDIR=" + re.escape(base) + r"(\s|$)")
    found = set()
    for line in shown.splitlines():
        head = line.split(None, 1)
        if head and head[0].isdigit() and marker.search(line):
            found.add(int(head[0]))
    return found


def leaked(base, grace=GRACE_SECONDS):
    """(servers, daemons) still behind a socket or alive under `base` after `grace` seconds."""
    deadline = time.time() + grace
    while True:
        servers, daemons = server_pids(base), daemon_pids(base)
        if not (servers or daemons) or time.time() >= deadline:
            return servers, daemons
        time.sleep(0.25)


def end(pids):
    for sig in (signal.SIGTERM, signal.SIGKILL):
        for pid in pids:
            try:
                os.kill(pid, sig)
            except ProcessLookupError:
                pass
        time.sleep(1)


def run(argv, kill, grace=GRACE_SECONDS, *, base=None):
    """Run `argv` with a private TMUX_TMPDIR; return (exit code, leaked PIDs)."""
    # Short and directly under /tmp: a unix socket path is limited to about
    # 100 bytes, and $TMPDIR on a Mac is already long.
    if base is None:
        base = tempfile.mkdtemp(prefix="fc-leak-", dir="/tmp")
    try:
        status = process_guard.run(argv, env={**os.environ, "TMUX_TMPDIR": base}).returncode
        servers, daemons = leaked(base, grace)
        for pid in sorted(servers):
            print(f"tmux-leak-check: LEAKED tmux server {pid} (socket under {base})", file=sys.stderr)
        for pid in sorted(daemons):
            print(f"tmux-leak-check: LEAKED farcoolerd {pid} (started by the run; a test must stop the daemon it starts)", file=sys.stderr)
        pids = servers | daemons
        if kill:
            end(pids)
        return (1 if pids else status), pids
    finally:
        # Kept while a leaked server or daemon remains, so it can still be found.
        if not (server_pids(base) or daemon_pids(base)):
            shutil.rmtree(base, ignore_errors=True)


def stop_fixture(child):
    if child.poll() is None:
        child.kill()
    child.wait()


def self_test():
    tmux = shutil.which("tmux")
    if not tmux or not shutil.which("lsof"):
        print("tmux-leak-check: SKIP self-test, no tmux or lsof")
        return 0
    failures = []
    noop = [sys.executable, "-c", "pass"]

    with ExitStack() as fixtures:
        def private_dir(prefix="fc-leak-"):
            return fixtures.enter_context(tempfile.TemporaryDirectory(prefix=prefix, dir="/tmp"))

        def spawn(argv, base):
            child = subprocess.Popen(argv, env={**os.environ, "TMUX_TMPDIR": base},
                                     stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL)
            fixtures.callback(stop_fixture, child)
            return child

        def server(base):
            # A foreground server keeps its real socket but stays in the
            # self-test's guarded session. No pane or shell is needed.
            child = spawn([tmux, "-D", "-L", "fixture", "-f", "/dev/null"], base)
            address = Path(base) / f"tmux-{os.getuid()}" / "fixture"
            deadline = time.monotonic() + 15
            while not address.exists():
                if child.poll() is not None or time.monotonic() >= deadline:
                    raise RuntimeError("tmux fixture did not open its socket")
                time.sleep(0.05)
            return child

        def stale_sockets(base):
            directory = Path(base) / f"tmux-{os.getuid()}"
            directory.mkdir(mode=0o700, exist_ok=True)
            for i in range(3):
                # A closed Unix socket leaves the same inode as a dead server.
                with socket.socket(socket.AF_UNIX) as stale:
                    stale.bind(str(directory / f"stale-{i}"))

        def check(label, base, *, child=None, kill=False, command=noop):
            rc, pids = run(command, kill, grace=0, base=base)
            expected = {child.pid} if child else set()
            if rc != int(bool(expected)) or pids != expected:
                failures.append(f"{label}: exit {rc}, PIDs {pids}, expected PIDs {expected}")
            if child and (child.poll() is not None) != kill:
                failures.append(f"{label}: wrong state for fixture {child.pid}")
            if child:
                stop_fixture(child)

        # The bystander shares the fixture lifetime, but uses a different
        # socket directory and must survive every individual leak check.
        other = private_dir("fc-other-")
        bystander = server(other)
        fakes = private_dir("fc-fake-")
        fake = Path(fakes) / "farcoolerd"
        fake.write_text("import time\ntime.sleep(600)\n")
        daemon_base = private_dir()
        daemon = spawn([sys.executable, str(fake)], daemon_base)
        process_guard.self_test_ready(bystander.pid, daemon.pid)

        base = private_dir()
        check("leaked server outside CI", base, child=server(base))
        base = private_dir()
        check("leaked server in CI", base, child=server(base), kill=True)
        base = private_dir()
        clean = server(base)
        check("clean server", base, command=[tmux, "-L", "fixture", "kill-server"])
        if clean.wait(timeout=8) != 0:
            failures.append("the clean server failed to exit")
        base = private_dir()
        live = server(base)
        stale_sockets(base)
        check("live server among stale sockets", base, child=live, kill=True)
        base = private_dir()
        stale_sockets(base)
        check("stale sockets alone", base)
        if bystander.poll() is not None:
            failures.append("a server outside the private TMUX_TMPDIR was killed")

        # The fakes stand outside the command's session, like a detached
        # daemon, but the outer self-test guard still owns their lifetime.
        check("leaked farcoolerd outside CI", daemon_base, child=daemon)
        base = private_dir()
        check("leaked farcoolerd in CI", base, child=spawn([sys.executable, str(fake)], base), kill=True)
        stops = [sys.executable, "-c", "import subprocess, sys; "
                 "p = subprocess.Popen([sys.executable, sys.argv[1]]); p.terminate(); p.wait()", str(fake)]
        check("stopped farcoolerd", private_dir(), command=stops)

        lone = spawn([sys.executable, str(fake)], other)
        check("farcoolerd of another run", private_dir(), kill=True)
        if lone.poll() is not None or lone.pid not in daemon_pids(other):
            failures.append("a farcoolerd outside the private TMUX_TMPDIR was killed or not identified")
        stop_fixture(lone)
        talker = spawn([sys.executable, "-c", "import time; time.sleep(60)", "tail", "/x/farcoolerd.log"], other)
        if daemon_pids(other):
            failures.append("a process that only names farcoolerd was taken for one")
        stop_fixture(talker)

    for failure in failures:
        print(f"tmux-leak-check: self-test FAILED, {failure}", file=sys.stderr)
    if failures:
        return 1
    print("tmux-leak-check: self-test ok")
    return 0


if __name__ == "__main__":
    args = sys.argv[1:]
    if args == ["--self-test"]:
        sys.exit(process_guard.run_self_test([sys.executable, __file__, "--self-test-worker"]))
    if args == ["--self-test-worker"]:
        sys.exit(self_test())
    if args[:1] == ["--"] and len(args) > 1:
        sys.exit(run(args[1:], kill=bool(os.environ.get("CI")))[0])
    print(__doc__)
    sys.exit(2)
