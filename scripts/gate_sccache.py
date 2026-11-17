#!/usr/bin/env python3
"""Opt-in, gate-owned local compiler cache (ov-1333).

Provision with `python3 scripts/gate_sccache.py --install` from the trusted
gate account, outside builder sandboxes. Never grant builders this root.
Builders must be sandboxed to their worktrees and temporary directories;
0700 alone does not isolate processes running as the same user.
"""
import argparse
import contextlib
import fcntl
import os
from pathlib import Path
import socket
import stat
import subprocess
import time

SWITCH = "FARCOOLER_GATE_SCCACHE"
ROOT = Path.home() / ".cache/farcooler-gate-sccache"
VERSION = "0.19.0"


def clear_stale_socket(path):
    """Under session.lock, remove only a socket with no listening server."""
    if not path.exists():
        return
    if not stat.S_ISSOCK(path.lstat().st_mode):
        raise ValueError(f"sccache: unexpected server socket path: {path}")
    with socket.socket(socket.AF_UNIX) as probe:
        probe.settimeout(1)
        try:
            probe.connect(str(path))
        except ConnectionRefusedError:
            path.unlink()
            return
    raise ValueError(f"sccache: existing server socket; inspect before retrying: {path}")


def private(path):
    """Fail closed on symlinks, foreign owners and non-private state."""
    if path.resolve() != path:
        raise ValueError(f"sccache: symlink in gate path: {path}")
    info = path.stat()
    if info.st_uid != os.getuid() or stat.S_IMODE(info.st_mode) != 0o700:
        raise ValueError(f"sccache: gate directory must be owned by this user and mode 0700: {path}")


def environment():
    """Empty unless explicitly enabled; never change Cargo's target directory."""
    if os.environ.get(SWITCH) != "1":
        return {}
    for path in (ROOT, ROOT / "tools", ROOT / "cache", ROOT / "tmp"):
        private(path)
    binary = ROOT / "tools/bin/sccache"
    if binary.resolve() != binary or not binary.is_file() or not os.access(binary, os.X_OK):
        raise ValueError("sccache: provision the gate tool with python3 scripts/gate_sccache.py --install")
    return {"RUSTC_WRAPPER": str(binary), "RUSTC_WORKSPACE_WRAPPER": "",
            "CARGO_INCREMENTAL": "0", "CARGO_PROFILE_DEV_INCREMENTAL": "false",
            "CARGO_PROFILE_TEST_INCREMENTAL": "false", "CARGO_PROFILE_RELEASE_INCREMENTAL": "false",
            "SCCACHE_DIR": str(ROOT / "cache"), "SCCACHE_CACHE_SIZE": "20G",
            "SCCACHE_SERVER_UDS": str(ROOT / "server.sock"),
            "SCCACHE_CONF": str(ROOT / "config"), "SCCACHE_CACHED_CONF": str(ROOT / "cached-config"),
            "SCCACHE_IDLE_TIMEOUT": "0", "SCCACHE_NO_DAEMON": "1"}


@contextlib.contextmanager
def session(logdir):
    """One foreground-owned server, serialized even for --no-lock gates."""
    settings = environment()
    if not settings:
        yield
        return
    # Strip remote backends and caller server controls. The installed binary
    # has no remote-storage features; the empty config rules out user config.
    env = {k: v for k, v in os.environ.items() if not k.startswith("SCCACHE_")}
    env.update(settings)
    env["TMPDIR"] = str(ROOT / "tmp")
    binary = settings["RUSTC_WRAPPER"]
    with open(ROOT / "session.lock", "a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        (ROOT / "config").write_text("")
        socket = ROOT / "server.sock"
        clear_stale_socket(socket)
        with open(logdir / "sccache-server.log", "w") as output:
            server = subprocess.Popen([binary], env={**env, "SCCACHE_START_SERVER": "1"},
                                      stdout=output, stderr=subprocess.STDOUT)
            try:
                deadline = time.monotonic() + 15
                while not socket.exists():
                    if server.poll() is not None or time.monotonic() >= deadline:
                        raise ValueError(f"sccache: server did not start; see {logdir}/sccache-server.log")
                    time.sleep(0.05)
                subprocess.run([binary, "--zero-stats"], env=env, check=True, timeout=15,
                               stdout=output, stderr=subprocess.STDOUT)
                yield
            finally:
                try:
                    if socket.exists() and server.poll() is None:
                        with open(logdir / "sccache-stats.txt", "w") as stats:
                            subprocess.run([binary, "--show-stats"], env=env, check=True,
                                           timeout=15, stdout=stats, stderr=subprocess.STDOUT)
                        subprocess.run([binary, "--stop-server"], env=env, timeout=15,
                                       stdout=output, stderr=subprocess.STDOUT)
                finally:
                    if server.poll() is None:
                        server.terminate()
                    try:
                        server.wait(timeout=15)
                    except subprocess.TimeoutExpired:
                        server.kill()
                        server.wait()
                    clear_stale_socket(socket)
                    with open(logdir / "sccache-disk.txt", "w") as disk:
                        subprocess.run(["du", "-sk", str(ROOT / "cache")], check=True, stdout=disk)


def install():
    """All downloads, Cargo state, build output and tools stay gate-owned."""
    ROOT.mkdir(mode=0o700, parents=True, exist_ok=True)
    private(ROOT)
    for name in ("tools", "cache", "cargo", "build", "tmp"):
        path = ROOT / name
        path.mkdir(mode=0o700, exist_ok=True)
        private(path)
    env = {k: v for k, v in os.environ.items()
           if not k.startswith(("SCCACHE_", "RUSTC_", "CARGO_"))}
    env.update(CARGO_HOME=str(ROOT / "cargo"), CARGO_TARGET_DIR=str(ROOT / "build"),
               TMPDIR=str(ROOT / "tmp"), CARGO_BUILD_JOBS="4")
    subprocess.run(["cargo", "install", "sccache", "--locked", "--version", VERSION,
                    "--no-default-features", "--root", str(ROOT / "tools")], env=env, check=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--install", action="store_true", required=True)
    parser.parse_args()
    install()
