# The Mac app updates itself

A Developer ID app has no App Store to update it. Today a new Far Cooler reaches
a Mac because someone notices a release exists, downloads a dmg and drags it over
the old one — which means canary, a channel that ships twenty times a day, is a
channel nobody actually re-installs twenty times a day.

This adds Sparkle, per channel, so an app tells you when its own channel has
something newer and installs it when you say so.

It builds directly on
[`2026-08-16-channel-app-identity-design.md`](2026-08-16-channel-app-identity-design.md):
the feed a build polls, the key it trusts and the app it replaces are all
per-channel facts, and none of them could be expressed before each channel's app
was its own app.

**iOS is not here.** TestFlight already does this, and better.

## The policy

Every shipping channel behaves identically: check on a schedule, show what is
available, install nothing without being told to.

| Key | Value | Why |
| --- | --- | --- |
| `SUEnableAutomaticChecks` | `true` | The point is to be told. |
| `SUAutomaticallyUpdate` | `false` | Nothing replaces itself unasked, on any channel. |
| `SUScheduledCheckInterval` | `86400` | Once a day. |
| `SUFeedURL` | per channel | `updates.farcooler.com/<channel>/appcast.xml` |
| `SUPublicEDKey` | per channel | Who is allowed to replace this app. |

**The same interval on canary as on stable**, which is not an oversight. Under
"always ask", a shorter interval does not deliver newer builds — it delivers more
dialogs, and a dialog dismissed twenty times a day is one nobody reads. Daily
means canary mentions once that there is something new. **Check for Updates…**
answers immediately for anyone who wants the newest right now.

**Local never checks.** A local build is the working tree of whoever built it,
and an updater offering to replace it with somebody else's build is a bug rather
than a feature. `build-app.sh` stamps no feed URL for local, and the app does not
start the updater when there is no feed — the same shape as an app that
registered no URL scheme being unable to receive a callback.

### What can be lost

Sparkle never swaps a running app: it installs on quit, or on relaunch. The
things worth minding — terminals, agents mid-edit — are held by tmux rather than
by the app or even the daemon, which is why restarting the daemon after an update
costs almost nothing.

The honest residual: the app reopened is a different build from the one closed,
and anything living only in the app's memory goes with it.

## Where the files live

```
updates.farcooler.com/
  stable/appcast.xml            enclosure -> the GitHub release asset
  preview/appcast.xml           enclosure -> the GitHub release asset
  canary/appcast.xml            enclosure -> canary/Far Cooler-<build>.dmg
  canary/Far Cooler-<build>.dmg one object per build
```

Only canary's binary is in the bucket. Stable and preview already publish
permanent, CDN-backed dmgs as GitHub release assets, and Sparkle signs a file's
bytes rather than its location, so pointing at them costs nothing and avoids
storing every release twice.

**One `<item>` per appcast.** Sparkle offers only the newest, and for stable the
GitHub releases page is already the history — a second, worse changelog in XML
earns nothing.

### Why the canary dmg is not a fixed path

Overwriting one `canary/Far Cooler.dmg` looks tidier and fails badly. A stale
appcast — an edge cache, a client that fetched an hour ago — names the old
version but resolves to the *new* bytes, so the signature in that appcast does
not match what was downloaded and Sparkle reports a **signature verification
failure**. That error is indistinguishable from tampering, and it is precisely
the error nobody should be trained to click past.

A per-build key turns the same situation into a 404, which reads as "could not
download, try later" and resolves itself on the next check. The build number is
the commit count, already in the dmg's filename, monotonic, and it names the
commit the build came from.

Stable and preview need none of this: a GitHub release asset is immutable per
version.

### Retention is a bucket rule, not a workflow step

An **R2 object lifecycle rule** deletes objects under the `canary/Far Cooler-`
prefix after 14 days — the dmgs only. The rule must not cover
`canary/appcast.xml`: that object lives under `canary/` too, and deleting it
would take the feed itself out from under every canary install, which would
then 404 silently on every check until the next push.

Pruning from CI would mean listing the bucket, and `wrangler r2 object` can only
get, put and delete by key — it cannot enumerate — so a workflow would have to
guess old keys or reach for the S3 API. The lifecycle rule needs no code, bounds
storage at roughly 3 GB at twenty builds a day (inside R2's free tier), and has a
side benefit: a stale appcast from inside the window still resolves to a real,
correctly signed, slightly older build rather than failing at all.

### The appcast is cached briefly

Five minutes of `Cache-Control`. Cloudflare's edge will otherwise serve
yesterday's canary feed for hours, and "the update exists but nobody is offered
it" is the hardest failure here to notice.

## Keys

Sparkle verifies an update with an EdDSA key pair, generated by its `generate_keys`
tool. One pair per channel.

**Private keys are GitHub secrets** — `STABLE_SPARKLE_KEY`, `PREVIEW_SPARKLE_KEY`,
`CANARY_SPARKLE_KEY` — used only by `sign_update` in CI.

**Public keys are committed to the repository** — one file,
`apps/macos/sparkle-public-keys.txt`, a line of `<channel> <key>` per channel,
which `build-app.sh` reads to stamp `SUPublicEDKey`. One file rather than four so
the whole trust picture is one `cat`, and outside `Resources/` so the keys are
not also copied into every bundle. This is a deliberate departure from how the
WorkOS client id is handled. That value is
per-environment configuration, so it lives in repository variables where a fork
can point at its own project. A Sparkle public key is not configuration: it is
the **trust anchor** deciding whose code may replace Far Cooler on someone's Mac.
Held in repository settings, anyone able to edit those settings could swap it
silently and no diff would ever show it. Committed, changing it is a reviewable
line in a commit. A fork editing one line to sign with its own key is the correct
cost — a fork *should* have to say so out loud.

Per-channel keys rather than one shared key follow the reasoning already applied
to APNs topics and WorkOS environments: a canary key cannot sign a stable update,
so a build structurally cannot accept an update from another channel even if it
were pointed at the wrong feed.

## What changes in the app

**Sparkle 2 as a SwiftPM dependency** — the first remote dependency
`apps/macos/Package.swift` has ever had.

The hard part is not linking. `build-app.sh` assembles the bundle by hand, so it
must find Sparkle's XCFramework in SwiftPM's artifact directory, copy it to
`Contents/Frameworks/`, sign it separately the way the bundled CLI and daemon
already are, and give the main binary an `@executable_path/../Frameworks` rpath
that `swift build` has no reason to set for a bundle it did not create. This is
the highest-risk piece of the feature and should be built first: everything else
is inert until an app can load Sparkle at all.

**`Updates.swift`** owns an `SPUStandardUpdaterController` and a
**Check for Updates…** menu item, and does not start the updater when no feed URL
is stamped.

**`version.sh` gains `feed-url`**, empty for local, so the feed joins the channel,
the app name, the URL scheme and the agent label in the one file that owns
per-channel facts. `build-app.sh` stamps it, alongside the public key read from
the committed per-channel file.

### The daemon restart, done where it actually works

The app carries the daemon it drives, so after an update the running daemon is
the old binary from the replaced bundle.

The obvious implementation is a Sparkle hook, and it is the wrong one:
`updaterWillRelaunchApplication` fires while the old bundle is still in place, so
it would restart the daemon into the binary being replaced.

Instead: **on launch, the app compares the running daemon's build stamp to its
own and restarts it when they differ.** Same outcome after an update, and it also
covers what a hook cannot — someone dragging a new app over the old one, or
replacing it while the app is not running. It is the rule the system already
believes, applied locally: `HostProbe` compares build stamps for remote machines
for exactly this reason. An update becomes one way the condition arises rather
than a special case.

## What changes in CI

`release.yml`, after the GitHub release exists so its asset URL resolves: sign
the dmg with the channel's private key, emit the appcast, `wrangler r2 object put`.

`canary.yml`, immediately after its dmg is built: upload
`Far Cooler-<build>.dmg` and the appcast together.

Both need Sparkle's `sign_update`, fetched from Sparkle's release tarball rather
than built from source, and both need R2 write on the Cloudflare API token.

`scripts/appcast.py` emits the XML — `sparkle:version` (the build number),
`sparkle:shortVersionString` (the marketing version), the enclosure URL, its
length, its `sparkle:edSignature`, a link to the release notes, and
`sparkle:minimumSystemVersion` of `26.0` — read from `Package.swift`'s
`platforms:` floor rather than written twice, so raising the floor cannot leave
the appcast offering an update to a Mac that cannot run it.
Sparkle also ships `generate_appcast`, which wants to own a directory of dmgs and
scan it — a model that does not fit stable and preview living on GitHub.

## How it is verified

- **`scripts/appcast.py` gets a test**: fixed inputs, expected XML, the signature
  present and the enclosure URL matching the channel.
- **The stamped keys get what naming already got**: four channels, four feed
  URLs, local empty, in the shape of `version-test.sh`.
- **The framework embedding is verified from the built bundle**, the way the
  keychain group, the URL scheme and the agent label were: build the app and read
  it back — `Contents/Frameworks/Sparkle.framework` exists, is signed, and the
  binary's rpath resolves to it.

**What cannot be tested here.** That an update actually installs needs two real
signed builds and a machine willing to be updated. The first genuine proof is a
canary offering itself an update and someone accepting it, and no amount of CI
substitutes for that.

## Setup this needs

1. An R2 bucket with `updates.farcooler.com` as a custom domain.
2. **R2 Object Read & Write** added to the existing Cloudflare API token.
3. An object lifecycle rule deleting `canary/Far Cooler-` dmgs after 14 days —
   scoped to exclude `canary/appcast.xml`, which lives under the same prefix.
4. Three key pairs from `generate_keys`: private halves as GitHub secrets, public
   halves committed.

## Deliberately not here

- **iOS.** TestFlight already does this.
- **Switching channels from inside the app.** Four installable apps make
  switching a download, which is enough until someone asks otherwise.
- **Delta updates.** Sparkle supports them; at 12 MB the bandwidth saved does not
  pay for a second artifact to sign, publish and get wrong.
- **Automatic installation on any channel.** Rejected deliberately: on a tool
  someone works inside, an app that replaces itself unasked is a worse failure
  than an update noticed a day late.
