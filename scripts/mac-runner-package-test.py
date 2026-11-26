#!/usr/bin/env python3
"""Exercise the shipped bootstrap with an isolated home and a fixture signer."""

import base64
import hashlib
import os
import pathlib
import platform
import subprocess
import tarfile
import tempfile

ROOT = pathlib.Path(__file__).resolve().parents[1]
# RFC 8032 test vector. These are public test values.
SEED = base64.b64encode(bytes.fromhex("9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60"))
PUBLIC = bytes.fromhex("d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a")
SIGNER_PREFIX = "canary ssh-ed25519 "


def run(*args, **kwargs):
    return subprocess.run(args, check=True, **kwargs)


def ssh_key(raw):
    return base64.b64encode(b"\x00\x00\x00\x0bssh-ed25519\x00\x00\x00\x20" + raw).decode()


class FixtureFeed:
    def __init__(self, temp):
        self.arch = platform.machine()
        if subprocess.run(["/usr/sbin/sysctl", "-n", "hw.optional.arm64"],
                          capture_output=True, text=True).stdout.strip() == "1":
            self.arch = "arm64"
        self.directory = temp / "feed" / self.arch
        self.directory.mkdir(parents=True)
        self.latest = self.directory / "latest"
        self.manifest = temp / "manifest"
        self.signer = temp / "sign-manifest"
        run("/usr/bin/swiftc", str(ROOT / "scripts/sign-mac-runner-manifest.swift"), "-o", str(self.signer))
        bootstrap = (ROOT / "scripts/install-mac-runner.sh").read_text()
        expected = next(line.split()[1] for line in (ROOT / "apps/macos/sparkle-public-keys.txt")
                        .read_text().splitlines() if line.startswith("canary "))
        production_signer = SIGNER_PREFIX + ssh_key(base64.b64decode(expected))
        assert production_signer in bootstrap, "the installer must pin the canary public key"
        self.installer = temp / "install-mac-runner.sh"
        self.installer.write_text(
            bootstrap.replace(production_signer, SIGNER_PREFIX + ssh_key(PUBLIC))
            .replace("https://updates.farcooler.com/canary/runner", self.directory.parent.as_uri())
            .replace("--proto '=https'", "--proto '=file'")
        )

    def sign(self, build, digest, channel="canary"):
        self.manifest.write_text(f"runner-v1 {channel} {self.arch} {build} {digest}\n")
        return run(str(self.signer), str(self.manifest), base64.b64encode(PUBLIC).decode(), input=SEED,
                   capture_output=True).stdout

    def publish(self, build, cli, daemon, lfs_filter):
        archive = self.directory / "runner.tar.gz"
        with tarfile.open(archive, "w:gz") as tar:
            for path, name in (
                (cli, "farcooler-canary"),
                (daemon, "farcoolerd-canary"),
                (lfs_filter, "farcooler-lfs-filter"),
                (self.installer, "install-mac-runner.sh"),
            ):
                tar.add(path, arcname=name)
        digest = hashlib.sha256(archive.read_bytes()).hexdigest()
        archive = archive.rename(self.directory / f"{build}-{digest}.tar.gz")
        self.latest.write_bytes(self.sign(build, digest))
        return archive, digest


def exercise(temp):
    archive_files = temp / "files"
    archive_files.mkdir()
    source = archive_files / "activate.c"
    source.write_text('#include <stdio.h>\nint main(void) { puts("verified payload activated"); }\n')
    cli = archive_files / "farcooler-canary"
    run("/usr/bin/cc", str(source), "-o", str(cli))
    run("/usr/bin/codesign", "-f", "-s", "-", str(cli))
    feed = FixtureFeed(temp)
    archive, digest = feed.publish(41, cli, cli, cli)
    home = temp / "home"
    bin_dir = home / ".local/bin"
    bin_dir.mkdir(parents=True)
    (bin_dir / "tmux").symlink_to("/usr/bin/true")
    env = dict(os.environ, HOME=str(home))
    env.pop("FARCOOLER_HOME", None)

    def install(okay, message):
        cmd = subprocess.run(["/bin/bash", str(feed.installer)], env=env, capture_output=True, text=True)
        assert (cmd.returncode == 0) == okay, cmd
        assert message in cmd.stdout + cmd.stderr, cmd
        assert ("verified payload activated" in cmd.stdout) == okay, cmd

    latest = feed.latest
    original = latest.read_bytes()
    latest.write_bytes(original.replace(b" 41 ", b" 42 ", 1))
    install(False, "manifest signature failed")
    latest.write_bytes(feed.sign(41, digest, "stable"))
    install(False, "does not describe a canary build")
    latest.write_bytes(original)
    payload = archive.read_bytes()
    archive.write_bytes(payload + b"tampered")
    install(False, "archive checksum failed")
    archive.write_bytes(payload)
    install(True, "verified payload activated")
    (home / ".local/share/farcooler-runner-canary/current").symlink_to("releases/42-fixture")
    install(False, "older than the installed runner")


if __name__ == "__main__":
    with tempfile.TemporaryDirectory(prefix="mac-runner-package-test-") as temp:
        exercise(pathlib.Path(temp))
    print("PASS: signed manifest, channel, archive digest, activation and downgrade refusal")
