# Onboarding a device without copying a key

Today, putting Far Cooler on a phone means reading a 400-character public key
off one screen, getting it onto a machine somehow, running a shell command
there, and then typing an address, a user and a port back into the app. Every
step is manual and every step is the kind of manual that people do once and then
avoid doing again.

This designs the flow that replaces it. The new device shows a QR code, a device
you already trust scans it, you pick which machines it may reach, and the trusted
device shows a QR code back.

The manual path is not removed. It is what works when there is no trusted device
and nothing left but a machine you can still log into — which is exactly the
situation in which a recovery path has to work.

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

## What three adversarial reviews changed

Every earlier version of this design was broken by review, and the corrections
are load-bearing enough to state before the design itself.

**Draft one proved possession of a short code with an HMAC.** That is an offline
verifier: the transcript is fetchable, so all 2⁴⁰ candidate codes can be tested
locally in about two minutes.

**Draft two replaced it with SPAKE2 and specified none of it** — no rendezvous,
no roles, no key confirmation, no key schedule, and a circular associated-data
list. The one Rust implementation says of itself that it "has never received an
independent third party audit."

**Draft three put a 128-bit secret in a QR code and sent one sealed message back
through the relay.** But a symmetric secret on a screen is a bearer token:
whoever films it derives the same key. Combined with an account session or a
compromised relay, that reads the manifest, or replaces it with one naming an
attacker's address and host key — and on a Mac that address lands in
`~/.ssh/config`, so Zed and git follow it.

**So the reply comes back the way it went out: through a camera.** Two scans, no
shared secret, no mailbox, no relay. Someone filming the exchange sees two public
keys and a list of addresses they already needed access to reach.

**Draft two and three also claimed "only a shell can grant a shell."** That is
false. A `control` device drives a terminal, and a terminal can append to
`authorized_keys`. See "What `control` really means".

## The rule that makes this safe

**Nothing that authorizes an enrollment travels over the network.**

Both legs of the ceremony are screen-to-camera. The trusted device enrolls over
access it already holds. The relay carries no key, no manifest and no
authorization — it answers exactly one question, below, and a wrong answer from
it can only refuse an enrollment, never cause one.

### Two gates, and only one of them is the account

**Both devices must be signed into the same account.** The new device signs in
and registers its keys through `/v1/devices`, which is session-authenticated — so
the relay holds `(device, account, key_a, key_b)` where the account came from a
WorkOS session rather than from anything the device claimed. When the trusted
device scans, it asks one question, **scoped to its own account**:

```sql
SELECT id, label FROM devices
 WHERE key_a_fingerprint = ? AND account_id = <the caller's account, from its session>
```

**The fingerprint, not the key.** The relay's only use for a device key is this
lookup, so it stores `SHA256:t7Xq…9Vd` — the same string a person reads on
screen — and never the key itself. An ed25519 public key is 32 random bytes, so
its hash is a one-way identifier: matchable, not reversible. That makes "never
install a key the relay handed you" structural rather than a rule to remember,
because the relay has no key to hand anyone. Key B never reaches the relay at
all; the shell fingerprint shown in Settings comes from the machine's fence,
which is ground truth.

The scoping is what makes it sound. A lookup by key alone would return some
device and leave the caller comparing account ids, which breaks the moment two
accounts register the same public key. Scoped, that case cannot arise: the answer
is "yes, on your account" or "no", and never whose it is otherwise — which also
stops the route being a key-enumeration oracle.

A phone signed into another account has no row on yours. A phone signed into
nothing has no row at all, which is why sign-in comes before the code.

**This gate is not relay-proof, and is not claimed to be.** A compromised relay
can answer yes to anything. What it cannot do is make someone scan a QR code or
produce a fingerprint, so the most it achieves is turning a refusal into a pass
for a device already being held up to your camera — relay compromise plus
physical presence, at which point the local gate below is what remains.

**Offline is a refusal, not a bypass.** A trusted device that cannot reach the
relay does not enroll.

A WorkOS token in the QR would remove the relay from this check, and is
deliberately not used: a JWT on a screen is a bearer token for the account, which
is a worse version of the mistake draft three made. Everything in the QR stays
public.

**The confirmation demands local authentication.** Touch ID, Face ID, or the
device passcode through `LocalAuthentication`, at the moment of the tap — not a
session that unlocked hours ago.

The second gate is the one that matters for the case people actually worry
about: someone walking past an unlocked laptop. An account check does nothing
there, because that laptop is already signed in and the passer-by would be using
*your* session. What stops them is being asked for a fingerprint they do not
have.

The account gate covers two other things. A device that is not yours cannot be
enrolled even if the first gate is somehow passed. And — the reason that would
justify it on its own — **a device enrolled from another account makes the device
list wrong.** It would hold keys on your machines while appearing in someone
else's Settings › Devices, so your list would not show it, "Remove Device" could
not reach it, and the one screen that is supposed to answer "what can reach my
machines" would be quietly incomplete. Someone with a work account and a personal
account would produce that by accident, not by attack.

The cost is that the ceremony no longer runs offline, which is a trade worth
making explicitly rather than by omission.

## Blocking prerequisites

None of this is safe to ship until four defects in existing code are fixed. They
are listed here because the design's argument assumes them.

**1. The daemon must enforce the scope it is given.**
`crates/daemon/src/main.rs:327` sets `granted_scope: Scope::HostAdmin` for every
stdio session, and when a daemon is already listening, `relay_stdio` pipes the
connection through with no scope check at all. So the forced command's `--scope`
is decorative: a `read` device would today receive full host administration.

**2. `ssh_args` must terminate its options.**
`crates/cli/src/remote.rs:62` appends `target` as the last element of the option
list with no `--` separator, so a target beginning with `-o` is parsed as an
option. `-oProxyCommand=…` is local command execution. Unreachable today because
the target is typed by a human; this design would make it reachable from a
manifest.

**3. `remote.rs` must actually select Key A, and multiplexing must not defeat
it.** The design below depends on Far Cooler connecting as a specific key.
Today `ssh_args` passes no `-i` at all, and two of its options actively break
the model:

- `ControlPath=~/.ssh/farcooler-%r@%h:%p` keys the multiplexed master on
  **user, host and port — not on the key**. A master already authenticated with
  some other key services the Key A invocation, `-i` is never consulted, and the
  daemon receives no forced-command identity. The path must include the key
  identity and the channel.
- `ControlPersist=120` keeps an authenticated master alive for two minutes.
  sshd reads `authorized_keys` at authentication, so a surviving master opens
  new channels **after the key is revoked**. Revocation must close the master,
  not only the channel.

**4. WorkOS session verification must check the claims it relies on.**
`services/relay/src/workos.ts:23` verifies a signature and an optional expiry. It
does not require `exp`, and does not check the issuer, the `client_id`, the token
type, `iat` or `nbf`. It also discards `auth_time`, which the
fresh-authentication requirement below depends on.

## The ceremony

**1. The new device signs in, then shows a QR code.** Sign-in comes first and is
not optional: the device registers its keys against the account, which is what
makes step 2's check answerable at all. A device that is not signed in has no QR
code to show.

It generates its keys, then displays:

| Field | |
| --- | --- |
| `v` | protocol version |
| `key_a` | its Far Cooler public key |
| `key_b` | its shell public key — Macs only, and only if shell access was chosen |
| `name` | its device name |
| `account` | which account it is joining — an opaque id, not a credential |

No secret. Everything here is public, and a photograph of it is worth nothing:
enrolling these keys grants access to a device the photographer does not hold.
There is no nonce, because enrolling a key twice is the same as enrolling it
once — a replayed scan has nothing to replay.

**2. A device you already trust scans it.** That device now holds the new
device's keys, taken off a screen with no network in between, and checks with the
relay that they belong to a device on this account.

Freshness is judged by the **scanner's** clock, from when it scanned — never
from a timestamp inside the code, which the displaying device controls. The
window bounds how long a confirmation may sit unanswered, not how old the
photograph was: a screenshot carries only public keys, so presenting one later
gets someone an enrollment of a device they still do not hold.

A key on another account, or on no account, stops here — before the confirmation
sheet appears, so there is never a moment where the machines are on screen and
only a fingerprint stands between a stranger and them:

> ### That device is signed into a different account
>
> Far Cooler can only add devices signed into **o.o@elt.sg**. Sign in to that
> account on the new device, then show its code again.
>
> **[ Done ]**

**3. The confirmation sheet**, then enrollment on each chosen machine.

**4. The trusted device shows a QR code back**, carrying the manifest: for each
machine granted, its address, user, port, host key fingerprint and alias. A
machine is about 120 bytes and a version-40 code holds roughly 1850, so **one
static code, capped at fifteen machines.** Past that, grant some now and the rest
from the new device later. No animated sequence, no reassembly, no partial-set
handling — a cap that will almost never be reached beats machinery that always
exists.

This leg needs no signature and no encryption. It is pixels to a lens, with no
network and no third party — the same channel as the first leg, pointed the
other way. Both devices are in the same room by construction, because the first
scan already required it.

**5. The new device records the manifest as consumed** and connects, with host
keys already pinned, so the unknown-host prompt never appears.

Machines that were asleep during step 3 are still listed, marked pending. Access
follows when the trusted device next reaches them; the new device simply retries
and succeeds. Nothing more has to be delivered.

**If the new device has no camera** — a Mac mini — there is no ceremony. That is
the manual path, and the app says so rather than inventing a weaker exchange for
the case where the strong one does not fit.

## Later grants are self-service

Giving an already-enrolled device access to a *new* machine happens months later,
with nobody standing next to anybody, so the camera is unavailable.

An earlier draft had the relay carry a signed, encrypted manifest between the two
devices, with signatures verified against the fence of a machine they share. It
worked, and it was a subsystem — signing, encryption, a TTL, rate limits, and a
verification story — built to move one thing: **a machine's address.**

There is already a path that moves it with no relay at all. **A device asks a
machine it already reaches to enroll it on the new one.** That machine has SSH to
the new one — it is how the new one was installed — so it calls `client.enroll`
there and returns the address, port, host key fingerprint and alias over the
connection the asking device already holds. Every hop is authenticated, every
host key is pinned, and nothing passes through anything that could forge it.

The rule that follows, and the reason this stays simple:

**A device learns a machine's address only from something it is already talking
to.** The ceremony, or a machine it already reaches. Nothing else.

So **later grants are self-service**: you add machines *from the device that
wants them*. Settings › Devices on another device shows that device's grants
read-only, and says to grant from there. That removes remote cross-device
granting, which is the only case that ever needed the relay, and costs a habit
nobody has formed yet — you ask for access on the device you are holding.

If no machine you can reach also reaches the new one, the answer is the ceremony
in person or the manual path, and the app says exactly that.

## What the relay stores

| | |
| --- | --- |
| Devices | id, label, push token, **key_a fingerprint**, created_at |
| Machines | id, label — the existing `daemons` rows |
| Keys | **none** |
| Addresses | **none** |
| Grants | **none** |

A fingerprint, not a key, and only Key A's — see above. The relay knows a machine
exists and what you call it. It never learns how to reach one, holds nothing that
could be installed anywhere, and is absent from enrollment except for one lookup
whose worst possible answer is a refusal.

A device row is created by the **trusted** device after enrollment. `/v1/devices`
requires only an account session (`index.ts:224`), so a row the relay or a
session-holder fabricates proves nothing: a device whose key matches no fence
entry and no completed ceremony is shown as **unverified** and cannot be granted
anything.

The account lookup is rate limited per account and fails closed. The existing
limiter covers only `/v1/auth/*`, is keyed by IP, and fails open when its binding
is absent (`index.ts:62`) — reasonable for sign-in, wrong here.

## Notifications

Push goes to every trusted device on the account when a device is **enrolled**.
The notice: *"iPhone 17 was added to your machines. If this wasn't you, secure
your account."* Its one action is **This Wasn't Me**, which ends every WorkOS
session and blocks grants until you sign in again.

This is detection, not prevention, and the document does not count it as a
defense. Its value is that a device enrolled by someone else produces a
notification naming machines that do not match what happened.

No approve button on any device: approval happened at the camera.

## What `control` really means

Earlier drafts claimed "only a shell can grant a shell," enforced by
`client.enroll` refusing to write plain lines. **That is not a security
boundary.** A `control` device can drive a terminal, and a terminal can run
`echo … >> ~/.ssh/authorized_keys`. `farcooler-design.md:1016` says as much: a
control client reaches a shell by writing into a managed terminal.

So the honest statement, which `:897` already makes:

- **`control` is shell-equivalent.** A control device can enroll keys of its own,
  install its own persistence, and survive revocation of the key it came in on.
- **Revocation is containment, not undo.** Revoke first, then audit the host.
- **`read` is a real boundary**, because `restrict` plus a forced command means
  no shell exists to escape through — and it is a boundary enforced by sshd,
  which is why prerequisite 1 matters.

`client.enroll` still refuses to write plain lines, and still refuses a scope
above the caller's own. That is a guard rail against a mistake, and the document
calls it one.

**Removing a device therefore offers to remove what it enrolled.** The audit log
names every key each device added, so removal lists the descendants. Without
this, "removed" would imply an eviction that did not happen.

## A Mac needs two keys

A key can prove who is holding it, or it can open a shell. Not both.

`command="farcooler transport stdio --client CLIENT_ID --scope SCOPE"` is what
makes identity server-asserted: the id was written into `authorized_keys` by
whoever enrolled the key, and the connecting device never sends it and cannot
change it. But a forced command means sshd runs *that program and only that
program*, so Zed's `ssh://` asks for a shell and gets the daemon instead. Remove
the forced command and there is nowhere left to put the client id.

The daemon needs that identity for three things that are not paperwork:

- **Writer leases.** One client at a time may type into a terminal.
- **Idempotency.** Requests are deduplicated per client, so two clients sharing
  an id means one's request is silently dropped as already done.
- **Revocation.** Closing a device's live sessions requires knowing which
  sessions are its.

The third survives the objection that a malicious shell client could lie anyway.
It could. An honest system still has to close the right sessions when someone
taps Remove.

| | Key A — Far Cooler | Key B — your shell |
| --- | --- | --- |
| Line | `restrict,command="farcooler transport stdio …"` | plain |
| Used by | the app and the CLI, nothing else | Zed, git, Terminal |
| Lives in | the channel's key directory, `0600` | `~/.ssh/`, yours |
| Managed by | the app, invisibly | you, with the app's help |
| In `~/.ssh/config`? | **never** | yes, by default |
| Revoking it | removes the Mac from Far Cooler | removes its shell access |

A phone gets Key A only. There is no Zed on a phone.

**Removing a Mac is two operations and the UI says so.** Removing Key A closes
its identifiable Far Cooler sessions. Removing Key B stops future
authentication but **does not terminate a shell that is already open**, and that
shell can put both lines back. So removal reports what it did, not what one
would like it to have done, and says to audit the machine if the Mac is believed
hostile.

### Where Key A lives

One Key A per device, not per machine — the same key is enrolled on every machine
that device reaches, exactly as a phone's is.

It lives at `<channel runtime dir>/keys/device`, a fixed name under the
channel's own directory. Not a path built from a machine label: a label is
mutable and user-supplied, so `../`, `/` and collisions all become key files in
the wrong place or orphaned on rename. Channel-scoped because stable, preview,
canary and local must not share a key any more than they share a daemon.

The directory is created `0700`, the key `0600`, both opened with
descriptor-relative calls and `O_NOFOLLOW`, and the key with `O_EXCL` so an
existing file is never written through. **The CLI and the app resolve this path
through one shared function**, not by each building the string — today the CLI
has no notion of Key A at all, which is prerequisite 3.

### Choosing Key B

**Generate a new key** (default), named `farcooler-<machine>` and editable. Or
**use an existing key**, chosen from `~/.ssh`.

Generating is the default because it is **independently revocable** — not
because it is safer. An existing key may be passphrase-protected, agent-held or
FIDO-backed, and so better protected than a fresh `0600` file. What it cannot be
is removed without consequence.

**Far Cooler manages every line it writes, including that one**, and removal
names what it is about to do:

> Removing **MacBook Air** also removes the key it shares with Terminal and git.
> That Mac will lose SSH access to **box** and **work-mini** entirely, not only
> to Far Cooler. Sessions already open there will stay open.

### `~/.ssh/config`, written so that only Zed gets Key B

The guarantee is structural rather than a matter of winning a precedence fight.

**Key A is never in `~/.ssh/config`.** Far Cooler passes it on the command line,
with its own `ControlPath`, from the shared connection path prerequisite 3
builds. So the app does not read its own block, and **deleting Far Cooler's block
cannot break Far Cooler** — it only takes Zed's access away. For that to hold,
the connection path must carry the address, user, port, host-key policy and key
itself rather than depending on any `Host` entry, which is part of the same
prerequisite.

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

Four rules make that block behave:

**The fence goes at the very top of the file.** `ssh_config` takes the **first**
obtained value for each keyword, not the last. An earlier `Host *` setting
`IdentityFile` or `User` wins, and `Include ~/.ssh/config.d/*` — a common first
line — pulls its content in at that point and wins the same way.

**The alias is collision-checked** against the config and everything it
includes. A machine labeled `github.com` would otherwise take over git. On a hit
Far Cooler suffixes and says which name it used. `HostName` is always explicit,
so the alias is never resolved as a hostname; shadowing is the only risk and the
scan is what addresses it.

**Far Cooler hands editors the alias.** `Editors.swift:194` builds
`ssh://{host}{path}`; given the long address, ssh matches no entry and the block
does nothing.

**The file gets the discipline `authorized_keys` gets**: a fenced block, an
atomic write with a checksummed backup, refusal to edit a fence it cannot verify,
and no edit to a `Host` block Far Cooler did not write.

Two honest limits. `IdentitiesOnly` bounds ssh to identities named in the
configuration and on the command line — additional matching `IdentityFile` and
`CertificateFile` directives still accumulate and can exhaust `MaxAuthTries`
before the right key is offered. And a textual scan is a snapshot: `Match exec`,
files included later, and hostname canonicalization can change behavior
afterwards. Far Cooler writes the clearest block it can and does not claim to own
the file.

A bastion or `ProxyJump` stays on your own `Host` entry; `--host` accepts any
target, so nothing here takes that away.

## Enrolling the key

The trusted device calls `client.enroll` on the target machine's daemon with the
public key, the name and the scope. On a Mac enrolling on itself there is no SSH
at all — the app writes the file. On a remote machine, a Mac uses its own shell
key; a phone uses the daemon, because its own key is restricted and cannot do
otherwise.

The daemon owns the write, for reasons a shell command cannot address:

- **Opens every path component with descriptor-relative calls and
  `O_NOFOLLOW`**, verifies each with `fstat`, and renames relative to the held
  directory descriptor. `O_NOFOLLOW` on the final path alone guards only that
  component, so an attacker who replaces `.ssh` with a symlink between check and
  rename redirects the write. (`openat2` would be neater and is Linux-only; this
  product also ships on macOS, so the portable form is the one specified.)
- **Refuses a path outside the user's own home**, whatever the configuration
  says, so following `AuthorizedKeysFile` cannot turn a user daemon into a write
  gadget.
- **Refuses unsafe ownership or modes** rather than adding to them. `umask`
  affects only newly created files.
- **Takes a machine-level lock, writes atomically, `fsync`s the file and its
  directory, and verifies.** A file with no trailing newline otherwise makes the
  new key part of the previous line's comment, and two concurrent approvals both
  pass a duplicate check.
- **Rechecks the caller's authorization while still holding that lock**,
  immediately before the rename, so a client revoked mid-operation does not
  commit after its sessions were closed.
- **Records an audit entry** naming the enrolling device, the enrolled
  fingerprint, the scope and the time.
- **Returns a result.** `Session::exec` in `crates/client/src/ssh.rs:202` returns
  once execution has been *requested* and its channel loop discards exit-status
  messages, so a shell path would report success after a permission denial.

There is no `sshd -T` check. It normally requires root, and `Match` blocks need
`-C user=…,addr=…` the daemon cannot know in advance. The connection attempt is
the ground truth, and the four states below report it honestly.

### Rendering the key

Never write bytes that came off the wire. `authorized_keys` is line-oriented and
every line may carry options *before* the key, so appending a received string can
append more than one line — the second granting a stranger a key that runs a
command on every connection.

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
error. Both were bugs in an earlier draft's snippet.

`comment_for` builds `farcooler-<name>-<first 8 of the fingerprint>` from a name
filtered to `[A-Za-z0-9_-]`, falling back to `device` when filtering empties it.
**The key is the identity; the comment is a label.** A key already enrolled on
that machine is reported as already present rather than failed — which is what an
existing Key B usually is.

## Grants are per machine

A device is not trusted globally. It holds a grant on each machine separately,
and a machine added next month grants nothing automatically. That is what lets
one account hold both a work machine and a personal phone without either reaching
the other.

The confirmation defaults to **only the machine being granted from**:

> ### Add "iPhone 17" to work@example.com?
>
> iPhone 17 will be able to run agents and commands on the machines you pick, as
> you. Only this account's machines are listed.
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

Adding a **Mac** shows the same list with the Key B choice above it:

> ### Add "MacBook Air"?
>
> **Far Cooler access** — run agents and terminals on the machines you pick.
>
> **Shell access** — Zed, git and Terminal on that Mac reach them too.
> ☑︎ New key — `farcooler-macbook-air` ⌄
> ☑︎ Add to `~/.ssh/config`
>
> ☑︎ MacBook Pro · this Mac
> ☐ box
>
> **[ Add Mac ]**  [ Cancel ]

**The tap demands local authentication**, every time — Touch ID, Face ID or the
passcode, through `LocalAuthentication`, evaluated at the moment of the tap. Not
a session, not an unlock from an hour ago. This is what a person standing at your
unlocked laptop runs into, and it is the only thing between them and an enrolled
device. The WorkOS session must also be fresh: `max_age` checked against
`auth_time`, which prerequisite 4 makes available.

Afterwards the same list is **Settings › Devices › iPhone 17**, a checkbox per
machine, with Far Cooler access and shell access as separate rows for a Mac. The
confirmation grants `control`; `read` is set from that screen and offered for
phones only, since a shell key cannot be held to a scope.

**Revocation closes live sessions**, per `farcooler-design.md:893` — including
the multiplexed master, not only the current channel. See prerequisite 3.

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
look unreachable rather than hostile. An earlier draft answered all of those with
"not authorized" and told the user to append the key again — which produces
duplicates and trains people to loosen sshd settings until the error goes away.
**No screen in this flow ever recommends relaxing a security setting**, and where
the client cannot know why sshd refused, it says so.

**Names come from the fence.** The comment is `farcooler-<name>-<fp8>`, so a
machine lists its own devices with no network at all.

**The relay cannot choose which key to install**, because it holds no key — only
a fingerprint. Every enrollment sources the key from ground truth: a completed
ceremony, or the fence of a machine that already holds it.

**An address that changes is redistributed over the protocol.** A daemon reports
its own reachability on every authenticated connection, so a client that can
still reach it updates its `~/.ssh/config` block. A machine that moves while
nothing can reach it keeps a stale block on every client, and the ceremony or the
manual path is how it comes back — host key pinning means the failure is an
outage rather than a takeover.

## One device, several accounts

A phone reaches a work laptop, a work VM, a personal Mac and a Linux box at home.
Two of those belong to an employer and two do not, and no arrangement of one
account per device serves that person.

So a device holds **as many accounts as you sign into**, and Key A belongs to a
`(device, account)` pair rather than to the device. This is the same rule that
the previous section arrived at from a different direction — and holding both
accounts at once is what makes it useful rather than merely safe, because the
alternative it replaces is signing out of one to use the other.

**A machine belongs to exactly one account.** Its daemon holds one pairing
(`crates/daemon/src/push.rs`), so an account is a property of the machine, not of
the connection. Moving a machine between accounts is re-pairing it, which is
deliberate and visible. Everything else follows from that: an action is
unambiguous because the machine it targets names the account, and no screen ever
has to ask which one you meant.

| | |
| --- | --- |
| The fleet | One list, machines from every account, each labeled with its own. Consistent with a product whose whole premise is that every machine is present at once. |
| Devices | Per account. Work's Settings › Devices lists work devices; it has no idea the personal ones exist. |
| The ceremony | Per account. The new device picks which account it is joining, and the QR names it. |
| Sign-out | Per account, and takes only that account's key and grants with it. |
| Notifications | Named by account, since two accounts can both reach the same phone. |

The QR carries the account id — an opaque identifier, not a credential — so the
trusted device knows which of its accounts to check against, and the confirmation
sheet says which one it is adding to. A QR naming an account the scanning device
does not hold is refused with the copy in step 2.

What this does **not** do is put a boundary between the two accounts on the
device itself. Both keys sit in the same Keychain, and a compromised phone
compromises both. An employer wanting a real boundary needs a managed device, not
a second account in a personal app, and the document does not pretend otherwise.

### The shape this leaves for teams, and the thing it cannot do

An account is a WorkOS account, and WorkOS's reason to exist is organizations,
SSO and directory sync. So "work account" becoming a real team with several
people, an admin and a bill is a change of plan rather than a change of
architecture, and the per-account key model already gives each organization its
own devices, its own ceremonies and its own revocations.

One limitation should be written down now, because it is the first thing an
administrator will ask for and this design deliberately cannot provide it:

**There is no central view of who can reach what, and no central revocation.**
Access lives in `~/.ssh/authorized_keys` on machines the relay cannot reach and
holds no address for, and the relay stores fingerprints rather than keys. An
admin can see which devices exist and can stop a person signing in; they cannot
enumerate that person's grants, and they cannot remove a key from a machine they
have no route to. Removing someone from the organization ends their sessions and
their notifications, and leaves their key in the fence of every machine they were
enrolled on until a device that reaches those machines removes it.

That is the same property that makes the relay untrusted, stated from the other
side. Selling centrally-administered access would mean the relay holding
addresses and grants, and a machine-side agent that obeys it — a different
product with a different threat model, and a decision to make on purpose rather
than by drifting into it.

## What signing out does not do

Key A is stored under the account's identifier and never reused across accounts.

A key enrolled on your machines outlives any session: **signing out removes
nothing from anyone's `authorized_keys`.** Without per-account keys, a phone that
signed out of one account and into another would keep SSH access to the first
account's machines and carry it silently into the second — a confusing device
list for one person with two accounts, and a stranger on the previous owner's
machines when a phone changes hands.

| | |
| --- | --- |
| Sign out, sign back into the same account | The same key is still there and still enrolled. Nothing breaks. |
| Sign into another account | A fresh key, enrolled nowhere. That account's ceremony starts clean. |
| A signed-out account's key | Dormant, unused, and still in that account's machines' fences — which is what the screen below is for. |

Regenerating on *every* sign-out would be the wrong reflex: an accidental
sign-out would cost every machine and a trip to run the ceremony again.

**Signing out says what it does not do**, and offers to do it, because at that
moment the device still holds the access it is about to stop managing:

> ### Sign out of work@example.com?
>
> Signing out doesn't remove this iPhone's access to that account's machines. Its
> key stays in `~/.ssh/authorized_keys` on **work-mini** and **build-vm** until
> it's removed.
>
> ☐ Also remove this iPhone's access to those machines
>
> Your other accounts on this iPhone aren't affected.
>
> **[ Sign Out ]**  [ Cancel ]

Unchecked by default, because most sign-outs are temporary. Checked, it revokes
itself from each machine it can still reach and names the ones it cannot, the
same as "Remove Device" from elsewhere.

## Where a phone's key lives

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

Say that, and tighten what can be tightened. `kSecAttrAccessibleWhenUnlocked-
ThisDeviceOnly`, so it is never in a backup and never on a restored device; an
access control requiring biometry or passcode, since this key grants access to
others; on Android, request hardware backing, verify it with `KeyInfo`, and
require user authentication rather than disabling it. None of these make the key
non-exportable once decrypted, and the document does not claim they do.

**The real fix is a P-256 key in the Secure Enclave**, which OpenSSH accepts as
`ecdsa-sha2-nistp256` and which Face ID can gate per use. The blocker is
concrete: russh is handed a private key today (`PrivateKeyWithHashAlg`,
`crates/client/src/ssh.rs:119`), and a Secure Enclave key signs through a
callback instead. That is a transport change and belongs in its own document.
Until it lands, "stolen unlocked device" and "cloned device" are the same threat.

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

**Adding a machine.** It appears with a label, granted to nothing.

| | |
| --- | --- |
| A Mac you are sitting at | Install the app. Local daemon over a Unix socket — no key, no SSH. |
| Remote server, from a Mac | Pick it from `~/.ssh/config` or type `you@box`. Your existing SSH installs the daemon. |
| Remote server, from a phone | Through a machine that already reaches it, using *its* access. |
| Remote server, phone alone | Manual — put the key on the box with the access you already have. |
| Any time | `farcooler host install you@box`. |

**Adding a device.** A phone or a Mac, the same ceremony either way. Both devices
signed into the same account, and a fingerprint at the confirmation.

| | |
| --- | --- |
| With a trusted device | Show the QR, have it scanned, pick machines, confirm, scan the QR it shows back. |
| A new Mac, no trusted device | It has local authority over itself, so it grants other devices from itself. |
| A new phone, no trusted device | Manual. It becomes a root once enrolled. |
| Either device has no camera | Manual. No weaker ceremony is offered. |
| Any time | Copy the key, paste into `authorized_keys`. |

**Granting an existing device access to a machine.**

| | |
| --- | --- |
| From the device that wants it | Ask a machine it already reaches to enroll it on the new one. That machine returns the address on the connection you already hold. |
| From the machine itself | A Mac you are at: Settings › Devices → check the device. Local write. |
| For some other device, remotely | Not offered. Grant from that device — its Settings › Devices is read-only elsewhere and says so. |
| No machine in common | The ceremony in person, or the manual path. |
| Any time | Paste the key into `authorized_keys` yourself. |

## Threat model

| Adversary | Outcome |
| --- | --- |
| Anonymous internet | Cannot begin. Nothing that authorizes an enrollment travels over the network. |
| **Someone at your unlocked laptop** | **Refused at the confirmation**, which demands a fingerprint or passcode they do not have. An account check would not have helped: that laptop is signed in, and they would be using your session. |
| A device that is not on your account | Refused before the confirmation, when the trusted device asks the relay whose key it just scanned. Also the answer to an honest mistake: a second account's phone would otherwise hold keys on your machines while appearing in a device list you cannot see. |
| WorkOS account takeover | Cannot enroll anything — enrollment needs a QR scanned in person and a local authentication, and later grants never cross the network. Can read the roster of device labels, machine labels and fingerprints, and can push notifications. |
| Compromised relay or D1 | Carries no key, no address and no grant. Its one role is the account lookup, where a false *yes* still needs a QR held to your camera and a fingerprint, and a false *no* is a refusal. Learns device labels, machine labels and fingerprints, and can push notifications. |
| Someone filming the QR exchange | **Nothing.** Two public keys and a list of addresses they cannot reach. There is no secret in either code. |
| MITM between an enrolling device and a machine | Refused — the host key is pinned from a manifest that came through a camera. |
| Hostile key bytes | Rebuilt from key data alone, Ed25519 only, CR/LF refused before parsing. |
| A compromised `control` device | **Full host-user authority, by design.** It can enroll its own keys and install persistence that survives revocation. Removal lists what it enrolled; the documented recovery is revoke, then audit. |
| A `read` device | Genuinely confined by `restrict` plus a forced command — once prerequisite 1 makes the scope real. |
| **A Mac's Key B, once enrolled** | **A shell, by design.** Key A keeps its Far Cooler sessions identifiable and revocable; Key B is not claimed to be contained, and removing it does not close an open shell. |
| **A device that changes hands** | The new owner signs into their own account and gets a **fresh key enrolled nowhere** — keys are per account, so the previous owner's access is not inherited. The old key stays dormant on the device and in the old machines' fences until removed, which is what the sign-out screen offers and what "Remove Device" does from elsewhere. |
| **One account's holder, reaching for another's machines on the same device** | Refused by the product: a ceremony lists only its own account's machines, and each account's device list is its own. **Not refused by the operating system** — both keys are in one Keychain, so a compromised device compromises both accounts. An employer needing a real boundary needs a managed device. |
| Stolen unlocked trusted device | **Works.** It already holds access. Mitigated by biometry on the confirmation, fresh authentication, and revocation from any other device. |
| **Cloned phone key** | **Works, and is currently undetectable.** The key is a software key; see above. The largest unmitigated risk in the design. |
| Every device lost | Ordinary SSH plus `farcooler client revoke`. No account-recovery bypass, deliberately. |

Requiring TOTP on the WorkOS account is defense in depth, not the defense. It
does not apply to SSO users, so it is not something to lean on.

## Testing

- **The ceremony.** A QR scanned outside the scanner's own freshness window is
  refused regardless of any timestamp inside it; the same code scanned twice
  produces two ceremonies, not one repeated; a manifest larger than one code
  reassembles by index and refuses a partial set.
- **Both gates.** A key belonging to no device on the account is refused before
  the confirmation appears; a failed or cancelled `LocalAuthentication` enrolls
  nothing; a relay that denies the account check produces a refusal and never a
  silent success.
- **Key rendering.** Golden files: an embedded newline, a leading options field,
  two keys in one value, trailing text, a hostile comment, a non-Ed25519 key.
  Each produces a refusal or exactly one canonical line that parses back to the
  same key data with the comment we chose.
- **Fence safety.** `authorized_keys::fence_safety` extended to cover enroll and
  revoke, a file with no trailing newline, a symlinked `.ssh` swapped between
  check and rename, wrong modes, an `AuthorizedKeysFile` outside the home
  directory, and two concurrent enrollments.
- **Revocation.** A client revoked while its enrollment is in flight does not
  commit; revoking a key closes the multiplexed master and not only the channel;
  removing a device lists every key it enrolled.
- **Key A selection.** A connection made while a master authenticated with
  another key is alive still authenticates as Key A — the test that
  prerequisite 3's `ControlPath` change exists to make passable.
- **`~/.ssh/config`.** The block lands above any `Include`; an existing pattern
  matching the alias causes a suffix and a message; a hand-written `Host` block
  is untouched; a damaged fence refuses; every byte outside the fence survives.
- **Key A never leaks into ssh config**, and deleting Far Cooler's block leaves
  the app working and only Zed broken.
- **The alias reaches the editor.** A workspace on a registered machine produces
  an `ssh://` URL naming the alias.
- **Later grants.** A device asking a machine it reaches to enroll it elsewhere
  receives the address on that same connection and nothing through the relay; a
  device with no machine in common is told to run the ceremony rather than
  offered a remote path.
- **The relay holds no key.** Its device rows carry a fingerprint, and every
  enrollment path refuses a key that did not come from a ceremony or a fence.
- **Keys are per account.** Signing out and back into the same account keeps the
  key and every grant; signing into a different one produces a different
  fingerprint enrolled nowhere, and never offers the first account's machines.
  Two accounts on one device hold two keys that are never interchanged.
- **Two accounts at once.** A device signed into both lists every machine from
  both, labeled; each account's Settings › Devices lists only its own; a ceremony
  for one account never offers the other's machines; and signing out of one
  leaves the other's key and grants untouched.
- **Scope.** A `read` client cannot enroll, cannot revoke, and cannot reach a
  `control` operation.
- **Derivation.** A device whose line is deleted by hand reports *not
  authorized*; a device sshd refused reports *refused*, and neither screen
  suggests changing an sshd setting.

## Deferred

- **P-256 in the Secure Enclave**, with russh signing through a callback. Its own
  document; until then a phone's key is a software key and the threat model says
  so.
- **Key rotation and `expiry-time=`.** OpenSSH 8.2+ expires an enrolled key by
  date with no revocation step, which is the cheapest available answer to a clone
  nobody noticed.
- **A camera-less ceremony.** Every attempt so far has been broken by review, and
  the manual path already covers the case honestly.
- **Granting another device access remotely.** Removed deliberately: it was the
  only case that needed the relay to carry anything, and self-service covers it
  at the cost of a habit nobody has formed.
- **Bulk grant.** "Every machine" is one checkbox away and is deliberately not
  offered, because the default this document argues for is the opposite.
