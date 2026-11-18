#!/usr/bin/env python3
"""Disposable Mac endpoint for RelayConnectionDeviceTest; no account or relay writes."""
import argparse
import base64
import getpass
import json
import os
from pathlib import Path
import shlex
import signal
import socket
import subprocess
import time

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--directory", type=Path, required=True, help="new private scratch directory")
parser.add_argument("--helper", type=Path, required=True, help="CGO_ENABLED=0 Go tunnel helper")
parser.add_argument("--daemon", type=Path, required=True, help="installed or built farcoolerd")
parser.add_argument("--offer", type=Path, help="optional emulator relay-offer.json (public keys only)")
parser.add_argument("--openssl", default="openssl", help="OpenSSL with X25519 support")
args = parser.parse_args()
os.umask(0o077)
root = args.directory.resolve()
root.mkdir(mode=0o700)


def output(*argv, data=None):
    return subprocess.check_output(argv, input=data)


for name in ("host", "phone"):
    output("ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-f", str(root / name))
seed = output(args.openssl, "genpkey", "-algorithm", "X25519", "-outform", "DER")
public = output(args.openssl, "pkey", "-inform", "DER", "-pubout", "-outform", "DER", data=seed)
private_key = base64.urlsafe_b64encode(seed[-32:]).decode().rstrip("=")
node_public = base64.urlsafe_b64encode(public[-32:]).decode().rstrip("=")
allowed = [node_public]
ssh_public = (root / "phone.pub").read_text()
if args.offer:
    offer = json.loads(args.offer.read_text())
    allowed.append(offer["node_public"])
    ssh_public += offer["ssh_public"].rstrip() + "\n"
(root / "authorized_keys").write_text(ssh_public)
wrapper = root / "daemon"
wrapper.write_text(
    "#!/bin/sh\n"
    f"export FARCOOLER_HOME={shlex.quote(str(root / 'runtime'))}\n"
    "export PATH=/opt/homebrew/bin:/usr/bin:/bin\n"
    f"exec {shlex.quote(str(args.daemon.resolve()))} --stdio\n"
)
wrapper.chmod(0o700)
with socket.socket() as sock:
    sock.bind(("127.0.0.1", 0))
    port = sock.getsockname()[1]
config = root / "sshd_config"
config.write_text(
    f"Port {port}\nListenAddress 127.0.0.1\nHostKey {root}/host\n"
    f"AuthorizedKeysFile {root}/authorized_keys\nPidFile {root}/sshd.pid\n"
    "StrictModes no\nUsePAM no\nPasswordAuthentication no\n"
    f"KbdInteractiveAuthentication no\nForceCommand {wrapper}\nLogLevel DEBUG1\n"
)
subprocess.run(["/usr/sbin/sshd", "-t", "-f", str(config)], check=True)
sshd = subprocess.Popen(["/usr/sbin/sshd", "-D", "-f", str(config), "-E", str(root / "sshd.log")])
helper = None


def stop(_signal, _frame):
    raise KeyboardInterrupt


signal.signal(signal.SIGTERM, stop)
try:
    with (root / "tailcat.log").open("w") as log:
        helper = subprocess.Popen(
            [str(args.helper.resolve()), "-key", str(root / "tailcat.key")],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=log, text=True,
        )

        def command(line):
            helper.stdin.write(line + "\n")
            helper.stdin.flush()
            answer = helper.stdout.readline().strip()
            if not answer.startswith("ok"):
                raise RuntimeError(f"{line.split()[0]}: {answer}; see tailcat.log")
            return answer[3:]

        command("identity")
        command(f"serve {port} {' '.join(allowed)}")
        token = command("blob")
        fingerprint = output("ssh-keygen", "-lf", str(root / "host.pub")).decode().split()[1]
        (root / "config.json").write_text(json.dumps({
            "user": getpass.getuser(), "private_key": (root / "phone").read_text(),
            "token": token, "node_key": private_key,
            "host_fingerprint": fingerprint, "derp_map": "",
        }))
        print(f"Ready: {root}/config.json (throwaway private keys; do not commit)", flush=True)
        while True:
            time.sleep(1)
except KeyboardInterrupt:
    pass
finally:
    for child in (helper, sshd):
        if child is not None:
            child.terminate()
            try:
                child.wait(timeout=5)
            except subprocess.TimeoutExpired:
                child.kill()
                child.wait()
