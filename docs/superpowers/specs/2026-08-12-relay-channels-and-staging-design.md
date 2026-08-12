# Design: one relay per channel

Date: 2026-08-12
Status: DESIGN — not yet approved, not implemented
Completes `docs/superpowers/specs/2026-08-11-release-channels-design.md`, whose
step 4 named the relay work but did not describe it.

## Problem

The channels design partitions everything on a machine — runtime directory,
database, tmux server, binary name, service unit, bundle identifier — and stops
at the network. Behind that boundary sits one relay, one D1, one WorkOS project
and one APNS topic, shared by every channel.

That leaves one thing broken and one thing missing.

**Broken:** `services/relay/src/push.ts:67` sends a single header,
`'apns-topic': env.APNS_TOPIC`. That value must equal the *receiving app's
bundle identifier*, and each channel now has its own. So the moment a beta app
registers a push token, its notifications go out under the release topic, APNS
rejects the pairing, and nothing says so — `sendApns` returns a boolean nobody
surfaces. It breaks the feature beta testers are most likely to be testing.

**Missing:** `.github/workflows/relay.yml` deploys to production on every push to
`main` touching `services/relay/**`, migrations included. There is no review gate
and nowhere to try a change first, so every relay change is tried for the first
time on real accounts.

## The partition continues through the server

**One relay per channel.** Three deployments, three D1s, three WorkOS
environments, three hostnames.

| | dev | beta | release |
| --- | --- | --- | --- |
| Worker | `farcooler-relay-dev` | `farcooler-relay-beta` | `farcooler-relay` |
| Hostname | `relay-dev.farcooler.com` | `relay-beta.farcooler.com` | `relay.farcooler.com` |
| D1 | `farcooler-dev` | `farcooler-beta` | `farcooler` |
| WorkOS | its own environment | its own environment | production |
| `APNS_TOPIC` | `com.farcooler.ios.dev` | `com.farcooler.ios.beta` | `com.farcooler.ios` |

The release column is unchanged. Nothing already deployed moves, no data
migrates, and every client in the field keeps talking to exactly what it talks
to now.

### Why this beats one channel-aware relay

An earlier draft of this document argued for a single relay with a `channel`
column, on the grounds that an account is one account and separate databases
would mean re-pairing every machine when you install the beta. That argument was
wrong, for two reasons.

**Accounts do not carry across anyway.** `accounts.id` is the WorkOS user id, and
a beta on its own WorkOS environment issues ids from a different namespace. The
same person is already two rows. There is no shared account to preserve, so
there is nothing for a shared database to buy.

**Sharing would push a decision in front of authentication.** `verifySession`
fetches the JWKS for `env.WORKOS_CLIENT_ID` (`workos.ts:30`), and the code
exchange spends `env.WORKOS_API_KEY` (`index.ts:146`, `:165`). One relay serving
two WorkOS environments would have to choose which credentials to verify a token
against *before* it knows who the caller is — keying auth off an unauthenticated
field. That is a worse shape than a second deployment, not a better one.

And re-pairing is a cost paid by the handful of people deliberately running a
pre-release build, not by users. A beta here is closer to a dev build than to a
parallel production population.

### What it costs in code: nothing

This is the part that decides it. Every per-channel value is **already** a
`[vars]` entry or a `wrangler secret`:

| Value | Where it lives today |
| --- | --- |
| `WORKOS_CLIENT_ID` | `[vars]` in `wrangler.toml` |
| `WORKOS_API_KEY` | secret |
| `APNS_TOPIC`, `APNS_KEY_P8`, `APNS_KEY_ID`, `APNS_TEAM_ID` | secrets |
| `FCM_SERVICE_ACCOUNT` | secret |
| D1 binding | `[[d1_databases]]` |

So `services/relay/src/**` needs no change at all. No `channel` column, no topic
derivation, no cross-channel delivery rules, no migration. Each deployment is
single-tenant for its channel and cannot see another's data — which is the same
property the channels design gives a daemon, arrived at the same way.

Several problems the previous draft had to solve stop existing rather than
getting solved:

- **The APNS topic** is a per-deployment secret naming that channel's bundle id.
  No derivation, no validation of a channel string, no way to produce a topic no
  app owns.
- **Cross-channel notification** is structurally impossible. A beta daemon and a
  release app cannot reach the same database, so there is no rule to write and
  none to get wrong.
- **`NULL` handling and backfill** do not arise. The production D1 keeps serving
  exactly the population it serves now.

### `wrangler.toml`

```toml
# Production stays the top-level configuration, unnamed, so `wrangler deploy`
# with no --env is still the release deploy and every existing command keeps
# working.
name = "farcooler-relay"
routes = [{ pattern = "relay.farcooler.com", custom_domain = true }]

[env.beta]
name = "farcooler-relay-beta"
routes = [{ pattern = "relay-beta.farcooler.com", custom_domain = true }]

[[env.beta.d1_databases]]
binding = "DB"
database_name = "farcooler-beta"
database_id = "REPLACE_WITH_BETA_D1_ID"

[env.dev]
name = "farcooler-relay-dev"
routes = [{ pattern = "relay-dev.farcooler.com", custom_domain = true }]

[[env.dev.d1_databases]]
binding = "DB"
database_name = "farcooler-dev"
database_id = "REPLACE_WITH_DEV_D1_ID"
```

**Pinning the routes matters on its own.** `wrangler.toml` declares none today,
so `wrangler deploy` publishes to `farcooler-relay.<subdomain>.workers.dev` —
while every client defaults to `https://relay.farcooler.com`. If that hostname
answers, it is bound in the Cloudflare dashboard: a click that exists in no diff,
no review and no backup, for a hostname compiled into binaries in the App Store
that cannot be told a new one for days. Wrangler treats an already-bound domain
as a no-op, so adding it is safe either way.

**Provisioning can be lazy.** Release exists. Beta is needed when the first beta
ships. Dev's is defined here but need not be created until someone wants push
from a dev build; until then a dev build's registration fails visibly, which is
the honest behaviour for a build whose whole point is that nothing is settled.

### Where each client points

Derived from the channel it was built for, beside every other per-channel value:

```rust
// crates/daemon/src/push.rs
pub fn default_relay() -> String {
    match farcooler_protocol::CHANNEL {
        Channel::Release => "https://relay.farcooler.com",
        Channel::Beta => "https://relay-beta.farcooler.com",
        Channel::Dev => "https://relay-dev.farcooler.com",
    }
    .to_string()
}
```

The same three-way derivation in `Account.swift:37` and `Account.kt:413`, from
the channel each app already stamps into its bundle.

### The override, on every channel

An advanced setting pointing any build at any relay — deliberately not
debug-only, because its purpose is testing an old app against a new relay, and
the app you most want to test that way is a release build.

The field already exists on all three clients and is already a defaulted setting
rather than a constant:

| Client | How |
| --- | --- |
| Daemon | `relay` in `push.json` (`crates/daemon/src/push.rs:28`) |
| iOS / macOS | `Account.relay`, a `UserDefaults` key (`Account.swift:32`) |
| Android | the same override beside `Account.DEFAULT_RELAY` (`Account.kt:413`) |

The Swift accessor's own comment already anticipates this: *"A setting so
self-hosting is configuration rather than a fork, and so a development build can
point at `wrangler dev`."* So what is missing is only a way to **reach** it on a
phone.

**It belongs behind an Advanced disclosure, and it should say what it does.**
Where a person's notifications go is worth changing on purpose and not by
accident: someone talked through changing it by a caller claiming to be support
has been phished, and the screen should read that way — the current value
visible, a way back to the default, and the machine's own pairing invalidated
when it changes, since a token issued by one relay means nothing to another.

## Deployment

Each relay gets its code the way its channel does, so the deploy story is the
same shape as the app one rather than a second story to learn:

| Trigger | Target |
| --- | --- |
| push to `main` touching `services/relay/**` | dev, continuously |
| promote to beta | beta, from `promote.yml`, at the tagged commit |
| promote to release | release, from `promote.yml`, at the tagged commit |
| `workflow_dispatch` | any of them, from any ref |

`relay.yml` used to deploy **production** on every push to main, and its own
comment said it had to: an App Store release takes days and can never be rolled
forward on demand, so there will always be phones running an app from months ago
that the relay must keep serving.

That is true, and it argues for **additive-only** — which is what actually keeps
a months-old app working. It does not argue for shipping every commit straight to
the people running those apps with nothing in between. So the continuous cadence
moves to dev, and the ability to roll forward on demand stays as
`workflow_dispatch`, which can put any commit on the release relay in one press.

**Beta and release are called from `promote.yml` rather than triggered by their
tag**, and that is not a preference. A tag pushed with the default
`GITHUB_TOKEN` raises no event — GitHub suppresses those to prevent recursion —
so `on: push: tags` in `relay.yml` would look correct, never fire, and leave the
beta relay silently months behind the app talking to it. It is the same trap
`promote.yml` already works around by building in a job that `needs:` its tag
job.

That job depends on the **tag only, not the app build**. The relay is
additive-only, so a newer one serves an older app by construction; making a relay
deploy wait on a Swift compile would couple two things with no reason to fail
together, and a signing failure would strand the relay.

Migrations run before the code on every path, for the reason the workflow already
states: the previous Worker is still serving while the new one rolls out.

The `vars.RELAY_DEPLOY == 'true'` gate stays, and so does its comment — `secrets`
is not available in an `if`, so `secrets.X != ''` compares `''` to `''` and is
always false. It fails closed, which is exactly why it looks like it works.

### What a non-release relay is not

**Not a place to break the additive-only rule.** A beta relay catches mistakes;
it does not license them. A destructive migration is destructive wherever it
runs, and the only difference is whose data.

**Not a rehearsal for the apps.** An app pointed at a beta relay is testing the
relay. The apps have their own channels for that.

## The marketing site

The same partition applies and is worth stating, but the site does not exist yet
and this document does not design it. When it does: `farcooler.com` serves the
release download, a staging deployment serves changes to the site itself, and the
beta's install link is TestFlight rather than a download — so the site's channel
axis is its own deployment lifecycle, not the app's.

The one rule that carries over now, before anything is built: the public download
must be the **release** build. `release.yml`'s publish job already marks a beta
as a GitHub prerelease so it never becomes `/releases/latest`, which is the
mechanism a download page would read.

## Sequence

Each step stands alone, and none breaks a client in the field.

1. **Pin the release route** in `wrangler.toml`. Independently worth doing: it
   takes a hostname compiled into App Store binaries out of a dashboard click.
2. **The beta environment** — its D1, its WorkOS environment, its secrets, its
   route — and the `workflow_dispatch` input that deploys to it.
3. **Per-channel relay defaults** in the daemon and both apps, derived from the
   channel each was built for.
4. **The advanced override**, reachable on every channel, with the pairing reset
   and the copy that makes changing it deliberate.
5. **The dev environment**, when someone first wants push from a dev build.

Step 1 is worth doing whether or not the rest lands. Steps 2 and 3 together are
what makes a beta app's notifications work at all.

## What this does not solve

**Nothing tests the additive-only rule on the relay.** The wire has
`scripts/proto-lint.py` now; the relay's equivalent — a check that a migration
adds only nullable columns and that no route disappeared — does not exist. A beta
relay makes a mistake survivable, which is not the same as catching it.

**Three deployments is three sets of secrets to keep in step.** The failure is
quiet: a beta relay with the release `APNS_TOPIC` reproduces exactly the bug this
design exists to fix, and nothing would report it. Worth a startup check that the
topic matches the channel the Worker was deployed as — which means the Worker
knowing its own channel, as a plain `[vars]` entry per environment.

**Android has no release path at all**, so its channel story cannot be exercised.
The channels design records the same gap. If beta and dev Android apps end up in
separate Firebase projects rather than one, each relay needs its own
`FCM_SERVICE_ACCOUNT` — which this design already gives it.
