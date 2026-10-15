# Far Cooler relay

The only server Far Cooler has, and it exists for one reason: a phone that is
asleep cannot be reached over ssh. Everything else in this product is
daemon-to-device over a connection the user already owns; push is the exception,
because Apple and Google will only deliver a notification that came from a
service holding their credentials.

## What it is not

It is not a backend for the fleet. It never sees a transcript, a repository, a
command, or a terminal's output. It knows: which accounts exist, which devices
they own, which runners they have paired, and — as counters — how much all of
that gets used.

## The rule that makes it safe

**A daemon never names the destination.** It says "notify my user"; the relay
resolves which devices that means. A daemon token is therefore worth exactly one
thing to a thief: the ability to notify its own owner's phone. Annoying,
revocable from the app, and useless as a spam vector.

Contrast with the obvious design — accept `(device_token, payload)` — which is
one leaked credential away from being an open gateway to anyone whose token you
can guess or steal.

## Shape

    account ──owns──> devices          (apns / fcm token, + push-to-start token)
            ├─owns──> daemons          (hashed bearer token, scoped to the account)
            └─has───> install_cards    (one update token, one card, per install)

- **Identity** is WorkOS. The relay verifies a WorkOS session JWT and never
  stores a password, an email/password flow, or a session of its own.
- **Storage** is D1: accounts, devices, daemons. Small, relational, and the
  same everywhere.
- **Metrics** are Analytics Engine, deliberately NOT D1 — see `analytics.ts`.

## The team directory

`/v1/directory/*` (`src/directory/`, migration 0021) is phase 1 of the teams
design (`.claude/agent/reports/ov-480/design.md`, ov-487): teams with admin,
member and billing roles, a personal team per person, invite links, runners
registered into a team with their install id, who the directory says may reach
each runner, each person's devices with their public keys, and an append-only
audit log.

It holds **public keys only**, each admitted with an SSH signature made by its
private half (`src/sshsig.ts`, the format `ssh-keygen -Y sign` writes). A
person's first device is trusted on first use, and every later one needs a
certificate signed by one of their trusted devices, so the directory can't add
a device to anyone's profile on its own. A device proves its key over a
challenge the relay chose (`devices/challenge`, migration 0024), which the
registration spends, so a copied proof registers nothing (ov-549). A team's
admins can see each member's trusted and certified Key A fingerprints in
`teams/members`; nobody else sees any.

That last part is behind **`DEVICE_KEYS_ENFORCED`** (ruling R-72), off by
default: unset, `teams/members` sends no `keys` at all, which the teammate rule
(`crates/client/src/ceremony/teammate.rs`) reads as "not told" and so checks
nothing. Set it to `1` (a `[vars]` entry per environment) only once the apps
that certify in person have shipped: the same-account ceremony signs a
certificate for the Key A it scanned and posts it to `devices/certify`, naming
the device by that key (ov-550). Before that, only each person's first device
is listed, and every admin's scan of a teammate's other devices is refused.

First-use trust can't tell a person's real first device from one a stolen
session registered first, so the person is told (ov-549 review M1). The record
is `directory/me`, which lists every device key with `createdAt` and
`revokedAt`, and no route deletes a row: each signed-in app shows a banner for
any device added or removed since it last showed one, until the person
dismisses it, and its Settings lists the keys with their key words, the date
each was added, and Remove. A push to each app install
(`src/directory/notices.ts`, on Android its own `account.devices` channel) is
only a hint, since the same session could delete every push install first.

An account with no trusted or certified device left starts over (ruling R-75,
ov-561): the next of its devices to register, a new one or a pending one
signing in again, is trusted on first use, as a first device is. That's how a
person who removed an impostor that was trusted first gets their own devices
back, with no support desk or admin step. `trustedAt` in `directory/me`
(migration 0025) dates it, and each app's banner announces it. The risk is a
stolen session that removes every confirmed device and then signs in: each
removal and the new trust show on every app, but the person has to end that
session too, or it can do the same again.

So a device trusted by that reset waits out a hold (ruling R-76, ov-565,
`src/directory/hold.ts`): for 72 hours (`TRUST_RESET_HOLD_MS`) its Key A is
left out of the `keys` that `teams/members` sends a team's admins, and so is
every device it certifies in that time. It's listed in `held` instead, with
the moment the hold ends, and `directory/me` gives the same moment as
`heldUntil`, which each app shows on the device's row. That's the person's
time to see the reset announced and remove a device that isn't theirs before
any admin's scan takes it as theirs. A person recovering their own account
needn't wait: an admin of their team who scans the device with them in person
confirms it (`teams/devices/confirm`, migration 0027), and that team lists it
at once. The confirmation is for that team only, never the admin's own
account, and never a later reset. Each confirmation is pushed to the person
and kept in `directory/me` as the device's `holdConfirmations`, so every
app's banner announces it even if the push never arrives.

A certified device whose chain of vouchers doesn't reach a trusted device (a
voucher removed, sent back to pending, or a cycle) is listed nowhere, in
neither `keys` nor `held`. A certified device whose node key changes goes back
to pending and takes everything it certified with it, as a removal does.

A known window (ov-565 review Low 3): a device certified by a held one shares
that device's hold, so one certified late in it waits only what's left. Its
own arrival is announced as soon as it registers, but the person may have
less than 72 hours to see that before the team's admins are sent its key.

A device keeps the label it first registered with (ov-560 R2), and every label
is stored and shown without format characters (bidi controls, zero-width
characters) or quote marks and their lookalikes, the apostrophe aside (R1).

A known limit (ov-549 review N2): on an iPhone or Android phone, Key A belongs
to the phone, whoever is signed in, and is the key every runner it was enrolled
on already holds. So a second account signed in on the same phone gets
`409 claimed` and is never listed; an admin's scan of that phone for the second
person is refused. Listing it would tell admins the key is that person's while
it also opens the first account's runners. Per-account phone keys, as the Mac
has, need re-enrollment and their own change.

The same one key makes a second limit (ov-565): a phone whose Key A was
removed, by its person or by a stolen session, can never be listed again,
because a revoked key can't be registered again (`409 revoked`) and the phone
has no other key to offer. It fails closed: the phone keeps working on the
runners that already hold its key and on its person's own runners (the
same-account ceremony doesn't ask the directory), and only an admin's scan for
a teammate's runner refuses it. **Make a New Key** in the phone's Settings is
the way back (ov-569):

1. The phone makes a new Key A and registers it like any new device, so it's
   pending unless R-75's reset trusts it.
2. It's confirmed the usual way: by the old key, on the phone, if that key is
   still confirmed, and otherwise by scanning the phone's code from another
   confirmed device.
3. The phone moves its own line on each runner it reaches to the new key,
   through the runner's `client.rekey`. The runner never asks the relay.
4. It revokes the old key here, naming the new one as `successor` when the
   old key confirmed it. Then `devices/revoke` moves what the old key
   confirmed to the successor rather than back to pending, and the successor
   takes the old key's place: its trust, with the same `trustedAt` and
   reset flag so an R-76 hold runs to the same end, or its voucher. The
   successor must be a device the old key certified (its `certified_by` is
   the old key's device). Only the old private key can sign that certificate,
   so a session without the phone can't name a device of its own.

A runner is identified by its **runner key**, an Ed25519 key it keeps in its
own runtime directory, never by its install id, which any Unix user on the
host can read from `ps`. Registering or claiming a runner needs a fresh
signature by that key over the account, team and install id, so only someone
who can make the box sign (a shell on it) can put it in a team. The daemon
side of that, making the key and signing on request, is not built yet.

It **records** access and never grants it. A runner's `authorized_keys` stays
the only thing sshd obeys; adding a key there will need a grant signed by a
runner admin's device (phase 4). Only removals may come from the directory
alone, because they can only take access away.

## Sign out everywhere

`POST /v1/account/signout-all` (`src/signout.ts`, migration 0026, ov-564) ends
every session on the caller's account.

- **Access tokens:** it stamps `accounts.sessions_revoked_at` and writes a
  `sessions.revoked_all` audit line in one batch. `requireSession`
  (`src/session.ts`, the only caller of `verifySession`) then refuses a token
  whose `iat` is within two seconds of the stamp or before it.
- **Refresh tokens:** a refresh keeps its `sid` and mints a fresh `iat`, so the
  stamp can't stop one. `sessions_seen` records the first time the relay saw
  each `sid`, and a token whose `sid` was first seen at or before the stamp is
  refused whatever its `iat`. Sign-out records every session WorkOS lists as
  seen at the stamp, revokes each at WorkOS, and stamps again afterwards.
- **Credentials no session guards:** it sends the other installs a "You were
  signed out everywhere" notice, then deletes every push row, pulse token and
  Live Activity or card row for the account. The apps register again at the
  next sign-in.
- **Runner pairings stay.** They aren't sessions. The answer lists the ones
  paired since the account last did this (or in the last 30 days) so the person
  can check them.

The answer is `{ ok, complete, ended, runners }`. `complete: false` means WorkOS
could not confirm every session ended. The stamp still holds and every session
the relay has seen is still refused; asking again retries WorkOS.

**What it can't do.** A session the relay has never seen, that WorkOS also
couldn't list, keeps working through a refresh until it is revoked upstream. And
a stolen session can run Sign Out Everywhere itself, which signs the person out
too. That is accepted: it costs the person a sign-in, the audit line records
it, and every other session ends with it.

## Live Activities

A `/v1/notify` that carries a `status` also drives the lock-screen card, on top
of the alert. Three tokens are involved and none is interchangeable:

- the **device token**, for the alert;
- the **push-to-start token**, one per app install, which is the only way to
  create a card while the app is not running — which is the whole point, since
  the agent blocks while the phone is in a pocket;
- the **update token**, issued per running activity and dead when it ends, which
  the app reports to `/v1/devices/activity` and withdraws with `updateToken:
  null`.

**One card per install, not per terminal.** Four running agents used to mean
four stacked cards, and a Dynamic Island that can present exactly one picking
between them. The card now LEADS with a single agent — `install_cards` remembers
which, because this worker holds nothing between requests — and the phone counts
the rest off the fleet snapshot it already keeps in its App Group. Blocked
outranks working, an agent that is not the leader never moves the card, and only
the leader's `done` ends it. `/v1/notify/retire` still names terminals, because
the runner is the side that knows whether a run is still behind one; at most one
of them can be the leader, and only that one takes the card down.

**The header counts what needs you, per machine.** Every notice may carry
`needsYou`, that runner's count of its asks, blocked agents, decisions and
reviews. The relay keeps the latest per machine on `daemons` (migration 0010),
overwriting and never adding. The card's `needsYou` is per runner: each
machine's count where it sent one, plus the blocked rows of the machines that
didn't (migration 0012 records whose row is whose). A machine silent for a day
is purged from the sum, a revoked one is gone with its row, and re-pairing under
the same label clears the old token's count. With no machine reporting, the card has no
`needsYou` and the app counts `blocked` rows as before. Two notices carry no
agent: `kind: "decision"` alerts and moves the card silently, and
`kind: "count"` only moves the card. Neither writes a roster row or starts a
card.

The alert is the guarantee and the card is the enhancement: an activity push
that fails is logged and dropped, never allowed to cost anyone the
notification. A daemon that sends no `status` gets the alert and nothing else,
exactly as before.

**Known hole.** A card the relay push-started has no update token until the
person next opens the app, so between those two moments the relay cannot end it
— an agent answered from the Mac leaves "Needs You" on the lock screen. The
start payload therefore carries a `stale-date` an hour out, which marks the card
as out of date without removing it. Deliberately not a `dismissal-date`: a card
that disappears on a timer while the agent is genuinely still blocked deletes
the one notification this product exists to deliver.

One card per install made that hole smaller rather than larger. A phone left in
a pocket while four agents start work now ends up with ONE unaddressable card
instead of four, because there is one row to claim and every later push either
moves that card or is refused as not the leader — so the single token the app
files when it next opens addresses the single card that exists. The window is
still "until the app runs"; what accumulates inside it no longer scales with the
fleet.

## Runner heartbeat

A paired runner beats: `POST /v1/heartbeat` every five minutes, carrying
nothing but its install id, its build, its own name (the computer's name on a
Mac) and how often it promised to beat. The relay stamps
`daemons.last_seen_at`, `beat_every` and `name` (migration 0015) and pushes
nothing. A runner unpaired on purpose sends one last beat marked `withdrawn`,
which clears `beat_every`, so it leaves the pulse instead of reading as lost.

A phone's widget, which can't see the app's links while the app is suspended,
posts its pulse token to `/v1/pulse` and gets back each beating runner's name,
pairing label, how long ago it was heard, and its promise; the phone decides
what "quiet" is. The phone makes the pulse token once and sends it on every
`/v1/devices` registration, so registrations are idempotent in any order. The
relay keeps only its hash, drops it when the device changes accounts, and the
token reads names and ages on its own account and nothing else. A runner too
old to beat never appears there, so its silence means nothing. See
docs/superpowers/specs/2026-09-30-runner-heartbeat-design.md.

## Sandbox and production

A locally-signed build has `aps-environment: development` and so holds a
*sandbox* APNs token, which `api.push.apple.com` rejects with `BadDeviceToken`.
`devices.environment` records which service issued the token; NULL means
production, because that is what every device registered before the column
existed must have been.

## Self-hosting

The relay URL is a client setting. This code is in the repo and the secrets are
in `wrangler` bindings, so running your own is a deploy rather than a fork.

## The daemon side, and the one dependency it cost

This section used to be called "what is not built yet", and the thing it said
was missing was the daemon end of all of the above: it needed to POST to
`/v1/notify` with its bearer token, the workspace had no HTTPS client, and
adding `reqwest`/`rustls` was a real decision rather than an oversight — one
line of `Cargo.toml` once made. That was true for the twenty minutes between
`a7cf97d`, which wrote this file, and `529fb26`, which made the decision. It has
been false ever since, and survived four later edits to this file.

The line is `Cargo.toml:54`. `rustls` rather than the platform's TLS, so a Linux
runner does not need OpenSSL headers to build the thing that watches its agents,
and `default-features = false` because nothing else `reqwest` ships is wanted on
this path. `crates/daemon/src/push.rs` posts to `/v1/notify` and
`/v1/notify/retire` with `bearer_auth` and the relay from the pairing file;
`watch.rs` calls the first when an agent starts needing its owner and the second
when a run stops being behind one.

Still true, and the reason to record all this rather than delete it: this is the
workspace's ONLY HTTPS client, and every other network path here is ssh, which
is a subprocess. The alternative considered was shelling out to `curl`, which
needs no dependency and is worse — no timeout control worth the name, no
connection reuse across a night of notifications, and an error surface that is a
string. That argument is the reason not to reach for a subprocess the next time
something here wants to make a request.
