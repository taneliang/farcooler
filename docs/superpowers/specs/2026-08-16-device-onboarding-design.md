# Onboarding a device without copying a key

Today, putting Far Cooler on a phone means reading a 400-character public key
off one screen, getting it onto a machine somehow, running a shell command
there, and then typing an address, a user and a port back into the app. Every
step is manual and every step is the kind of manual that people do once and
then avoid doing again.

This designs the flow that replaces it. A device you already trust shows a
code, you carry it to the new device, you pick which machines it may reach, and
the trusted device enrolls it through the access it already has.

The manual path is not removed. It is what works when there is no account, no
trusted device, and nothing left but a machine you can still log into — which
is exactly the situation in which a recovery path has to work.

## What is wrong today

`AuthorizeView` in `apps/ios/FarCooler/FarCoolerApp.swift` shows the device's
public key and tells you to run `echo '<paste>' >> ~/.ssh/authorized_keys`.
`Screens.kt` says the same thing on Android. Then `HostEditorView` asks for a
label, an address, a user and a port, and the first connection asks you to
approve a host key fingerprint you have no way to check from a phone.

The design document already disagrees with this. `docs/farcooler-design.md:1007`
says the first device enrolls using access the user already has, and `:867`
says every device self-enrolls with the SSH access it already has. That was
never built, so the fallback became the whole flow.

## Two reversals from the first draft

An adversarial review broke the first version of this design. Two of its
decisions are inverted here, and both inversions are load-bearing.

**The proof is a PAKE, not an HMAC.** The first draft had the new device prove
possession of the code with `HMAC-SHA256(key: S, message: public key)`. That is
an offline verifier: anyone holding an account session — and the relay by
construction — can fetch `(public key, proof)` and test all 2⁴⁰ candidate codes
locally, in about two minutes on a GPU. No guess is ever submitted, so no
attempt limit fires. A short secret carried by a human cannot be proved with a
MAC whose transcript is public. That is the problem PAKEs exist for.

**The write goes through the daemon, not through plain `ssh`.** The first draft
had the approving device append the key with
`ssh user@host 'cat >> ~/.ssh/authorized_keys'`, deliberately avoiding Far
Cooler's own binaries as attack surface. That instinct is right in general and
wrong here, for a reason that is structural rather than a matter of taste: every
Far Cooler device key carries `restrict` plus a forced command
(`farcooler-design.md:1006`), so a phone's key **cannot run that command at
all**. The only way to make it run is to hand every phone a plain shell key,
which destroys the identity model the forced command exists to provide — the
daemon learns which client is connected from `--client CLIENT_ID`, and a plain
key carries nothing.

So the choice is not "shell versus daemon". It is "shell plus every phone
holding an unrestricted shell key" versus "daemon". And the daemon path is the
one that can actually do the write safely: an atomic fenced rewrite that refuses
symlinks, checks the effective sshd configuration, verifies after writing, and
records an audit entry. A shell one-liner can do none of that, and it cannot
even tell you whether it succeeded — see `Session::exec` below.

The Mac is the exception that proves this is about restriction, not about
binaries: it writes its own machine's file directly, and reaches *other*
machines with your ordinary SSH identity, which is a real shell.

## The rule that makes this safe

**The relay never touches a machine, and never sees anything it could act on.**

A key is enrolled only by a device that already holds authority there. After
the PAKE completes, everything between the two devices is encrypted under a key
the relay cannot derive, so it cannot forge, substitute, relabel or read. It can
carry a message, and it can fail to carry one. It cannot answer one.

## Blocking prerequisites

None of this is safe to ship until three existing defects are fixed. They are
listed here because the design's security argument assumes them.

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
`services/relay/src/workos.ts:23` verifies a signature and an optional expiry.
It does not require `exp`, and does not check the issuer, the `client_id`, the
token type, `iat` or `nbf` — so a token minted for another application in a
reused environment can be accepted as a user session. It also discards
`auth_time`, which is the claim the fresh-authentication requirement below
depends on.

## The ceremony

**1. On a device you already trust**, tap *Add a Device*. It generates a code —
eight characters of Crockford base32, no lookalike letters — displays it, and
holds it for ten minutes. Where a camera exists on both sides it also displays a
QR code carrying 128 bits instead, which removes the typing without changing
anything below.

**2. On the new device**, sign in and enter the code, by scanning or typing.

**3. Both devices run SPAKE2** through the relay as a mailbox, with the code as
the password. Two messages each way; each side derives the same session key `K`,
and neither the relay nor anyone holding an account session can derive it.

The property that matters: an attacker gets **exactly one password guess per
protocol run**, online, and a wrong guess is detected at key confirmation. There
is no transcript to grind. The first draft's ten-failure counter is gone because
one guess per ceremony is what the primitive already provides — a mistyped code
ends the ceremony and you start again, which is cheap when you are looking at
the code.

`spake2` is the Rust implementation, the same one magic-wormhole uses for
exactly this problem: a short human-carried code over an untrusted rendezvous
server. It runs in `crates/client` behind the existing FFI, so there is one
implementation rather than one per platform — the pattern
`farcooler_client_generate_key` already follows. `curve25519-dalek`, `hmac` and
`sha2` are already in the tree.

**4. Everything after the PAKE is AEAD under `K`**, with this bound into the
associated data of every message:

| Bound | Why |
| --- | --- |
| Protocol version | So a future version cannot be downgraded into this one |
| Account id | So a ceremony cannot be replayed into another account |
| Channel | Stable, preview, canary and local are four deployments; make the separation cryptographic rather than operational |
| Ceremony nonce and expiry | So nothing replays |
| Both device public keys | So neither key can be substituted |
| Device name | It is what a person reads when deciding what to revoke, so it must not be mutable by the carrier |
| The machine set | So the relay cannot widen what was granted |

**5. The new device sends** its name and public key under `K`.

**6. The trusted device shows the confirmation**, then enrolls on each chosen
machine.

**7. The trusted device replies** with a signed manifest under `K`: the machines
granted, and for each one its address, user, port and host key fingerprint. The
first draft encrypted this with `age` to the new device's SSH public key, which
authenticates nothing — the relay knows that public key, so it could encrypt its
own manifest naming an address and host key it controls, and the new device
would pin them and connect without a warning. Under `K` that is not possible,
and `age` is dropped entirely.

**8. The new device connects** with host keys already pinned, so the
unknown-host prompt never appears.

## What the relay stores

| | |
| --- | --- |
| Devices | id, label, push token, public key, created_at |
| Machines | id, label — the existing `daemons` rows |
| Ceremonies | account, PAKE messages, ciphertext, created_at · **10-minute TTL** |
| Grants | **nothing** |

The relay knows a machine exists and what you call it. It never learns how to
reach one, and after the PAKE it holds only ciphertext.

Routes, all session-authenticated:

- `POST /v1/enroll/open` — begin a ceremony, return its id.
- `POST /v1/enroll/message` — post one PAKE or AEAD message.
- `POST /v1/enroll/fetch` — messages for this ceremony.

**Rate limiting is per account, and fails closed.** The existing limiter covers
only `/v1/auth/*`, is keyed by IP, and fails open when the binding is absent
(`index.ts:62`) — reasonable for sign-in, wrong here. Enrollment is capped per
account per hour and refuses rather than admits when the limiter is unavailable.

**A device row is created by the trusted device, at completion.** The first
draft had the new device create its own row through `/v1/devices`, which
requires only an account session (`index.ts:224`) — so "an unapproved request
leaves no device behind" was unenforceable, and anyone with a session could
manufacture roster entries. The party that completed a ceremony is the party
that may say a device exists.

## Notifications

Push goes to every trusted device on the account when a ceremony opens. An
unexpected one is evidence of a compromised account, and that evidence should
reach every device you own rather than whichever one you happen to be holding.

The device holding the code treats it as the working notification. Every other
device shows a notice: *"A device is being added to your account. If this wasn't
you, secure your account."*

**The notice carries one action: "This Wasn't Me."** It ends every WorkOS
session on the account and blocks enrollment until you sign in again. Without
it, the notice is a warning with nothing to do about it, which is how a security
signal becomes noise — and an attacker who can send a few of them a day trains
you to swipe them away. An action makes the notice worth reading.

No approve button, on any device: approval requires the ceremony, so a
notification can only ever make a screen appear.

## Enrolling the key

The trusted device calls `client.enroll` on the target machine's daemon,
carrying the new device's public key, its name, and the scope. On a Mac
enrolling on itself there is no SSH at all — the app writes the file. On a
remote machine the Mac uses your ordinary SSH identity; a phone uses the daemon,
because its own key is restricted and cannot do otherwise.

The daemon owns the write, and owns it for a list of reasons a shell command
cannot address:

- **Opens `~/.ssh/authorized_keys` with `O_NOFOLLOW`** and refuses a symlinked
  path or file. A symlinked `~/.ssh` otherwise redirects the append into another
  file the user owns.
- **Refuses unsafe ownership or modes** rather than adding to them. `umask`
  affects only newly created files, so an existing world-writable
  `authorized_keys` stays world-writable and sshd may ignore it under
  `StrictModes`.
- **Reads the effective sshd configuration** (`sshd -T`) to find where keys
  actually come from. `AuthorizedKeysFile` may name several files or `none`;
  `AuthorizedKeysCommand`, `TrustedUserCAKeys` and `Match` blocks change what is
  accepted. Enrolling into a file sshd does not read produces a device that
  never connects and a UI that says it should.
- **Takes a lock, writes atomically, and verifies.** Temporary file, `fsync`,
  rename, checksummed backup — the care `farcooler-design.md:1017` already
  demands. This fixes what append could not: a file with no trailing newline
  silently makes the new key part of the previous line's comment, two concurrent
  approvals both pass a duplicate check and write twice, and a disconnect
  mid-append leaves a partial line.
- **Records an audit entry** naming the device, the fingerprint, the scope and
  the time.
- **Returns a result.** `Session::exec` in `crates/client/src/ssh.rs:202`
  returns once execution has been requested and its channel loop discards
  exit-status messages, so the shell path could report success after a
  permission denial, a full disk or a `ForceCommand` substitution. A protocol
  call answers.

The entry is `restrict` plus `command="farcooler transport stdio --client
CLIENT_ID --scope SCOPE"` — for every *restricted* device. Which is every phone,
and not a Mac. See below.

## Macs are shell clients, and pretending otherwise buys nothing

A forced command gives the daemon an identity the client cannot lie about. That
is worth having, and it is worth having only for a key that cannot open a shell.
A client holding a shell key can rewrite `authorized_keys`, connect as any other
client, or skip the protocol and run the command directly — so `--client
abc123` on an unrestricted key is a label it can forge in one line.

So identity is **enforced** where it can be and **self-reported** where it
cannot, and the two are not presented as the same thing:

| | Phone | Mac |
| --- | --- | --- |
| Key | managed, generated by the app | an ordinary SSH key |
| Line | `restrict,command=…` | plain |
| Reaches | the daemon protocol only | a shell — Far Cooler, Zed, git, Terminal |
| Identity | server-asserted, unforgeable | self-reported, advisory in audit records |

A Mac gets **one** key, not a restricted one plus a shell one. Two would mean two
things to revoke, and the restricted half would protect nothing its own
machine's other key already gives away.

This is also the answer to a question the first draft could not have asked: a new
laptop needs SSH to your machines for Zed, git and Terminal, not only for Far
Cooler. `apps/macos/Sources/FarCooler/Editors.swift:194` opens a remote worktree
as `ssh://{host}{path}` — Zed does its own SSH through `~/.ssh/config` and never
sees a Far Cooler key. Delivering that access is most of the value of onboarding
a Mac, and the ceremony is already the right vehicle.

### Which key, and where it is registered

Two choices on the new Mac, both defaulted:

**Generate a new key** (default), named `farcooler-<machine>` and editable. Or
**use an existing key**, chosen from `~/.ssh`.

Generating is the default because of revocation. A generated key can be removed
cleanly when the Mac is removed. Your `id_ed25519` cannot: deleting that line
takes away the access your laptop has always had, from Far Cooler and everything
else at once. So choosing an existing key warns, and Far Cooler declines to
manage that line — it will add it, and it will not offer to take it away.

**Register it in `~/.ssh/config`** (default yes), so everything else on the Mac
gets the access too:

```
Host box
  HostName box.tail-1234.ts.net
  User you
  IdentityFile ~/.ssh/farcooler-macbook-pro
  IdentitiesOnly yes
```

`IdentitiesOnly` matters: without it the agent offers every key it holds and a
machine with several can hit `MaxAuthTries` before reaching the right one.

`~/.ssh/config` is a user-critical file and gets exactly the discipline
`authorized_keys` gets: a fenced block, an atomic write with a checksummed
backup, refusal to edit a fence it cannot verify, and no edit at all to a `Host`
block Far Cooler did not write. A conflicting hand-written entry is reported, not
replaced.

The alias is the machine's label, and **Far Cooler uses the alias when it hands a
host string to an editor.** Otherwise `Editors.swift` builds
`ssh://you@box.tail-1234.ts.net/path`, ssh matches no `Host` entry, and the
config Far Cooler just wrote does nothing.

### Rendering the key

Never write bytes that came off the wire. `authorized_keys` is line-oriented and
every line may carry options *before* the key, so appending a received string
can append more than one line — one value in, two lines out, the second granting
a stranger a key that runs a command on every connection.

The first draft's snippet was wrong in two ways that matter. `to_openssh()`
already returns `algorithm base64 comment`, so prefixing `algorithm()` produced
`ssh-ed25519 ssh-ed25519 AAAA…`, which enrolls nothing. And `from_openssh`
*keeps* the comment it parsed, so "the comment is regenerated" was false and
"trailing garbage fails to parse" was false — trailing text is a valid comment.

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

`PublicKey::new(key_data, comment)` and `to_openssh()` are both in ssh-key
0.7.0-rc.11, already in the tree through russh.

`comment_for` builds `farcooler-<name>-<first 8 of the fingerprint>` from a name
filtered to `[A-Za-z0-9_-]`, falling back to `device` when filtering empties it.
The fingerprint suffix is there because a filtered name is not an identity: two
devices can filter to the same string, and a person renaming a phone must not
thereby collide with another. **The key is the identity; the comment is a label
for humans.** Refuse a key already enrolled; do not refuse a name.

## Grants are per machine

A device is not trusted globally. It holds a grant on each machine separately,
and a machine added next month grants nothing to anyone automatically. That is
what lets one account hold both a work machine and a personal phone without
either reaching the other.

The confirmation defaults to **only the machine being granted from**:

> ### Add "iPhone 17"?
>
> iPhone 17 will be able to run agents and commands on the machines you pick,
> as you.
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

Face ID gates the tap, and the WorkOS session must be fresh — `max_age` on the
authorize endpoint, checked against `auth_time`, which prerequisite 3 makes
available.

Adding a **Mac** shows the same machine list with the key choice above it, and
says what the difference is out loud:

> ### Add "MacBook Air"?
>
> MacBook Air will be able to run agents and commands on the machines you pick,
> as you.
>
> **Key** · New key — `farcooler-macbook-air` ⌄
> ☑︎ Add to `~/.ssh/config`, so Zed, git and Terminal reach these machines too
>
> ☑︎ MacBook Pro · this Mac
> ☐ box
>
> Unlike a phone, a Mac's key is an ordinary SSH key: it opens a shell on the
> machines you pick, not only Far Cooler.
>
> **[ Add Mac ]**  [ Cancel ]

Afterwards the same list is **Settings › Devices › iPhone 17**, a checkbox per
machine. The confirmation grants `control`. `read` is set from that same screen,
and says out loud that a `read` device cannot enroll anything — and that it is
available for phones only, since a shell key cannot be held to a scope.

**Revocation is not complete until live connections are closed.** Unchecking a
machine removes the fence entry *and* terminates that client's open sessions
before reporting success, per `farcooler-design.md:893`. Otherwise a device
downgraded from `control` to `read` continues at `control` over the connection
it already holds, for as long as it holds it.

Machines that were asleep at confirmation time stay pending and enroll when this
device next reaches them.

## The list of devices, and of machines

Existence is stored. Access is derived.

**The machine is the authority on access.** Its daemon reads its own fence and
reports the enrolled fingerprints — the existing `farcooler client list`. No
grant recorded anywhere can disagree with the file.

Four honest states, and the fourth is the one the first draft got wrong:

| State | Means |
| --- | --- |
| Authorized | It connected. |
| Not authorized | The machine's daemon says the fingerprint is not in its fence. |
| Refused | sshd rejected the connection, and the reason is shown as sshd gave it. |
| Unknown | The machine did not answer. Not resolved to either, like `LOST`. |

**Refused is not the same as not authorized**, and conflating them was a real
hazard. A key can be rejected for the wrong Unix user, `StrictModes`, ownership,
a declined algorithm, a `Match` block, an `AuthorizedKeysCommand`, SELinux, or
`MaxAuthTries`; fail2ban makes a machine look unreachable rather than hostile.
The first draft answered every one of those with "not authorized" and told the
user to append the key again — which produces duplicates, and trains people to
loosen sshd settings until the error goes away. **No screen in this flow ever
recommends relaxing a security setting.**

Likewise, `authorized_keys` is the authority only to the extent sshd is
configured to read it. Where `sshd -T` shows keys arriving from an
`AuthorizedKeysCommand` or a trusted CA, the daemon says so and reports Unknown
rather than claiming an authority it does not have.

**Names come from the fence.** The comment is `farcooler-<name>-<fp8>`, so a
machine lists its own devices with no network at all. The relay's roster adds
only devices granted nowhere yet, and surfaces a fence entry matching no device
on the account, shown with its fingerprint and a way to remove it.

**Never let the relay choose which key to install.** Its stored public key is
for displaying a fingerprint. Every enrollment sources the key from ground
truth: the completed ceremony, or the fence of a machine that already holds it —
and the fingerprint used for that lookup comes from local record, never from the
relay. Otherwise a compromised relay relabels some other key as "Alice's iPhone",
and granting that device to a second machine copies the wrong principal across.
If neither source exists, the granting device says so and points back at the
ceremony.

**Removal is honest.** Deleting a device from the account stops its
notifications and does nothing to its SSH access, so "Remove Device" walks the
grants first, unchecking each machine it can reach and naming the ones it
cannot:

> iPhone 17 still has access to **box**, which isn't reachable right now. Remove
> it from box when you next reach it, or run `farcooler client revoke` there.

## Where the device key actually lives

This section is about **phones**. A Mac's key is an ordinary file in `~/.ssh` at
`0600`, which is what every other tool on that machine expects and what makes it
usable by Zed and git at all. Its protection is the Mac's own — FileVault, and
the user account it lives in.

The first draft said the key is generated "in the Secure Enclave or the Android
Keystore". That is false on both platforms, and on iOS it is not implementable
as stated: the Secure Enclave holds NIST P-256 only, never Ed25519, and cannot
import a key generated elsewhere.

What the code does today: `crates/client/src/ffi.rs:390` generates an Ed25519
key in Rust and returns the OpenSSH private key **as a string**; `Store.swift`
puts that string in the Keychain, and `Identity.kt` stores it as ciphertext in
preferences under a Keystore AES key with user authentication explicitly
disabled. It is a software key, exportable by anything that can read process
memory. A rooted or jailbroken device yields a clone that is indistinguishable
from the original.

Say that, and tighten what can be tightened now:

- `kSecAttrAccessibleWhenUnlockedThisDeviceOnly`, so it is never in a backup and
  never on a restored device.
- An access control requiring biometry or passcode for the *enrolling* device's
  key, since that is the key that grants access to others.
- Android: request hardware backing and verify it with `KeyInfo`, and require
  user authentication rather than disabling it.

**The real fix is a P-256 key in the Secure Enclave**, which OpenSSH accepts as
`ecdsa-sha2-nistp256` and which Face ID can gate per use. The blocker is
concrete: russh is handed a private key today
(`PrivateKeyWithHashAlg`, `crates/client/src/ssh.rs:119`), and a Secure Enclave
key cannot be handed to anything — it signs through a callback. That is a
transport change, not a key-format change, and it belongs in its own document.
Until it lands, "stolen unlocked device" and "cloned device" are the same threat.

## Remote Login

A phone reaches a Mac over SSH, and macOS ships with Remote Login off. Shown
when a Mac is added as a machine, and again on any device that cannot reach it:

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
> Grant access from a device that already reaches box — open Settings › Devices
> › iPhone 17 there and add box.
>
> Or add this device's key to `~/.ssh/authorized_keys` on box yourself.
>
> **[ Copy This Device's Key ]**

## Every flow

**Adding a machine.** It appears on the account with a label, granted to
nothing.

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
| With a trusted device | *Add a Device*, carry the code, pick machines, confirm. |
| A new Mac, no trusted device | It has local authority over itself, so it grants other devices from itself. |
| A new phone, no trusted device | Manual. It becomes a root once enrolled. |
| Any time | Copy the key, paste into `authorized_keys`. |

**Granting an existing device access to a machine.**

| | |
| --- | --- |
| From the machine itself | A Mac you are at: Settings › Devices → check the device. Local write. |
| From another device | Any device holding `control` there: the same screen. |
| Any time | Paste the key into `authorized_keys` yourself. |

A new Mac joining is two rows at once — a machine nothing can reach and a device
nothing has granted — and because it holds local authority over itself, it is
the one place that can resolve the first half without asking anyone.

The ceremony requires an account. The manual path is the one that works without
one, and the one that works when every device is lost.

## Threat model

| Adversary | Outcome |
| --- | --- |
| Anonymous internet | Cannot open a ceremony — it needs an account session. |
| WorkOS account takeover | Can open ceremonies, cannot complete one: SPAKE2 gives one online guess per run against the code, and there is no transcript to attack offline. Every device is notified and can end every session with one tap. |
| Compromised relay or D1 | Cannot derive `K`, so it cannot forge a manifest, substitute a host key, relabel a device, or read anything after the PAKE. Can suppress ceremonies, and learns account emails, push tokens, machine labels and public keys. |
| Network attacker on the relay path | Nothing beyond the above. The mailbox is untrusted by design. |
| MITM between an enrolling device and a machine | Refused — the host key is already pinned, and manifests are authenticated under `K`. |
| Hostile key bytes | Rebuilt from key data alone, Ed25519 only, CR/LF refused before parsing. |
| Shoulder-surfing the code | **Works, within ten minutes.** Requires physical proximity — the accepted cost of a human-carried secret, and the QR path shortens the window it is visible. |
| Stolen unlocked trusted device | **Works.** It already holds access. Mitigated by biometry on the confirmation, fresh authentication, and revocation from any other device. |
| **Cloned device key** | **Works, and is currently undetectable.** The key is a software key; see above. This is the largest unmitigated risk in the design and the reason the Secure Enclave work matters. |
| Stolen untrusted phone | Cannot open a ceremony anyone will complete. |
| **A Mac's key, once enrolled** | **It is a shell, by design.** A Mac can rewrite `authorized_keys`, impersonate any client, and act outside the protocol entirely. Its audit records are advisory, and the design says so rather than implying a forced command constrains it. |
| An existing key chosen instead of a generated one | Enrolled but never managed. Far Cooler will not offer to revoke the key your laptop has always used, so removing that Mac leaves its shell access in place and says so. |
| Every device lost | Ordinary SSH plus `farcooler client revoke`. No account-recovery bypass, deliberately. |

Requiring TOTP on the WorkOS account is worth doing and is defense in depth, not
the defense. It does not apply to SSO users, so it is not something to lean on.

## Testing

- **The PAKE.** A wrong code fails key confirmation and ends the ceremony; a
  correct one derives the same `K` on both sides; a replayed message from
  another ceremony, account or channel is refused by the bound associated data.
- **Key rendering.** Golden files: an embedded newline, a leading options field,
  two keys in one value, trailing text, a hostile comment, a non-Ed25519 key.
  Each produces a refusal or exactly one canonical line — and the line is
  asserted to parse back to the same key data with the comment we chose.
- **Fence safety.** The existing `authorized_keys::fence_safety` fixture
  extended to cover enroll and revoke, a file with no trailing newline, a
  symlinked path, wrong modes, and two concurrent enrollments.
- **sshd reality.** A fixture where `AuthorizedKeysFile` points elsewhere, and
  one where `AuthorizedKeysCommand` is set: both must report Unknown rather than
  claiming authority.
- **Scope.** A `read` client cannot enroll, cannot revoke, and cannot reach a
  `control` operation — the test that prerequisite 1 exists to make passable.
- **`~/.ssh/config` safety.** Its own fence fixture: a hand-written `Host` block
  of the same name is reported and left alone, a damaged fence refuses rather
  than rewrites, and every entry outside the fence survives byte for byte.
- **The alias reaches the editor.** A workspace on a registered machine produces
  an `ssh://` URL naming the alias, not the long address, so the `IdentityFile`
  Far Cooler wrote is the one ssh uses.
- **Revocation latency.** A downgraded client's live session is closed before
  the operation reports success.
- **Derivation.** A device whose line is deleted by hand reports *not
  authorized*; a device refused by `StrictModes` reports *refused* with the
  reason, and neither screen suggests changing an sshd setting.
- **Expiry.** A ceremony older than ten minutes is gone.

## Deferred

- **P-256 in the Secure Enclave**, with russh signing through a callback. Its
  own document; until then the device key is a software key and the threat model
  says so.
- **A local-network path**, so two devices on one LAN could run the ceremony
  with no account.
- **Granting across machines a device has never met.** Today the answer is to
  run the ceremony again, which is honest and rare.
- **Bulk grant.** "Every machine" is one checkbox away from existing and is
  deliberately not offered, because the default this document argues for is the
  opposite.
