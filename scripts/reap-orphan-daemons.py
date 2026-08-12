#!/usr/bin/env python3
"""Kill farcoolerd processes that lost the race to own their runtime directory.

Before the startup lock existed, daemons started in the same moment all probed
the socket, all found nothing listening, all spent a hundred milliseconds or
more opening SQLite and inventorying tmux, and all bound — the last one
unlinking everyone else's socket. The losers never noticed and never exited.
They kept the database open and went on sampling every pane once a second,
through a watcher no client could reach, until the machine was restarted.

`farcoolerd` cannot reach that state any more (see `acquire_daemon_lock` in
crates/daemon/src/main.rs), but the ones already running will not leave on
their own.

    ./scripts/reap-orphan-daemons.py          # say what would be killed
    ./scripts/reap-orphan-daemons.py --yes    # actually kill it

THE OWNER OF EACH RUNTIME DIRECTORY IS NEVER TOUCHED. Ownership is settled by
asking the operating system which process is listening on that directory's
socket, not by guessing from age or command line: the listener is the one a
client actually reaches, and that is the only definition that matters. Where
nothing is listening, every daemon for that directory is left alone — there is
no owner to keep, so there is no safe way to choose which to kill.

Daemons in --stdio, --stream and --fanout mode are skipped entirely. They own no
socket, and each one is somebody's live session or stream.

Python rather than shell because this walks paths that contain spaces, which is
where the shell version of this script quietly went wrong.
"""

import os
import platform
import subprocess
import sys
from collections import defaultdict

# Modes that are not socket owners. Killing one of these ends a live ssh
# session, a terminal stream, or the pipe feeding every watcher of a pane.
WORKER_FLAGS = ("--stdio", "--stream", "--fanout")


def default_home() -> str:
    if platform.system() == "Darwin":
        return os.path.expanduser("~/Library/Application Support/com.farcooler.FarCooler")
    xdg = os.environ.get("XDG_DATA_HOME") or os.path.expanduser("~/.local/share")
    return os.path.join(xdg, "farcooler")


def run(args: list[str]) -> str:
    """Best-effort command output. A tool that is absent or refuses is not fatal
    here — every caller treats an empty answer as "nothing found"."""
    try:
        return subprocess.run(args, capture_output=True, text=True, timeout=30).stdout
    except (OSError, subprocess.SubprocessError):
        return ""


def daemon_pids() -> list[int]:
    return [int(p) for p in run(["pgrep", "-x", "farcoolerd"]).split()]


def command_of(pid: int) -> str:
    return run(["ps", "-o", "command=", "-p", str(pid)]).strip()


def home_of(pid: int) -> str:
    """The runtime directory a daemon is serving.

    Read from the process's own environment: a daemon started with its own
    FARCOOLER_HOME — which is how scratch daemons and agent-driven test runs are
    launched — is serving a different directory and is every bit as live as the
    default one. `ps eww` prints the environment after the command line.
    """
    for word in run(["ps", "eww", "-o", "command=", "-p", str(pid)]).split():
        if word.startswith("FARCOOLER_HOME="):
            return word[len("FARCOOLER_HOME=") :]
    return default_home()


def listener_of(home: str) -> int | None:
    """Whichever process is listening on this directory's socket, or None."""
    out = run(["lsof", "-t", "-U", "-a", "--", os.path.join(home, "farcoolerd.sock")])
    pids = [int(p) for p in out.split()]
    return pids[0] if pids else None


def main() -> int:
    confirm = "--yes" in sys.argv[1:]

    by_home: dict[str, list[int]] = defaultdict(list)
    for pid in daemon_pids():
        command = command_of(pid)
        if any(flag in command for flag in WORKER_FLAGS):
            continue
        by_home[home_of(pid)].append(pid)

    if not by_home:
        print("no socket-owning farcoolerd processes.")
        return 0

    keeping: list[str] = []
    doomed: list[tuple[int, str]] = []
    for home, pids in sorted(by_home.items()):
        owner = listener_of(home)
        if owner is None:
            print(
                f"!! nothing is listening on {home} — leaving all {len(pids)} of its "
                f"daemons alone, because there is no owner to tell them apart from",
                file=sys.stderr,
            )
            continue
        keeping.append(f"{owner}  {home}")
        doomed += [(pid, home) for pid in pids if pid != owner]

    print(f"keeping {len(keeping)} listening daemon(s):")
    for line in keeping:
        print(f"  {line}")

    if not doomed:
        print("no orphans.")
        return 0

    print(f"\n{len(doomed)} orphaned daemon(s) — running, listening on nothing.")
    for pid, home in doomed[:20]:
        print(f"  {pid}  {home}")
    if len(doomed) > 20:
        print(f"  … and {len(doomed) - 20} more")

    if not confirm:
        print("\nThis was a dry run. Re-run with --yes to kill them.")
        return 0

    killed = 0
    for pid, _ in doomed:
        try:
            # SIGTERM, not SIGKILL. These hold an open SQLite database, and a
            # daemon given the chance to close it cleanly is worth the wait.
            os.kill(pid, 15)
            killed += 1
        except ProcessLookupError:
            pass
        except PermissionError:
            print(f"  cannot signal {pid}", file=sys.stderr)
    print(f"\nsignalled {killed} daemon(s).")
    return 0


if __name__ == "__main__":
    sys.exit(main())
