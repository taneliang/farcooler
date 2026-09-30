# Runner heartbeat: the widget can tell a live runner from a stale one (ov-53)

## The problem, as the tree has it

The phone's widgets draw `fleet.json` in the App Group. Two things write it:
the app, on every SSH poll (`FleetSnapshotWriter`), and the notification
service extension, on every ALERT push (`NotificationService.swift`). A widget
does no I/O of its own today (`FleetProvider.getTimeline` reads the file and
asks for `.never`).

So while the app is suspended nothing marks a runner lost: `lostRunners` is
stamped only by `FleetPublication.keeping(runners:answering:)`, which runs in
the app. A runner that stopped hours ago leaves its last rows on the widget
under "from notifications" (`FleetSnapshot.Hedge.fromNotifications`), the
same words a healthy fleet gets. Nothing that reaches the phone after a runner
dies can say it died, because a dead runner sends nothing.

## The design

**1. The daemon beats.** `farcoolerd`'s watcher sends `POST /v1/heartbeat`
every **5 minutes** while it runs and is paired, first beat at start. It
carries the runner's install id, its build, its own name (the Mac's computer
name, else the short hostname) and `beatEvery: 300` (seconds), and nothing
else — no count, no agent, no card. Failure is logged and
swallowed, like `notify`. A relay too old for the route answers 404, logged at
debug so an old relay isn't a warning every five minutes.

**2. The relay records it.** `/v1/heartbeat` authenticates the daemon token as
`/v1/notify` does, sets `daemons.last_seen_at` (the column 0001 already has,
and `/v1/notify` already stamps), `daemons.beat_every` and `daemons.name`
(new, migration 0015, additive). `beat_every` is what tells a runner that promised to beat
from one too old to: NULL is today's daemon, and it never reaches the widget.
Clamped to 60..3600 s. No push, no card.

**3. The widget asks.** A new device-scoped read credential, the **pulse
token**. The PHONE makes it (32 random bytes), once per account, keeps it in
the Keychain under the App Group's access group so the widget can read it,
and sends it on every `/v1/devices` registration. The relay stores its
SHA-256 (`devices.pulse_hash`, migration 0015), idempotently: the same token
again is a no-op, a new one replaces the old, a token already on another row
moves to this one, and a device that changes accounts loses it. Sign-out
deletes it on the phone; the next sign-in makes a new one, which replaces
the hash at the next registration. (A relay-minted token per registration
was the first cut, and lost a race: two registrations at launch, answered
out of order, left the widget holding a token whose hash was gone.) The widget's `getTimeline` posts it to
`POST /v1/pulse`, which answers the account's beating runners:
`[{label, name, heardAgo (ms), beatEvery (s)}]`, one per runner (install key, else
token), newest beat wins, silent over 24 h dropped (`ROW_RETENTION_MS`, the
age at which the relay forgets a runner's rows too). The relay reports an
AGE, not a timestamp, so the phone's clock never enters it.

The token can read runner names and ages on its own account, and nothing
else. It dies with the device row (revoke). Mac and Android don't send one.

**4. The stale threshold.** A runner is quiet when `heardAgo > 2 × beatEvery
+ 5 min` — **15 minutes** at the shipped interval: two missed beats plus a
slow request. `RunnerPulse.quietAfter` in AgentKit, the one place.

**5. The words.** ov-50's: a quiet runner joins `lostRunners` in the widget's
hedge, so the footer reads "lost touch with Studio" and a lost runner outranks
"from notifications", as it already does. `FleetSnapshot.hedge(quiet:)`.
The name is the runner's own (`name` from its beat), before the pairing
label, which is "This Mac" for every Mac's own runner and names nothing on a
phone. **A fresh app snapshot wins**: when the app polled every runner more
recently than one could have gone quiet (`complete`, nobody lost, younger
than the threshold), the relay's reading is ignored, since the phone heard
its runners over its own links (a LAN-only runner, a relay outage).

**5a. Unpaired on purpose.** Stop Notifying and `push forget` send one last
`/v1/heartbeat` with `withdrawn: true`, which sets `beat_every` back to NULL,
so the runner leaves the pulse and the widget says nothing about it rather
than "lost touch". If that call fails, the local unpair still happens and
the relay drops the runner from the pulse a day after its last beat; until
then the widget may say it lost touch with it.

**6. Old daemons.** A runner with no `beat_every` isn't in the pulse answer,
so the widget says exactly what it says today. A failed pulse fetch (offline,
old relay, no token) is the same: today's hedge. The widget claims "lost
touch" only on the relay's word, never on its own clock.

**7. Budget.** While the pulse answer names a runner, or after a fetch that
failed (offline, a timeout, a server error), the timeline asks for
`.after(30 min)` instead of `.never`; with none, no credential, or a refused
token it keeps `.never`. All of it is `RunnerPulse.plan`. 48 asks a
day sits inside WidgetKit's ~40-70 daily reloads, and the system throttles
past that anyway. Relay cost: 288 D1 writes per runner per day for beats, and
at most one read per widget reload.

## Rejected

- **Cron on the relay pushing "runner went quiet".** Needs a background push
  (`content-available`), which iOS throttles, never delivers to a
  force-quit app, and which doesn't run the notification service extension.
  The app has no `remote-notification` background mode either.
- **Folding liveness into the pushes already sent.** A dead runner sends
  none; with one runner, nothing would ever arrive to say it died.
- **Forwarding every beat to the phone.** A push per runner per 5 minutes per
  device, all throttled, to learn one bit.
- **The widget using the WorkOS session.** The session lives in the app's
  keychain group, which the extension can't read; sharing it would widen the
  keychain the channels design keeps per bundle, and refresh-token rotation
  between two processes would sign one of them out.
- **The push token as the widget's bearer.** It's an address, not a secret,
  and it rotates under the app.
- **A goodbye beat on shutdown.** A sleeping laptop can't send one, so the
  timeout is needed anyway, and it covers the clean case too.
- **A 1-minute beat.** Five times the D1 writes for nothing: the widget can't
  look more often than about every 20-30 minutes.

## Not in this change

- ~~The watch complication and the Live Activity keep today's gate.~~ Done
  in ov-71; see below.
- Marking a quiet runner's WORKING rows "last seen": the widget's rows name a
  runner by the phone's label and the relay by the runner's own name, and the
  two don't reliably match. The footer names the runner; the rows are
  unchanged. Matching by install id (the phone learning each runner's over
  SSH) would close it.
- A beat in flight while `push forget` runs can land after the withdrawal and
  put the runner back in the pulse until it ages out. The daemon reads the
  pairing file per beat, so at most one such beat.
- A daemon downgraded below the heartbeat keeps its `beat_every` and reads as
  quiet while running, until it ages out.
- Rate limits on `/v1/heartbeat` and `/v1/pulse` (both need a credential; a
  guessed one costs a hash and an indexed read).
- Android has no widget.

## ov-71: the watch and the Live Activity

**The watch borrows the phone's token.** A watch has no sign-in (the WorkOS
session is in the phone app's keychain group) and no push registration, so
it can't file a token of its own at `/v1/devices`. `WatchLinkHost` puts the
pulse credential in every application context beside the snapshot
(`PulseCredential.watchContextKey`); a context without it, after sign-out,
takes it away. `WatchLinkClient` keeps it in a file in the watch's own App
Group container (`ContainerPulseVault`), because the keychain app-group
share the phone relies on is unverified on watchOS and the container
demonstrably works (the snapshot lives there). WatchConnectivity is
encrypted between the paired devices, and the token reads names and ages.

**The watch reads it the widget's way** (`RunnerPulse.look`, which is
`fetch` then `plan`): the fleet list asks while it's in front and names a
quiet runner in its footer (`hedge(quiet:)`, "Lost touch with Studio, so
its agents may have changed."); the rectangular complication asks in
`getTimeline` and says "Lost touch with Studio" in place of the top agent's
line when no count outranks it, looking again hourly while anything beats
(24 a day; the circular and inline families have no room and don't ask).

**The Live Activity is moved by a relay cron sweep.** A card redraws only
when pushed, and a dead runner sends nothing, so a five-minute cron trigger
(`sweepQuiet`) finds each addressable, undismissed card whose quiet runners
differ from what it last said (`install_cards.quiet`, migration 0016) and
pushes an update: the quiet runner's working rows leave `working` and the
lines (blocked and done hold), and the state carries `quiet: [name]`, which
the tail draws as "+1 more · lost touch with Studio". A card left with no
rows and a quiet runner is drawn like a stale one (`unvouched(stale:)`), so
its headline stops saying "Working". The relay states the quiet rule once
more (`quietAfterMs`), pinned to `RunnerPulse.quietAfter` by tests on both
sides. Every other push composes the same card, and remembers what it said.

Rejected:

- **Ending the card when its runner goes quiet.** It could take a blocked
  agent's question with it, and a card starts again only on a blocked
  notice. An update says the true thing and leaves the question up.
- **A `stale-date` pushed with every beat** (so the card goes stale 15
  minutes after the last one). A push per card per five minutes to learn
  one bit, and a stale card can't say which runner, or why.
- **Checking on other runners' beats instead of a cron.** With one runner,
  nothing would ever arrive to notice it stopped; the widget's spec rejects
  folding liveness into existing traffic for the same reason.
- **A cron push to the phone app** is still rejected (background pushes;
  see above). A Live Activity push goes straight to the card and runs no
  app code, which is why the sweep is sound here and not there.
- **The watch registering its own device.** No session on the watch.
- **The watch's keychain.** Unverified for an App Group access group on
  watchOS; a failure there would be silent, the footer never changing.
- **The phone pushing quiet names to the watch.** The phone app is
  suspended exactly when a runner goes quiet unnoticed.

Not in this change: a card whose app never filed an update token can't be
moved (as before); the rows of a quiet runner on the watch and the widget
are unchanged (the labels still don't reliably match); the complication's
look shares the watch's reload budget with the app's reloads.
