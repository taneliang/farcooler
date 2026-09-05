# Runners

**A runner is one `farcoolerd`:** one Unix user, on one host, with its own
worktrees, its own `~/.ssh/authorized_keys` and its own `systemd --user` service.

A host may carry several. Three engineers sharing a Linux box is three runners —
three fences, three key sets, three sets of worktrees, nothing shared — because
Far Cooler has no concept above the Unix user. That is the distinction this
document is about, and it is why the word is *runner* rather than *host*: a host
is the box, and there can be more than one runner on it.

Far Cooler drives runners over SSH. There is no Far Cooler network protocol, no
port to open on the runner, and no second set of credentials.
`farcooler --runner you@box workspace list` runs `ssh you@box farcoolerd --stdio`
and speaks the same protocol it speaks over a local Unix socket. (`--host` still
works: it lives in shell history and in scripts, and a vocabulary change is not a
reason to break either.)

Three consequences worth understanding before anything else:

- **A runner reachable by SSH is reachable by `--runner`, and one that is not, is
  not.** If you need to reach a host behind NAT from the command line, use
  Tailscale, a bastion, or an SSH config alias. Far Cooler does not reimplement
  any of that.

  A runner can also serve a **tunnel**: a connection it makes outward and holds
  open, which a device that runner admits dials to reach that runner's own sshd.
  Nothing is opened on the runner and no address is needed, so a box behind NAT
  is reachable without any of the three above. It carries ordinary SSH inside, so
  it changes how a device arrives and nothing about who is let in — and it has
  its own consequences, in [Security posture](#security-posture).

  A runner serves a tunnel only once a device enrolled on it has registered a
  node key with it, and **no client does that yet** — every device Far Cooler
  can enroll today reaches its runners by address. The runner half is built and
  the route is real; what is not built is the step where a device obtains a node
  key of its own. It is described here rather than when that lands, because what
  it costs is worth reading before somebody turns it on. Either way, `--runner`
  is SSH: this is the apps' route, not the command line's.
- **Everything installs into your home directory.** No root, no package manager,
  no system service. A user who can SSH in can install Far Cooler; a user who
  cannot, cannot. This is also why runners are per-user rather than per-host.
- **Every name below carries the channel.** A release build installs and reaches
  for `farcoolerd`; a canary one installs and reaches for `farcoolerd-canary`,
  and likewise `-preview` and `-local`. So one runner can carry several channels
  at once without them meeting, and a build only ever talks to the daemon it put
  there. See [releasing.md](releasing.md).

## What a Linux host needs

- **tmux.** Far Cooler keeps every terminal inside one, and that is what keeps
  your agents alive across disconnects. `runner install` refuses to proceed
  without it rather than installing successfully and being mysteriously broken.
- **git**, for worktrees.
- **systemd with user sessions**, for the daemon to survive logout. Almost every
  modern distribution has this.

```bash
ssh you@box 'sudo apt-get install -y tmux git'    # Debian, Ubuntu
ssh you@box 'sudo dnf install -y tmux git'        # Fedora, RHEL
ssh you@box 'sudo apk add tmux git'               # Alpine
```

## Installing

Build the Linux binaries, then push them:

```bash
./scripts/build-linux.sh x86_64      # or aarch64
farcooler runner install you@box
```

The binaries are static musl builds. A glibc build made on one distribution
refuses to start on an older one, and `GLIBC_2.38 not found` is the single most
common way a self-installed daemon fails. musl links everything in, so one
binary runs on Debian, Ubuntu, Alpine, Arch and a NAS alike.

If you cannot cross-compile, build on the runner itself and point the installer at
the result:

```bash
ssh you@box 'git clone <repo> farcooler && cd farcooler && cargo build --release'
farcooler runner install you@box --from /path/to/checkout/target/release
```

### What `runner install` actually does

1. Checks `uname`: Linux only. A **macOS** host is set up by running the
   Far Cooler app there once — registering its LaunchAgent needs a GUI session,
   so there is no headless path.
2. Checks for tmux, and stops with instructions if it is missing.
3. Copies `farcoolerd` and `farcooler` to `~/.local/bin/`, each as `.new` first.
4. **Verifies the SHA-256 on the host before anything runs.** The transfer is
   already inside SSH, so this is not about the wire — it is about a truncated
   copy producing a half-written executable that then gets registered as a
   service. A mismatch leaves the bad file with its `.new` suffix, off the path,
   as evidence.
5. Writes `~/.config/systemd/user/farcooler.service`.
6. Runs `loginctl enable-linger`, so the daemon survives logout and reboot.
   Without lingering, systemd tears down your services the moment you log out —
   which is exactly the situation Far Cooler exists for. If lingering cannot be
   enabled, the installer says so plainly rather than reporting a success that
   will not survive the night.
7. Enables and starts the service.

## Using a runner

Every command takes `--runner`:

```bash
farcooler --runner you@box status
farcooler --runner you@box repo register ~/code/api
farcooler --runner you@box workspace create api "add auth" --branch feat/auth
farcooler --runner you@box terminal create <workspace> --preset claude
farcooler --runner you@box workspace list
```

Live terminal commands work too, and go over their own SSH session rather than
through the protocol — SSH is a byte pipe and so are they:

```bash
farcooler --runner you@box terminal stream <terminal>
farcooler --runner you@box terminal send <terminal> 'ls -la\n'
```

Connections are multiplexed (`ControlMaster`), so a burst of commands does not
mean a burst of key exchanges. `BatchMode=yes` is always set: a host that wants
a password, or whose key has changed, fails immediately with SSH's own message
instead of hanging forever waiting for input a GUI client will never provide.

## Checking a runner

```bash
farcooler runner status you@box
```

```
you@box
  os        Linux
  arch      x86_64
  tmux      /usr/bin/tmux
  daemon    farcoolerd 0.1.0
  cli       farcooler 0.1.0
  service   active
  linger    yes
```

`linger yes` is the one to check. Without it the runner stops answering when you
log out of it.

## Troubleshooting

**"`farcoolerd --stdio` did not answer"** — the binary is not on the remote
user's `PATH`. `~/.local/bin` is added by the login shell, so check that the
runner's shell profile includes it, or re-run `runner install`. The message names
the binary this build actually asked for, so a canary client reports
`farcoolerd-canary` — a release daemon being present on that runner is not the
same thing as this one being installed.

**"runs protocol N; this client speaks M"** — the two sides are different
versions. Re-run `runner install` to bring the runner up to date, or update the
client.

**Everything derives `LOST`** — tmux is not answering on the runner. `farcooler
runner status` will show whether tmux is present; if the daemon is running but
tmux was killed, every terminal is honestly reported as lost rather than
guessed at.

**The runner goes away when you log out** — lingering is off. Fix with
`ssh you@box 'sudo loginctl enable-linger $USER'`.

## Security posture

SSH is the only control-plane transport and the only authentication mechanism.
Far Cooler does not run a certificate authority, issue client certificates, pin
its own identity key, or operate a pairing flow. OpenSSH already authenticates
both directions, and the asset being protected is a shell that SSH gates anyway.

A remote client is granted `host_admin`, the same as a local one, because it has
already proved it is the Unix user who owns that runner's database and worktrees —
someone who could read them directly regardless. The daemon runs as that user
and never as root.

The tunnel does not change any of that: what arrives down it is handed to the
sshd already on the host, which authenticates it exactly as it authenticates a
connection from the open internet. What the tunnel changes is who can *reach*
that sshd, and the three paragraphs below are what that costs.

**One tunnel entrance per box, not per runner.** A runner's tunnel forwards to
the sshd on its host, and a host may carry several runners. So one runner's
token plus one admitted node key opens a TCP connection to the sshd that fronts
every runner there. Nothing is granted by this: each user's `authorized_keys`
still gates their own account, and a device holding no key on your account gets
an authentication failure exactly as it would from the open internet. But "three
engineers on one box is three runners sharing nothing" now has one shared thing,
and it is this.

**A runner cannot check that a device owns the node key it registers.** A device
tells its runner which key to admit, and the runner writes that key onto the
device's own line — which is what stops a device putting a key on somebody
else's line or widening its own access. It is not what stops a device sending a
key it does not hold the private half of: the tunnel admits by key, so an
enrolled device can extend tunnel reachability to a machine that is not enrolled
at all. That machine gets a TCP connection to your host's sshd and nothing more:
holding no key on any account there, it fails authentication exactly as it would
from the open internet. Asking devices to prove possession
would not close it — a device that wants to admit a machine controls that
machine, and could prove whatever was asked — so what bounds it is revoking the
device, which removes its line and its node key in the same write. Read it as
one more reason enrollment is a decision about a device rather than about a
scope.

**Revoking one device drops every device's tunnel, when it takes effect.** The
line goes immediately, so the revoked device cannot log in from that moment, and
the sessions it was holding are closed. Its tunnel *route* outlives that: the
tunnel is handed the list of admitted keys when it starts, one key cannot be
subtracted from a running tunnel, and the only withdrawal is starting a fresh
one — which drops every other device's live tunnel with it. Today that happens
when the runner's daemon next starts, so a revoked device keeps a path to an
sshd that now refuses it until then. If you want the path gone as well as the
login, restart the daemon: `ssh you@box 'systemctl --user restart farcooler'`.
