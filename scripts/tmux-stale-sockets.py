#!/usr/bin/env python3
"""List, and with --delete remove, tmux sockets that no server answers on (ov-286).

Test runs leave a socket file behind whenever their tmux server is killed
rather than asked to exit; /private/tmp/tmux-502 once held 65,000 of them.
A socket is stale only when `tmux -S <path> ls` fails with "no server running"
or "connection refused". A socket whose server answers, or any other outcome
(a timeout, an unfamiliar error, a path that is not a socket), is left alone.
No tmux process is ever signaled.

  ./scripts/tmux-stale-sockets.py                  dry run on $TMUX_TMPDIR/tmux-<uid>
  ./scripts/tmux-stale-sockets.py --dir DIR        dry run on DIR
  ./scripts/tmux-stale-sockets.py --delete         remove the stale sockets
  ./scripts/tmux-stale-sockets.py --self-test
"""

import argparse
import os
import shutil
import stat
import subprocess
import sys
import tempfile
from concurrent.futures import ThreadPoolExecutor

DEAD = ("no server running", "connection refused")
ASK_SECONDS = 10


def default_dir():
    return os.path.join(os.environ.get("TMUX_TMPDIR", "/tmp"), f"tmux-{os.getuid()}")


def classify(tmux, path):
    """'dead', 'alive' or 'unknown' for the socket at `path`."""
    try:
        if not stat.S_ISSOCK(os.lstat(path).st_mode):
            return "unknown"
        done = subprocess.run([tmux, "-S", path, "ls"], capture_output=True, text=True, timeout=ASK_SECONDS)
    except (OSError, subprocess.TimeoutExpired):
        return "unknown"
    if done.returncode == 0:
        return "alive"
    return "dead" if any(m in done.stderr.lower() for m in DEAD) else "unknown"


def scan(tmux, directory, jobs):
    """(dead paths, counts by class) for every entry in `directory`."""
    paths = [os.path.join(directory, n) for n in sorted(os.listdir(directory))]
    counts = {"dead": 0, "alive": 0, "unknown": 0}
    dead = []
    with ThreadPoolExecutor(max_workers=jobs) as pool:
        for path, kind in zip(paths, pool.map(lambda p: classify(tmux, p), paths)):
            counts[kind] += 1
            if kind == "dead":
                dead.append(path)
    return dead, counts


def remove(tmux, paths):
    """Remove each path, rechecking it first so a server that has since started stays."""
    removed = 0
    for path in paths:
        if classify(tmux, path) != "dead":
            continue
        try:
            os.unlink(path)
            removed += 1
        except FileNotFoundError:
            pass
    return removed


def self_test():
    import socket
    import time

    tmux = shutil.which("tmux")
    if not tmux:
        print("tmux-stale-sockets: SKIP self-test, no tmux")
        return 0
    base = tempfile.mkdtemp(prefix="fc-stale-", dir="/tmp")
    sockets = os.path.join(base, f"tmux-{os.getuid()}")
    os.makedirs(sockets)
    live = os.path.join(sockets, "live")
    dead = os.path.join(sockets, "dead")
    plain = os.path.join(sockets, "plain-file")
    failures = []
    try:
        subprocess.run([tmux, "-S", live, "-f", "/dev/null", "new-session", "-d", "-s", "x", "sleep 600"], check=True)
        # A dead socket: bound, then abandoned with no listener.
        s = socket.socket(socket.AF_UNIX)
        s.bind(dead)
        s.close()
        open(plain, "w").close()
        dead_found, counts = scan(tmux, sockets, 4)
        if dead_found != [dead] or counts != {"dead": 1, "alive": 1, "unknown": 1}:
            failures.append(f"scan wrong: {dead_found} {counts}")
        if not (os.path.exists(dead) and os.path.exists(live)):
            failures.append("a dry run removed something")
        if remove(tmux, dead_found) != 1:
            failures.append("the dead socket was not removed")
        if os.path.exists(dead):
            failures.append("the dead socket is still there")
        if not os.path.exists(live) or not os.path.exists(plain):
            failures.append("a live socket or other file was removed")
        if classify(tmux, live) != "alive":
            failures.append("the live server stopped answering")
        # A path handed over as dead that now answers must survive.
        if remove(tmux, [live]) != 0 or not os.path.exists(live):
            failures.append("a live socket was removed when passed as stale")
    finally:
        subprocess.run([tmux, "-S", live, "kill-server"], capture_output=True)
        time.sleep(0.2)
        shutil.rmtree(base, ignore_errors=True)
    if failures:
        for f in failures:
            print(f"tmux-stale-sockets: self-test FAILED, {f}", file=sys.stderr)
        return 1
    print("tmux-stale-sockets: self-test ok")
    return 0


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--dir", default=None)
    ap.add_argument("--delete", action="store_true")
    ap.add_argument("--jobs", type=int, default=16, help="parallel checks (default 16)")
    ap.add_argument("--self-test", action="store_true")
    args = ap.parse_args()
    if args.self_test:
        return self_test()
    tmux = shutil.which("tmux")
    if not tmux:
        print("tmux-stale-sockets: no tmux on PATH", file=sys.stderr)
        return 2
    directory = args.dir or default_dir()
    if not os.path.isdir(directory):
        print(f"tmux-stale-sockets: no such directory {directory}", file=sys.stderr)
        return 2
    dead, counts = scan(tmux, directory, max(1, args.jobs))
    print(f"{directory}: {counts['dead']} stale, {counts['alive']} answering, {counts['unknown']} left alone")
    if args.delete:
        print(f"removed {remove(tmux, dead)}")
    else:
        print("dry run; pass --delete to remove the stale ones")
    return 0


if __name__ == "__main__":
    sys.exit(main())
