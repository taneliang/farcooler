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
