# Far Cooler relay

The only server Far Cooler has, and it exists for one reason: a phone that is
asleep cannot be reached over ssh. Everything else in this product is
daemon-to-device over a connection the user already owns; push is the exception,
because Apple and Google will only deliver a notification that came from a
service holding their credentials.

## What it is not

It is not a backend for the fleet. It never sees a transcript, a repository, a
command, or a terminal's output. It knows: which accounts exist, which devices
they own, which machines they have paired, and — as counters — how much all of
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
            └─has───> live_activities  (one update token per terminal)

- **Identity** is WorkOS. The relay verifies a WorkOS session JWT and never
  stores a password, an email/password flow, or a session of its own.
- **Storage** is D1: accounts, devices, daemons. Small, relational, and the
  same everywhere.
- **Metrics** are Analytics Engine, deliberately NOT D1 — see `analytics.ts`.

## Live Activities

A `/v1/notify` that carries a `status` also drives the lock-screen card for that
terminal, on top of the alert. Three tokens are involved and none is
interchangeable:

- the **device token**, for the alert;
- the **push-to-start token**, one per app install, which is the only way to
  create a card while the app is not running — which is the whole point, since
  the agent blocks while the phone is in a pocket;
- the **update token**, issued per running activity and dead when it ends, which
  the app reports to `/v1/devices/activity` and withdraws with `updateToken:
  null`.

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

## Sandbox and production

A locally-signed build has `aps-environment: development` and so holds a
*sandbox* APNs token, which `api.push.apple.com` rejects with `BadDeviceToken`.
`devices.environment` records which service issued the token; NULL means
production, because that is what every device registered before the column
existed must have been.

## Self-hosting

The relay URL is a client setting. This code is in the repo and the secrets are
in `wrangler` bindings, so running your own is a deploy rather than a fork.

## What is not built yet

The daemon side. It needs to POST to `/v1/notify` with its bearer token, and
this workspace has no HTTPS client — the dependency list is deliberately small
and every network path so far has been ssh, which is a subprocess. Adding
`reqwest`/`rustls` is a real decision rather than an oversight, and it is one
line of `Cargo.toml` once made.

The alternative is shelling out to `curl`, which needs no dependency and is
worse: no timeout control worth the name, no connection reuse across a night of
notifications, and an error surface that is a string.
