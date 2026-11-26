#!/usr/bin/env python3
"""Exercise the activation path with real launchd, daemons, tmux and an isolated home.

Run as a Mac user without a loaded standalone canary job. Pass an app bundle
for the older executables and the Cargo profile directory for the new ones.
The fixture replaces the ServiceManagement probe with its expected response.
The installer and updater use a file feed signed with the public test key.
"""

import argparse
import fcntl
import json
import os
import pathlib
import plistlib
import runpy
import select
import signal
import shutil
import subprocess
import tempfile
import time
import uuid

ROOT = pathlib.Path(__file__).resolve().parents[1]
JOB = f"user/{os.getuid()}/com.farcooler.daemon.canary.standalone"
GUI = f"gui/{os.getuid()}"
BOOTSTRAP_LABEL = "com.farcooler.daemon.canary.bootstrap"
RUNTIME = "Library/Application Support/com.farcooler.FarCoolerCanary"
APP_RESOURCES = "Contents/Resources"


def run(*args, env=None, check=True):
    result = subprocess.run(args, env=env, capture_output=True, text=True, timeout=75)
    if check and result.returncode:
        raise RuntimeError(f"{args[0]} exited {result.returncode}: {result.stderr}")
    return result


def publish(feed, build, profile):
    feed.publish(build, profile / "farcooler", profile / "farcoolerd", profile / "farcooler-lfs-filter")


def fixture_app(app, home):
    bundle = home / "Far Cooler Canary.app"
    resources = bundle / APP_RESOURCES
    resources.mkdir(parents=True)
    shutil.copy(app / "Contents/Info.plist", bundle / "Contents")
    for name in ("farcoolerd", "farcooler"):
        shutil.copy(app / APP_RESOURCES / name, resources / name)
        # A fixture home does not participate in the production notarization flow.
        run("/usr/bin/codesign", "-f", "-s", "-", resources / name)
    probe = bundle / "Contents/MacOS/FarCooler"
    probe.parent.mkdir()
    probe.write_text('#!/bin/sh\n[ "$FARCOOLER_SERVICE_PROBE" = unregister ] || exit 1\nprintf "state:  notRegistered\\n"\n')
    probe.chmod(0o700)
    return bundle


def environment(home):
    home.mkdir()
    return dict(HOME=str(home), PATH="/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin",
                USER=os.environ["USER"], LOGNAME=os.environ["USER"])


def unload(env):
    run("/bin/launchctl", "bootout", JOB, env=env, check=False)
    deadline = time.monotonic() + 10
    while run("/bin/launchctl", "print", JOB, env=env, check=False).returncode == 0:
        if time.monotonic() >= deadline:
            raise RuntimeError(f"{JOB} did not unload")
        time.sleep(0.1)


def unload_login(env):
    run("/bin/launchctl", "bootout", f"{GUI}/{BOOTSTRAP_LABEL}", env=env, check=False)


def load_older_template(env, plist, cli):
    older = plistlib.loads(plist.read_bytes())
    older["EnvironmentVariables"]["LEGACY_FIXTURE"] = "old-template"
    old_bytes = plistlib.dumps(older, fmt=plistlib.FMT_BINARY)
    unload(env)
    plist.write_bytes(old_bytes)
    run("/bin/launchctl", "bootstrap", f"user/{os.getuid()}", plist, env=env)
    run(cli, "status", env=env)
    return old_bytes


def clean_install(home, feed, profile):
    env = environment(home)
    try:
        publish(feed, 1, profile)
        result = run("/bin/bash", feed.installer, env=env)
        assert "is running" in result.stdout, result.stdout
        assert (home / RUNTIME / "install-id").is_file()
        assert (home / ".local/bin/farcoolerd-canary").is_file()
        print("Clean HOME: launchd started the daemon and the CLI/daemon links resolve.", flush=True)

        if run("/bin/launchctl", "print", GUI, env=env, check=False).returncode == 0:
            unload_login(env)
            unload(env)
            plist = home / f"Library/LaunchAgents/{BOOTSTRAP_LABEL}.plist"
            run("/bin/launchctl", "bootstrap", GUI, plist, env=env)
            end = time.monotonic() + 15
            while True:
                info = run("/bin/launchctl", "print", JOB, env=env, check=False)
                if info.returncode == 0 and "state = running" in info.stdout:
                    break
                if time.monotonic() >= end:
                    raise RuntimeError("The login LaunchAgent did not bootstrap the runner")
                time.sleep(0.1)
            print("Login bootstrap: the Aqua LaunchAgent restored the background job.", flush=True)
    finally:
        unload_login(env)
        unload(env)


def preserved_state(home, app, feed, profile):
    env = environment(home)
    app = fixture_app(app, home)
    resources = app / APP_RESOURCES
    previous = resources / "farcooler"
    new = home / ".local/bin/farcooler-canary"
    new.parent.mkdir(parents=True)
    new.symlink_to(previous)
    new.with_name("farcoolerd-canary").symlink_to(resources / "farcoolerd")
    runtime = home / RUNTIME
    log = home / "app-runner.log"
    repo = home / "repo"
    ssh_key = home / "fixture-key"
    server = None

    with log.open("wb") as output:
        old = subprocess.Popen([resources / "farcoolerd"], env=env, stdout=output, stderr=output)
        try:
            end = time.monotonic() + 20
            while not (runtime / "farcoolerd.sock").exists():
                if old.poll() is not None or time.monotonic() >= end:
                    raise RuntimeError(log.read_text())
                time.sleep(0.1)
            # Resolve git roots physically because macOS standardizes /tmp.
            repo.mkdir()
            run("git", "-C", repo, "init", "-b", "main", env=env)
            run("git", "-C", repo, "-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
                "commit", "--allow-empty", "-m", "initial", env=env)
            run(previous, "root", "add", home.resolve(), env=env)
            run(previous, "repo", "register", repo.resolve(), env=env)
            run(previous, "workspace", "create", "repo", "--name", "Keep", "--prefix", "keep", env=env)
            worktree = json.loads(run(previous, "--json", "worktree", "create", "repo", "keep-tree",
                                      "--branch", "keep-tree", "--workspace", "Keep", env=env).stdout)
            run("/usr/bin/ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-f", ssh_key, env=env)
            ssh_dir = home / ".ssh"
            ssh_dir.mkdir(mode=0o700, exist_ok=True)
            authorized = ssh_dir / "authorized_keys"
            key = " ".join(ssh_key.with_suffix(".pub").read_text().split()[:2])
            authorized.write_text(
                "# BEGIN FAR COOLER — do not edit inside this block\n"
                'restrict,command="~/.local/bin/farcoolerd-canary --stdio --client fixture-device --scope read" '
                f"{key} farcooler-Fixture-t7xq9vd8\n"
                "# END FAR COOLER\n"
            )
            authorized.chmod(0o600)
            assert "fixture-device" in run(previous, "--json", "client", "list", env=env).stdout
            install_id = (runtime / "install-id").read_text().strip()
            server = f"farcooler-{install_id}"
            keys = (home / ".ssh/authorized_keys").read_bytes()
            run(previous, "terminal", "create", worktree["id"], "--preset", "shell", env=env)

            def panes():
                return run("tmux", "-L", server, "list-panes", "-a", "-F", "#{pane_pid}:#{pane_id}", env=env).stdout

            def assert_state(expected_panes):
                assert (runtime / "install-id").read_text().strip() == install_id
                assert (home / ".ssh/authorized_keys").read_bytes() == keys
                assert "fixture-device" in run(new, "--json", "client", "list", env=env).stdout
                assert "Keep" in run(new, "--json", "workspace", "list", env=env).stdout
                assert worktree["id"] in run(new, "--json", "worktree", "list", env=env).stdout
                assert panes() == expected_panes

            existing_panes = panes()
            assert existing_panes
            publish(feed, 900000, profile)
            run("/bin/bash", feed.installer, "--migrate-app", app, env=env)
            assert old.wait(timeout=10) == 0
            assert_state(existing_panes)
            print("Migration fixture: workspace, worktree, tmux pane PID, install ID and enrollment survived.", flush=True)

            publish(feed, 900001, profile)
            # Load an older template so the test checks both the on-disk
            # replacement and launchd's in-memory configuration.
            plist = home / "Library/LaunchAgents/com.farcooler.daemon.canary.standalone.plist"
            original_plist = plist.read_bytes()
            load_older_template(env, plist, new)
            job = run("/bin/launchctl", "print", JOB, env=env).stdout
            assert "LEGACY_FIXTURE" in job
            old_pid = next(line.strip() for line in job.splitlines() if line.strip().startswith("pid = "))
            with (runtime / "landing-update.lock").open("r+b") as landing:
                fcntl.flock(landing, fcntl.LOCK_SH)
                refused = run(new, "runner", "update", env=env, check=False)
                assert refused.returncode and "--wait" in refused.stderr, refused
                assert_state(existing_panes)
                with subprocess.Popen([new, "runner", "update", "--wait"], env=env, text=True, start_new_session=True,
                                      stdout=subprocess.PIPE, stderr=subprocess.PIPE) as update:
                    try:
                        assert select.select([update.stderr], [], [], 20)[0], "update did not announce the wait"
                        assert "Waiting for in-flight" in update.stderr.readline()
                        assert update.poll() is None
                        fcntl.flock(landing, fcntl.LOCK_UN)
                        stdout, stderr = update.communicate(timeout=40)
                        assert update.returncode == 0 and "build 900001 is running" in stdout, stderr
                    finally:
                        if update.poll() is None:
                            os.killpg(update.pid, signal.SIGKILL)
            assert_state(existing_panes)
            job = run("/bin/launchctl", "print", JOB, env=env).stdout
            assert old_pid not in {line.strip() for line in job.splitlines()}
            assert "LEGACY_FIXTURE" not in job
            assert plist.read_bytes() == original_plist
            current = home / ".local/share/farcooler-runner-canary/current"
            previous = current.readlink()
            assert previous.name.startswith("900001-")
            print("Update fixture: signed CLI update waited for the landing, reloaded the old LaunchAgent template and preserved state.", flush=True)

            # A correctly signed payload whose daemon exits before it can
            # answer must leave the previous release serving the same state.
            previous_plist = load_older_template(env, plist, new)
            feed.publish(900002, profile / "farcooler", pathlib.Path("/usr/bin/false"),
                         profile / "farcooler-lfs-filter")
            failed = run(new, "runner", "update", env=env, check=False)
            assert failed.returncode and "Previous release restored" in failed.stderr, failed
            assert current.readlink() == previous
            assert plist.read_bytes() == previous_plist
            assert "LEGACY_FIXTURE" in run("/bin/launchctl", "print", JOB, env=env).stdout
            assert_state(existing_panes)
            print("Rollback fixture: a signed failing daemon left the previous release serving the same state.", flush=True)
        finally:
            unload_login(env)
            unload(env)
            if old.poll() is None:
                old.terminate()
                old.wait(timeout=10)
            if server is None and (runtime / "install-id").is_file():
                server = f"farcooler-{(runtime / 'install-id').read_text().strip()}"
            if server:
                run("tmux", "-L", server, "kill-server", env=env, check=False)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("app", type=pathlib.Path)
    parser.add_argument("profile", type=pathlib.Path)
    args = parser.parse_args()
    if os.getuid() == 0 or any(run("/bin/launchctl", "print", service, check=False).returncode == 0
                              for service in (JOB, f"{GUI}/{BOOTSTRAP_LABEL}")):
        raise SystemExit("Run as a regular Mac user with both standalone launchd jobs unloaded.")
    with tempfile.TemporaryDirectory(dir=os.environ.get("FARCOOLER_SCRATCH")) as temp:
        feed = runpy.run_path(str(ROOT / "scripts/mac-runner-package-test.py"))["FixtureFeed"](pathlib.Path(temp))
        # Only the alias is outside scratch. sun_path cannot fit the full
        # runner socket name beneath the engine's scratch pathname.
        short = pathlib.Path("/tmp") / ("f" + uuid.uuid4().hex[:6])
        short.symlink_to(temp, target_is_directory=True)
        try:
            clean_install(short / "c", feed, args.profile.resolve())
            preserved_state(short / "m", args.app.resolve(), feed, args.profile.resolve())
        finally:
            short.unlink()


if __name__ == "__main__":
    main()
