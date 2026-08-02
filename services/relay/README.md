# Overnight relay

The only server Overnight has, and it exists for one reason: a phone that is
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

    account ──owns──> devices  (apns / fcm token)
            └─owns──> daemons  (hashed bearer token, scoped to the account)

- **Identity** is WorkOS. The relay verifies a WorkOS session JWT and never
  stores a password, an email/password flow, or a session of its own.
- **Storage** is D1: accounts, devices, daemons. Small, relational, and the
  same everywhere.
- **Metrics** are Analytics Engine, deliberately NOT D1 — see `analytics.ts`.

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
