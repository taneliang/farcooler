# Remote hosts

Overnight drives other machines over SSH. There is no Overnight network
protocol, no port to open, no pairing flow, and no second set of credentials.
`overnight --host you@box workspace list` runs `ssh you@box overnightd --stdio`
and speaks the same protocol it speaks over a local Unix socket.

Two consequences worth understanding before anything else:

- **A host reachable by SSH is reachable by Overnight, and one that is not, is
  not.** If you need to reach a machine behind NAT, use Tailscale, a bastion, or
  an SSH config alias. Overnight does not reimplement any of that.
- **Everything installs into your home directory.** No root, no package manager,
  no system service. A user who can SSH in can install Overnight; a user who
  cannot, cannot.

## What a Linux host needs

- **tmux.** Overnight keeps every terminal inside one, and that is what keeps
  your agents alive across disconnects. `host install` refuses to proceed
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
overnight host install you@box
```

The binaries are static musl builds. A glibc build made on one distribution
refuses to start on an older one, and `GLIBC_2.38 not found` is the single most
common way a self-installed daemon fails. musl links everything in, so one
binary runs on Debian, Ubuntu, Alpine, Arch and a NAS alike.

If you cannot cross-compile, build on the host itself and point the installer at
the result:

```bash
ssh you@box 'git clone <repo> overnight && cd overnight && cargo build --release'
overnight host install you@box --from /path/to/checkout/target/release
```

### What `host install` actually does

1. Checks `uname`: Linux only. A **macOS** host is set up by running the
   Overnight app there once — registering its LaunchAgent needs a GUI session,
   so there is no headless path.
2. Checks for tmux, and stops with instructions if it is missing.
3. Copies `overnightd` and `overnight` to `~/.local/bin/`, each as `.new` first.
4. **Verifies the SHA-256 on the host before anything runs.** The transfer is
   already inside SSH, so this is not about the wire — it is about a truncated
   copy producing a half-written executable that then gets registered as a
   service. A mismatch leaves the bad file with its `.new` suffix, off the path,
   as evidence.
5. Writes `~/.config/systemd/user/overnight.service`.
6. Runs `loginctl enable-linger`, so the daemon survives logout and reboot.
   Without lingering, systemd tears down your services the moment you log out —
   which is exactly the situation Overnight exists for. If lingering cannot be
   enabled, the installer says so plainly rather than reporting a success that
   will not survive the night.
7. Enables and starts the service.

## Using a host

Every command takes `--host`:

```bash
overnight --host you@box status
overnight --host you@box repo register ~/code/api
overnight --host you@box workspace create api "add auth" --branch feat/auth
overnight --host you@box terminal create <workspace> --preset claude
overnight --host you@box workspace list
```

Live terminal commands work too, and go over their own SSH session rather than
through the protocol — SSH is a byte pipe and so are they:

```bash
overnight --host you@box terminal stream <terminal>
overnight --host you@box terminal send <terminal> 'ls -la\n'
```

Connections are multiplexed (`ControlMaster`), so a burst of commands does not
mean a burst of key exchanges. `BatchMode=yes` is always set: a host that wants
a password, or whose key has changed, fails immediately with SSH's own message
instead of hanging forever waiting for input a GUI client will never provide.

## Checking a host

```bash
overnight host status you@box
```

```
you@box
  os        Linux
  arch      x86_64
  tmux      /usr/bin/tmux
  daemon    overnightd 0.1.0
  cli       overnight 0.1.0
  service   active
  linger    yes
```

`linger yes` is the one to check. Without it the host stops answering when you
log out of it.

## Troubleshooting

**"`overnightd --stdio` did not answer"** — the binary is not on the remote
user's `PATH`. `~/.local/bin` is added by the login shell, so check that the
host's shell profile includes it, or re-run `host install`.

**"runs protocol N; this client speaks M"** — the two sides are different
versions. Re-run `host install` to bring the host up to date, or update the
client.

**Everything derives `LOST`** — tmux is not answering on the host. `overnight
host status` will show whether tmux is present; if the daemon is running but
tmux was killed, every terminal is honestly reported as lost rather than
guessed at.

**The host goes away when you log out** — lingering is off. Fix with
`ssh you@box 'sudo loginctl enable-linger $USER'`.

## Security posture

SSH is the only control-plane transport and the only authentication mechanism.
Overnight does not run a certificate authority, issue client certificates, pin
its own identity key, or operate a pairing flow. OpenSSH already authenticates
both directions, and the asset being protected is a shell that SSH gates anyway.

A remote client is granted `host_admin`, the same as a local one, because it has
already proved it is the Unix user who owns that host's database and worktrees —
someone who could read them directly regardless. The daemon runs as that user
and never as root.
