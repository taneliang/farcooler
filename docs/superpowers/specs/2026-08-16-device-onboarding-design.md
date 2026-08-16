# Onboarding a device without copying a key

Today, putting Far Cooler on a phone means reading a 400-character public key
off one screen, getting it onto a machine somehow, running a shell command
there, and then typing an address, a user and a port back into the app. Every
step is manual and every step is the kind of manual that people do once and then
avoid doing again.

This designs the flow that replaces it. The new device shows a QR code, a device
you already trust scans it, you pick which machines it may reach, and the trusted
device enrolls it through the access it already has.

The manual path is not removed. It is what works when there is no account, no
trusted device, and nothing left but a machine you can still log into — which is
exactly the situation in which a recovery path has to work.

## What is wrong today

`AuthorizeView` in `apps/ios/FarCooler/FarCoolerApp.swift` shows the device's
public key and tells you to run `echo '<paste>' >> ~/.ssh/authorized_keys`.
`Screens.kt` says the same thing on Android. Then `HostEditorView` asks for a
label, an address, a user and a port, and the first connection asks you to
approve a host key fingerprint you have no way to check from a phone.

The design document already disagrees with this. `docs/farcooler-design.md:1007`
says the first device enrolls using access the user already has, and `:867` says
every device self-enrolls with the SSH access it already has. That was never
built, so the fallback became the whole flow.

## What two adversarial reviews changed

Three earlier versions of this design were broken by review. The corrections are
load-bearing enough to state before the design itself.

**The first draft proved possession of a short code with an HMAC.** That is an
offline verifier: the transcript is fetchable by anyone with an account session,
so all 2⁴⁰ candidate codes can be tested locally in about two minutes. No guess
is ever submitted, so no attempt limit fires.

**The second draft replaced it with SPAKE2 and specified none of it** — no
rendezvous, no roles, no key confirmation, no key schedule, no nonce discipline,
and an associated-data list that was circular, since a receiver cannot
authenticate values it only learns by decrypting. The one Rust implementation
says of itself: *"This crate has never received an independent third party audit
for security and correctness. USE AT YOUR OWN RISK!"*

**So there is no password and no PAKE.** The QR carries a 128-bit secret from
one screen to one camera. A camera is an authenticated channel with no network
in it, which is the property the whole ceremony needed and the reason none of the
above is required. See "The ceremony".

**The second draft also gave Macs a single plain key and self-reported
identity**, on the argument that a forced command is theater for a key that can
open a shell. That was wrong. A malicious client with a shell is already beyond
containment, but an *honest* system still cannot close the right sessions on
revocation, or keep two clients' writer leases and idempotency namespaces apart,
if it cannot tell which key authenticated. A Mac gets two keys. See "A Mac needs
two keys".

## The rule that makes this safe

**The relay never touches a machine, and never sees anything it could act on.**

A key is enrolled only by a device that already holds authority there. The secret
that protects everything else travels screen-to-camera, so the relay carries only
ciphertext it cannot read and cannot forge. It can deliver a message, and it can
fail to deliver one. It cannot answer one.

## Blocking prerequisites

None of this is safe to ship until three existing defects are fixed. They are
listed here because the design's security argument assumes them, and they are
defects in shipped code rather than in this document.

**1. The daemon must enforce the scope it is given.**
`crates/daemon/src/main.rs:327` sets `granted_scope: Scope::HostAdmin` for every
stdio session, and when a daemon is already listening, `relay_stdio` pipes the
connection through with no scope check at all. So the forced command's `--scope`
is decorative: a `read` device would today receive full host administration.
Until the daemon reads the client and scope from its own arguments and enforces
them, every grant in this document is a grant of everything.

**2. `ssh_args` must terminate its options.**
`crates/cli/src/remote.rs:62` appends `target` as the last element of the option
list with no `--` separator, so a target beginning with `-o` is parsed as an
option. `-oProxyCommand=…` is local command execution. This is unreachable today
because the target is typed by a human; this design would make it reachable from
a network message, which is exactly the transition that turns a latent hazard
into a vulnerability. Add `--`, and validate that a host is a host.

**3. WorkOS session verification must check the claims it relies on.**
`services/relay/src/workos.ts:23` verifies a signature and an optional expiry. It
does not require `exp`, and does not check the issuer, the `client_id`, the token
type, `iat` or `nbf` — so a token minted for another application in a reused
environment can be accepted as a user session. It also discards `auth_time`,
which is the claim the fresh-authentication requirement below depends on.

## The ceremony

**1. The new device shows a QR code.** It generates its device key, then displays:

| Field | |
| --- | --- |
| `v` | protocol version |
| `secret` | 128 random bits, generated here and never transmitted |
| `key` | its SSH public key |
| `name` | its device name |
| `at` | when it was generated, so a photographed screen goes stale |

**2. A device you already trust scans it.** That device now holds the new
device's public key, taken directly off a screen with no network between them,
and a shared secret nobody else can have.

Direction matters and is not arbitrary. Every device that can be onboarded has a
screen, including a Mac mini; the device doing the approving is usually a phone,
which has a camera. If the approving device has no camera, the answer is the
manual path, and the app says so rather than inventing a weaker ceremony for that
case.

**3. Everything after is AEAD under the QR secret.** One HKDF from `secret`,
with the version and the ceremony's mailbox id as info, yields:

- `k_reply`, the only direction that carries anything — the trusted device
  answers, the new device reads.
- `mailbox`, the first 16 bytes, which is where the answer is posted.

The relay sees an opaque mailbox id and a ciphertext. It cannot derive either, so
it cannot read a manifest, substitute one, or correlate a mailbox with an
account.

A single message in one direction needs no sequence numbers, no directional key
pair, and no replay window. That simplicity is the point: it is what removes the
nonce-reuse and confirmation-ordering questions that sank the PAKE version.

**4. The confirmation sheet**, then enrollment on each chosen machine.

**5. The trusted device posts the manifest** to `mailbox`, sealed under
`k_reply`: for each machine granted, its address, user, port, host key
fingerprint and alias. Associated data carries only what both sides know before
decrypting — version, mailbox id, and a transcript hash over the QR fields. The
payload is inside the ciphertext, which the AEAD authenticates anyway.

**6. The new device fetches and opens it**, and connects with host keys already
pinned, so the unknown-host prompt never appears.

## What the relay stores

| | |
| --- | --- |
| Devices | id, label, push token, public key, created_at |
| Machines | id, label — the existing `daemons` rows |
| Mailboxes | opaque id, one ciphertext · **10-minute TTL** |
| Grants | **nothing** |

The relay knows a machine exists and what you call it. It never learns how to
reach one.

Two routes, both session-authenticated:

- `POST /v1/enroll/post` — `{mailbox, ciphertext}`.
- `POST /v1/enroll/fetch` — `{mailbox}`.

**Rate limiting is per account, with numbers, and fails closed.** Ten posts per
account per hour, one ciphertext per mailbox — a second post to the same mailbox
is refused, not overwritten. The existing limiter covers only `/v1/auth/*`, is
keyed by IP, and fails open when its binding is absent (`index.ts:62`);
reasonable for sign-in, wrong here.

**A device row is created by the trusted device**, after enrollment, not by the
new device through `/v1/devices` — which requires only an account session
(`index.ts:224`). A row the relay fabricates cannot become access, because of the
ground-truth rule below, but it is still roster noise: a device whose key matches
no fence entry and no completed ceremony is shown as **unverified** and cannot be
granted anything.

## Notifications

Push goes to every trusted device on the account when a device is **enrolled** —
the event that matters, and now the only one an attacker cannot manufacture,
since opening a ceremony requires scanning a QR code off a screen you are
standing in front of.

The notice: *"iPhone 17 was added to your machines. If this wasn't you, secure
your account."* Its one action is **This Wasn't Me**, which ends every WorkOS
session on the account and blocks enrollment until you sign in again. A warning
with nothing to do about it is how a security signal becomes noise.

No approve button on any device: approval happened at the camera, so a
notification can only ever make a screen appear.

## A Mac needs two keys

A key can prove who is holding it, or it can open a shell. Not both.

`command="farcooler transport stdio --client CLIENT_ID --scope SCOPE"` is what
makes identity server-asserted: the id was written into `authorized_keys` by
whoever enrolled the key, and the connecting device never sends it and cannot
change it. But a forced command means sshd runs *that program and only that
program* — so Zed's `ssh://` asks for a shell and gets the daemon instead.
Remove the forced command and there is nowhere left to put the client id.

The daemon needs that identity for three things that are not paperwork:

- **Writer leases.** One client at a time may type into a terminal.
- **Idempotency.** Requests are deduplicated per client, so two clients sharing
  an id means one's request is silently dropped as already done.
- **Revocation.** "Remove this Mac and close its live sessions" requires knowing
  which sessions are its. Delete the line while a self-identified shell session
  is open and it keeps running — and can put the line straight back.

The third is the one that survives the objection that a malicious shell client
could lie anyway. It could. An honest system still has to close the right
sessions when someone taps Remove.

| | Key A — Far Cooler | Key B — your shell |
| --- | --- | --- |
| Line | `restrict,command="farcooler transport stdio …"` | plain |
| Used by | the app and the CLI, nothing else | Zed, git, Terminal |
| Lives in | `~/.farcooler/keys/<machine>`, `0600` | `~/.ssh/`, yours |
| Managed by | the app, invisibly | you, with the app's help |
| In `~/.ssh/config`? | **never** | yes, by default |
| Revoking it | removes the Mac from Far Cooler | removes your shell access |

A phone gets Key A only. There is no Zed on a phone.

**Only a shell can grant a shell.** `client.enroll` may write restricted lines
only, so a phone can onboard a Mac for Far Cooler but cannot hand it Key B
access. That needs a Mac that already holds a shell there, or the manual path.
Without this rule, one compromised phone could enroll an attacker's unrestricted
key on every machine and leave the daemon's identity, audit and scope behind
entirely.

### Choosing Key B

Two choices on the new Mac, both defaulted:

**Generate a new key** (default), named `farcooler-<machine>` and editable. Or
**use an existing key**, chosen from `~/.ssh`.

Generating is the default because it is **independently revocable** — not
because it is inherently safer. An existing key may be passphrase-protected,
agent-held or FIDO-backed, and so better protected than a fresh `0600` file. What
it cannot be is removed without consequence: deleting that line takes away the
access your laptop has always had, from everything at once.

**Far Cooler manages every line it writes, including that one.** The earlier
draft said it would add an existing key and decline to manage it, which
deliberately creates access the product cannot revoke — remove a stolen Mac,
watch every managed grant disappear, and leave its pre-existing key opening
shells forever. Instead, removal names exactly what it is about to do:

> Removing **MacBook Air** also removes the key it shares with Terminal and git.
> That Mac will lose SSH access to **box** and **work-mini** entirely, not only
> to Far Cooler.

### `~/.ssh/config`, written so that only Zed gets Key B

The guarantee is structural rather than a matter of getting precedence right.

**Key A is never in `~/.ssh/config`.** Far Cooler passes it on the command line —
`-i ~/.farcooler/keys/<machine> -o IdentitiesOnly=yes` — from `remote.rs`, which
already builds its own argument list. So the app does not read its own block, and
**deleting Far Cooler's block from `~/.ssh/config` cannot break Far Cooler.** It
only takes Zed's access away.

**Key B gets one block per machine**, and that is the whole of what Far Cooler
writes there:

```
Host box
  HostName box.tail-1234.ts.net
  User you
  Port 22
  IdentityFile ~/.ssh/farcooler-macbook-air
  IdentitiesOnly yes
```

`IdentitiesOnly` matters: without it an agent holding a dozen keys offers them
all and can exhaust `MaxAuthTries` before reaching the right one.

Four rules make that block behave:

**The fence goes at the very top of the file.** `ssh_config` takes the **first**
obtained value for each keyword, not the last. An earlier `Host *` setting
`IdentityFile` or `User` would win, and an `Include` near the top — `Include
~/.ssh/config.d/*` is a common first line — pulls its content in at that point
and wins the same way. Appending the block, which is what the earlier draft
implied, is the one placement that reliably does nothing.

**The alias is collision-checked.** A label is not a safe alias: a machine called
`github.com` would silently take over git. Before writing, Far Cooler scans the
config and everything it includes for any pattern matching the proposed alias. On
a hit it suffixes and says which name it used. `HostName` is always set
explicitly, so the alias is never resolved as a hostname — the only risk is
shadowing, and that is what the scan is for.

**Far Cooler hands editors the alias.** `Editors.swift:194` builds
`ssh://{host}{path}`. Given the long address, ssh matches no `Host` entry and the
block does nothing, so the host string for an editor is the alias.

**The file gets the discipline `authorized_keys` gets**: a fenced block, an
atomic write with a checksummed backup, refusal to edit a fence it cannot verify,
and no edit at all to a `Host` block Far Cooler did not write.

A bastion or `ProxyJump` stays in your own configuration on your own `Host`
entry; `--host` accepts any target, so nothing here takes that away.

## Enrolling the key

The trusted device calls `client.enroll` on the target machine's daemon with the
public key, the name and the scope. On a Mac enrolling on itself there is no SSH
at all — the app writes the file. On a remote machine, a Mac uses its own shell
key; a phone uses the daemon, because its own key is restricted and cannot do
otherwise.

`client.enroll` writes **restricted lines only**, and the scope it accepts is
bounded by the caller's own scope. A plain line — Key B — is written only by a
caller that already holds a shell there, which is a Mac, over its own SSH.

The daemon owns the write, for reasons a shell command cannot address:

- **Opens every path component with `openat2`**, anchored to directory
  descriptors and verified with `fstat`, and renames relative to the held
  descriptor. `O_NOFOLLOW` alone guards only the final component, so an attacker
  who replaces `.ssh` with a symlink between the check and the rename redirects
  the write.
- **Refuses a path outside the user's own home**, whatever the configuration
  says. Following `AuthorizedKeysFile` blindly to a root-managed shared file
  turns a user daemon into a write gadget.
- **Refuses unsafe ownership or modes** rather than adding to them. `umask`
  affects only newly created files, so an existing world-writable
  `authorized_keys` stays that way and sshd may ignore it under `StrictModes`.
- **Takes a machine-level lock, writes atomically, `fsync`s the file and its
  directory, and verifies.** This is what append could not do: a file with no
  trailing newline silently makes the new key part of the previous line's
  comment, and two concurrent approvals both pass a duplicate check.
- **Rechecks the caller's authorization while still holding that lock**,
  immediately before the rename. Otherwise a client revoked mid-operation commits
  its enrollment after its sessions were closed.
- **Records an audit entry** naming the enrolling device, the enrolled
  fingerprint, the scope and the time. Which is what makes the next rule
  possible.
- **Returns a result.** `Session::exec` in `crates/client/src/ssh.rs:202` returns
  once execution has been *requested* and its channel loop discards exit-status
  messages, so a shell path would report success after a permission denial, a
  full disk or a `ForceCommand` substitution.

**Removing a device offers to remove what it enrolled.** A `control` device can
enroll keys of its own before anyone revokes it — that follows from being able
to grant at all — so the audit log is read at removal and every descendant is
listed. Claiming that removing one device ends an attacker's persistence, without
this, would be false.

**There is no `sshd -T` check.** The earlier draft promised to read the effective
configuration and warn when keys come from somewhere else. `sshd -T` normally
requires root, and `Match` blocks need `-C user=…,addr=…` which the daemon cannot
know in advance. The connection attempt is the ground truth, and the four states
below report it honestly instead.

### Rendering the key

Never write bytes that came off the wire. `authorized_keys` is line-oriented and
every line may carry options *before* the key, so appending a received string can
append more than one line — one value in, two lines out, the second granting a
stranger a key that runs a command on every connection.

```rust
if received.contains(['\r', '\n']) { return Err(Rejected::MultiLine) }
let parsed = ssh_key::PublicKey::from_openssh(received)?;
if !matches!(parsed.algorithm(), Algorithm::Ed25519) { return Err(Rejected::Algorithm) }
// Rebuilt from key data alone. Nothing from the wire survives except the
// 32 bytes that are the key.
let key = ssh_key::PublicKey::new(parsed.key_data().clone(), comment_for(device));
let line = key.to_openssh()?;
debug_assert!(!line.contains(['\r', '\n']));
```

`to_openssh()` already returns `algorithm base64 comment`, so prefixing
`algorithm()` would emit `ssh-ed25519 ssh-ed25519 AAAA…` and enroll nothing. And
`from_openssh` *keeps* the comment it parsed, so rebuilding from `key_data` is
what actually regenerates it — trailing text is a valid comment, not a parse
error. Both were bugs in the first draft's snippet.

`comment_for` builds `farcooler-<name>-<first 8 of the fingerprint>` from a name
filtered to `[A-Za-z0-9_-]`, falling back to `device` when filtering empties it.
**The key is the identity; the comment is a label for humans.** Refuse a key
already enrolled *on that machine* — which is adoption, not an error, when an
existing Key B is already authorized somewhere, and is reported as already
present rather than failed.

## Grants are per machine

A device is not trusted globally. It holds a grant on each machine separately,
and a machine added next month grants nothing to anyone automatically. That is
what lets one account hold both a work machine and a personal phone without
either reaching the other.

The confirmation defaults to **only the machine being granted from**:

> ### Add "iPhone 17"?
>
> iPhone 17 will be able to run agents and commands on the machines you pick, as
> you.
>
> **SHA256:t7Xq…9Vd** ⌄
>
> ☑︎ MacBook Pro · this Mac
> ☐ box
> ☐ work-mini · needs a device that already reaches it
>
> Far Cooler adds this key to `~/.ssh/authorized_keys` on each machine you pick,
> and changes nothing else. You can add or remove machines later in
> Settings › Devices.
>
> **[ Add Device ]**  [ Cancel ]

Adding a **Mac** shows the same list with the Key B choice above it, and says
what the difference is:

> ### Add "MacBook Air"?
>
> **Far Cooler access** — run agents and terminals on the machines you pick.
>
> **Shell access** — Zed, git and Terminal on that Mac reach them too.
> ☑︎ New key — `farcooler-macbook-air` ⌄
> ☑︎ Add to `~/.ssh/config`
>
> ☑︎ MacBook Pro · this Mac
> ☐ box · shell access needs a Mac that already reaches box
>
> **[ Add Mac ]**  [ Cancel ]

Face ID gates the tap, and the WorkOS session must be fresh — `max_age` on the
authorize endpoint, checked against `auth_time`, which prerequisite 3 makes
available.

Afterwards the same list is **Settings › Devices › iPhone 17**, a checkbox per
machine, with Far Cooler access and shell access as separate rows for a Mac. The
confirmation grants `control`; `read` is set from that screen and is offered for
phones only, since a shell key cannot be held to a scope.

**Revocation is not complete until live sessions are closed**, per
`farcooler-design.md:893`. The daemon indexes sessions by the client id its
forced command supplied, which is why Key A exists.

Machines that were asleep at confirmation time stay pending and enroll when this
device next reaches them.

## The list of devices, and of machines

Existence is stored. Access is derived.

**The machine is the authority on access.** Its daemon reads its own fence and
reports the enrolled fingerprints — the existing `farcooler client list`. No
grant recorded anywhere can disagree with the file.

| State | Means |
| --- | --- |
| Authorized | It connected. |
| Not authorized | The machine's daemon says the fingerprint is not in its fence. |
| Refused | sshd rejected the connection. The reason is shown as sshd gave it, which is usually generic. |
| Unknown | The machine did not answer. Not resolved to either, like `LOST`. |

**Refused is not the same as not authorized.** A key can be rejected for the
wrong Unix user, `StrictModes`, ownership, a declined algorithm, a `Match` block,
an `AuthorizedKeysCommand`, SELinux or `MaxAuthTries`; fail2ban makes a machine
look unreachable rather than hostile. The earlier draft answered every one of
those with "not authorized" and told the user to append the key again — which
produces duplicates and trains people to loosen sshd settings until the error
goes away. **No screen in this flow ever recommends relaxing a security setting**,
and where the client cannot know why sshd refused, it says that rather than
guessing.

**Names come from the fence.** The comment is `farcooler-<name>-<fp8>`, so a
machine lists its own devices with no network at all.

**Never let the relay choose which key to install.** Its stored public key is for
displaying a fingerprint. Every enrollment sources the key from ground truth: a
completed ceremony, or the fence of a machine that already holds it — and the
fingerprint used for that lookup comes from local record, never from the relay.
Otherwise a compromised relay relabels some other key as "Alice's iPhone", and
granting that device to a second machine copies the wrong principal across.

**An address that changes is redistributed over the protocol**, not through the
relay. A daemon reports its own reachability on every authenticated connection,
so a client that can reach a machine at all learns its current address and
updates its `~/.ssh/config` block. A machine no client can reach keeps its stale
block, and host key pinning means the failure is an outage rather than a
takeover.

## Where the device key actually lives

This section is about **phones**. A Mac's keys are ordinary files at `0600` —
Key A under `~/.farcooler/keys`, Key B in `~/.ssh` — which is what makes Key B
usable by Zed and git at all. Their protection is the Mac's own: FileVault, and
the user account they live in.

An earlier draft said a phone's key is generated "in the Secure Enclave or the
Android Keystore". That is false on both platforms, and on iOS not implementable:
the Secure Enclave holds NIST P-256 only, never Ed25519, and cannot import a key
generated elsewhere.

What the code does today: `crates/client/src/ffi.rs:390` generates an Ed25519 key
in Rust and returns the OpenSSH private key **as a string**; `Store.swift` puts
that string in the Keychain, and `Identity.kt` stores it as ciphertext in
preferences under a Keystore AES key with user authentication explicitly
disabled. It is a software key, exportable by anything that can read process
memory. A rooted or jailbroken device yields a clone indistinguishable from the
original.

Say that, and tighten what can be tightened. `kSecAttrAccessibleWhenUnlockedThis-
DeviceOnly`, so it is never in a backup and never on a restored device; an access
control requiring biometry or passcode, since this is the key that grants access
to others; on Android, request hardware backing, verify it with `KeyInfo`, and
require user authentication rather than disabling it. None of these make the key
non-exportable once decrypted, and the document does not claim they do.

**The real fix is a P-256 key in the Secure Enclave**, which OpenSSH accepts as
`ecdsa-sha2-nistp256` and which Face ID can gate per use. The blocker is
concrete: russh is handed a private key today (`PrivateKeyWithHashAlg`,
`crates/client/src/ssh.rs:119`), and a Secure Enclave key cannot be handed to
anything — it signs through a callback. That is a transport change and belongs in
its own document. Until it lands, "stolen unlocked device" and "cloned device"
are the same threat.

## Remote Login

A phone reaches a Mac over SSH, and macOS ships with Remote Login off:

> ### Turn on Remote Login
>
> Your other devices reach this Mac over SSH, and macOS keeps that off until you
> allow it.
>
> Open System Settings › General › Sharing and turn on Remote Login.
>
> **[ Open Sharing Settings ]**

An ungranted machine, on any device:

> ### box hasn't authorized this iPhone
>
> Grant access from a device that already reaches box — open Settings › Devices ›
> iPhone 17 there and add box.
>
> Or add this device's key to `~/.ssh/authorized_keys` on box yourself.
>
> **[ Copy This Device's Key ]**

## Every flow

**Adding a machine.** It appears on the account with a label, granted to nothing.

| | |
| --- | --- |
| A Mac you are sitting at | Install the app. Local daemon over a Unix socket — no key, no SSH. |
| Remote server, from a Mac | Pick it from `~/.ssh/config` or type `you@box`. Your existing SSH installs the daemon. |
| Remote server, from a phone | Through a machine that already reaches it, using *its* access. |
| Remote server, phone alone | Manual — put the key on the box with the access you already have. |
| Any time | `farcooler host install you@box`. |

**Adding a device.** A phone or a Mac, the same ceremony either way.

| | |
| --- | --- |
| With a trusted device | Show the QR on the new device, scan it, pick machines, confirm. |
| A new Mac, no trusted device | It has local authority over itself, so it grants other devices from itself. |
| A new phone, no trusted device | Manual. It becomes a root once enrolled. |
| Approving device has no camera | Manual. No weaker ceremony is offered for this case. |
| Any time | Copy the key, paste into `authorized_keys`. |

**Granting an existing device access to a machine.**

| | |
| --- | --- |
| From the machine itself | A Mac you are at: Settings › Devices → check the device. Local write. |
| From another device | Any device holding `control` there — and shell access only from a device that already holds a shell there. |
| Any time | Paste the key into `authorized_keys` yourself. |

The ceremony requires an account. The manual path is the one that works without
one, and the one that works when every device is lost.

## Threat model

| Adversary | Outcome |
| --- | --- |
| Anonymous internet | Cannot begin — a ceremony starts at a camera. |
| WorkOS account takeover | Cannot enroll anything. It never sees the QR secret, so it cannot post a manifest anyone will open, and it cannot make a machine write a line. It can read the roster and machine labels. |
| Compromised relay or D1 | Cannot derive the mailbox key, so it cannot read, forge or substitute a manifest. Cannot reach a machine. Can withhold a ciphertext, and learns device names, public keys and machine labels. |
| Network attacker on the relay path | Nothing beyond the above. The mailbox is untrusted by design. |
| MITM between an enrolling device and a machine | Refused — the host key is pinned before first use, from a manifest that came through the camera channel. |
| Hostile key bytes | Rebuilt from key data alone, Ed25519 only, CR/LF refused before parsing. |
| Someone photographing the QR | **Works, while it is on screen.** They gain a device key they do not hold and a mailbox nobody will post to unless a human also scans and confirms. The `at` field ages the code out. |
| Someone who scans the QR *and* holds a trusted device | That is the owner. |
| A compromised `control` phone | Can enroll restricted keys wherever it holds `control`, and cannot enroll a shell key anywhere. Removal lists everything it enrolled. |
| **A Mac's Key B, once enrolled** | **It is a shell, by design.** That Mac can rewrite `authorized_keys` and act outside the protocol entirely. Key A is what keeps its Far Cooler sessions identifiable and revocable; Key B is not claimed to be contained. |
| Stolen unlocked trusted device | **Works.** It already holds access. Mitigated by biometry on the confirmation, fresh authentication, and revocation from any other device. |
| **Cloned phone key** | **Works, and is currently undetectable.** The key is a software key; see above. The largest unmitigated risk in the design. |
| Every device lost | Ordinary SSH plus `farcooler client revoke`. No account-recovery bypass, deliberately. |

Requiring TOTP on the WorkOS account is worth doing and is defense in depth, not
the defense. It does not apply to SSO users, so it is not something to lean on.

## Testing

- **The ceremony.** A manifest sealed under one QR secret does not open under
  another; a mailbox id is derived identically on both sides and by neither from
  the ciphertext; a second post to a mailbox is refused; a ciphertext older than
  ten minutes is gone.
- **Key rendering.** Golden files: an embedded newline, a leading options field,
  two keys in one value, trailing text, a hostile comment, a non-Ed25519 key.
  Each produces a refusal or exactly one canonical line, and the line parses back
  to the same key data with the comment we chose.
- **Fence safety.** The existing `authorized_keys::fence_safety` fixture extended
  to cover enroll and revoke, a file with no trailing newline, a symlinked `.ssh`
  swapped between check and rename, wrong modes, an `AuthorizedKeysFile` pointing
  outside the home directory, and two concurrent enrollments.
- **Only a shell grants a shell.** `client.enroll` refuses a plain line from a
  remote caller at every scope, including `host_admin`.
- **Revocation.** A client revoked while its enrollment is in flight does not
  commit; a downgraded client's live session is closed before the operation
  reports success; removing a device lists every key it enrolled.
- **`~/.ssh/config`.** Its own fence fixture: the block lands above any
  `Include`; an existing pattern matching the alias causes a suffix and a
  message; a hand-written `Host` block is left alone; a damaged fence refuses
  rather than rewrites; every byte outside the fence survives.
- **Key A never leaks into ssh config**, and deleting Far Cooler's block leaves
  the app working and only Zed broken.
- **The alias reaches the editor.** A workspace on a registered machine produces
  an `ssh://` URL naming the alias, not the long address.
- **Scope.** A `read` client cannot enroll, cannot revoke, and cannot reach a
  `control` operation — the test prerequisite 1 exists to make passable.
- **Derivation.** A device whose line is deleted by hand reports *not
  authorized*; a device sshd refused reports *refused*, and neither screen
  suggests changing an sshd setting.

## Deferred

- **P-256 in the Secure Enclave**, with russh signing through a callback. Its own
  document; until then the phone key is a software key and the threat model says
  so.
- **Key rotation and `expiry-time=`.** OpenSSH 8.2+ can expire an enrolled key by
  date with no revocation step, which is the cheapest available answer to a clone
  nobody noticed.
- **A local-network path**, so two devices on one LAN could enroll with no
  account.
- **Bulk grant.** "Every machine" is one checkbox away from existing and is
  deliberately not offered, because the default this document argues for is the
  opposite.
