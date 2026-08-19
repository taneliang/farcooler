# Overseeing a fleet from wherever you are

A lock screen card that follows a whole run, a home screen widget, and an Apple
Watch app that can read what an agent said and answer it.

Three surfaces, one subsystem. They are specified together because they render
the same six fields and go stale in the same way, and because the thing that
makes them possible was built for all three at once.

## What already exists

The daemon reads agents' own session logs rather than their screens
(`crates/core/src/session_log/`), and derives from them a **compact-rendering
ladder** that is already on the wire per terminal. `proto/farcooler.proto`
carries it:

| Field | What it is |
| --- | --- |
| `glyph` | The state in one character: `?` blocked, `●` working, `✓` done, `✗` failed, `·` idle |
| `headline` | ≤18 characters: `codex needs you`, `claude 4m` |
| `line` | ≤40 characters, priority-ordered: the blocked question, else task position (`3/7 · Designing test matrix`), else what it is doing now |
| `rank` | Fleet sort order. Blocked outranks done outranks working; oldest first within a tier |
| `feed` | The last three things the agent *said*, kept after the turn ends |
| `subagents`, `turn_failed`, `turn_started_at` | Running subagent names; whether the turn died; when it began |

Every string is redacted (`crates/core/src/redact.rs`) and truncated on the
host. `feed.rs` says why in its own words: the destination is "a lock screen, a
Dynamic Island, a watch face", and "a Mac, a phone and a watch must not each
decide where the ellipsis goes."

Two things this spec would otherwise have proposed are therefore already
shipped: **a blocked agent's actual question reaches the lock screen** (there is
a test named `a_blocked_agent_puts_its_question_on_the_card`), and **a failed
turn is distinguishable from a finished one**. The macOS app is a complete
reference client for the whole ladder.

This spec is mostly about teaching iOS what the Mac already knows, and then
putting it on three more surfaces.

## The decomposition

One spec for this would be a spec nobody can execute. Three, each shipping
something usable:

- **Spec 1 — glance surfaces (this document).** The iOS ladder catch-up, the
  snapshot, the Live Activity for a whole run, the home and lock screen widgets.
  Ships the widget and a card that shows progress.
- **Spec 2 — the watch that alerts and unblocks.** watchOS target, the
  WatchConnectivity proxy, fleet list, dictation, reply, Allow / Deny,
  complication, Smart Stack widget.
- **Spec 3 — reading on the wrist.** Transcript and diff review on the watch.

Specs 2 and 3 are sketched at the end at enough fidelity to be written from
later. They are not detailed here.

---

# Spec 1

## Task 0: prove a watchOS target can be generated

`apps/ios/generate-project.py` is a hand-written `pbxproj` emitter. Two of the
three specs rest on it growing a watchOS app and a watch widget extension, and
watch app embedding — bundle identifier nesting,
`WKCompanionAppBundleIdentifier`, the embed phase, a separate asset catalog — is
the fiddliest thing in this project.

So before anything depends on it: a **throwaway spike** that emits a watchOS app
target which builds and installs on hardware, and is then deleted. It answers
one question — is this an afternoon or a week — and it answers it while the
answer can still change the plan.

Nothing else in Spec 1 depends on the spike. It runs first because Specs 2 and 3
are written against its result.

## The iOS app catches up to the ladder

`apps/ios/FarCooler/Model.swift` decodes `turnFailed` and none of the other new
fields. Add `feed`, `line`, `glyph`, `headline`, `rank`, `subagents` and
`turnStartedAt`, porting from `apps/macos/Sources/FarCooler/Model.swift`, which
already decodes and renders all of them. `FleetView` sorts by `rank` rather than
by a rule of its own.

This is a prerequisite and not a nice-to-have. Every surface below renders
exactly these fields. A phone that derived its own headline would reintroduce
precisely the Mac-and-phone disagreement the ladder exists to prevent, and it
would do it on the surface where it is hardest to notice.

The two `Model.swift` files are already acknowledged copy-duplicates of each
other. This spec does not attempt to merge them; it makes the copy faithful.

## The snapshot

A widget cannot hold an SSH session, and neither can a watch. Every surface here
renders whatever was last written for it.

`FleetSnapshot`, in AgentKit, whose per-agent record **is** the wire's
vocabulary — `terminal`, `label`, `machine`, `status`, `glyph`, `headline`,
`line`, `feed`, `rank`, `turnFailed`, `activityChangedAt` — plus a `capturedAt`
and a `complete` for the whole snapshot. No invented per-agent fields, so there
is nothing for a surface to derive differently.

Stored as one atomically-replaced JSON file in a shared App Group container. A
file rather than `UserDefaults`: an atomic replace either lands or does not, and
a half-written snapshot is a widget showing something that was never true.

Two writers:

- **The app**, whenever its fleet poll yields new state.
- **A notification service extension**, which merges the single agent a push is
  about and calls `WidgetCenter.reloadAllTimelines()`.

The NSE needs `status` and `label` in the APNs payload. `services/relay/src/push.ts`
currently drops both — its `Payload` carries only `title`, `subtitle` and
`terminal`. Adding them is small, and it now ships to four relays rather than
one.

The NSE's write is best-effort. It has roughly thirty seconds, and it must never
delay delivery of the notification itself; a failed write leaves the previous
snapshot rather than a corrupt one, which the atomic replace already guarantees.

### A snapshot built only from pushes is partial, and says so

A push carries one agent. On a phone where the app has never completed a fleet
poll — a fresh install, or a first launch that never connected — the NSE would
write a snapshot containing exactly the one agent that happened to notify, and
the widget would render it as though it were the fleet.

So the snapshot carries a `complete` flag, set only by the app's fleet poll and
cleared by nothing. A partial snapshot renders the agents it has and says how it
knows them ("1 agent · from notifications"), rather than implying the other five
do not exist. This is the same rule as the staleness treatment below: show what
is known, and do not let the shape of the display assert more than that.

### The App Group has to be per channel

There are four apps now — stable, canary, preview, local — each with its own
bundle identifier, keychain group, URL scheme and relay. A shared App Group
would be the one thing they all wrote to, which is exactly the isolation
everything else already has.

The keychain solves this with `$(PRODUCT_BUNDLE_IDENTIFIER)` in the entitlements
file. **That trick does not transfer.** An app extension's bundle identifier is
the app's with a component appended, so each target would expand the same
expression to a different group and the app and its widget would not share
anything at all.

Instead, add `FARCOOLER_APP_GROUP` to `generate-project.py` beside the existing
`FARCOOLER_CHANNEL` and `FARCOOLER_URL_SCHEME`, and set it in every target that
needs it. One definition, per channel, no second list to keep in step — the same
shape the keychain fix established, by a mechanism that survives having
extensions.

The Activity extension has no entitlements file today and gains one.

### Staleness is per status, not per snapshot

The snapshot is always somewhat old. This project's rule is to answer `LOST`
rather than guess, and honoring it here means noticing that the values of
`status` do not rot at the same rate. Staleness is classified off `status`, the
semantic field, and never off `glyph`, which is a rendering of it — one
authority, the same reason the ladder is computed on the host:

- `blocked` and `done` are **latched**. An agent waiting on you is still waiting
  an hour later; nothing but a person changes that.
- `working` is **volatile**. An agent that was working an hour ago has very
  likely finished.

So past the threshold a `working` agent degrades to "last seen working" and
loses its confident styling, while `blocked` and `done` render normally, because
they have not stopped being true. Every surface carries an "as of" age.

The threshold is the relay's existing `STALE_AFTER_S` (one hour). Reusing it
means one number with one meaning across the system rather than a second one
free to drift.

## The Live Activity, for a whole run

Today a card exists only while an agent is blocked. `notification()` in
`crates/daemon/src/watch.rs` returns `None` for working — there is an explicit
test that a working agent gets no card *even with a question in hand* — and
`services/relay/src/index.ts` hard-rejects any status that is not `blocked` or
`done`.

That makes the Dynamic Island empty during the only period there is anything to
watch. The card should follow the run:

```
start ──▶ [● claude  3/7 · Designing test matrix  2m]
       ──▶ [● claude  Writing fruit.txt           4m]
block  ──▶ [? claude  Create haiku.txt?           6m]   ← alerts
done   ──▶ dismissed
```

Raised via the push-to-start token when the agent starts working, updated as
`line` advances, alerted on `blocked`, ended on `done` with the dismissal date
that already exists.

### Throttling by tier

A `line` can change several times a second. Pushing each one is a waste of the
budget and of the battery, and unreadable besides.

Throttle **in the daemon, by tier**: a tier transition — working to blocked to
done — pushes immediately, because that is the thing that matters. Within-tier
changes coalesce to **at most one push every 10 seconds per terminal**. This
reuses the tier concept `feed.rs` already has for `rank` rather than inventing a
second notion of what counts as a significant change.

Ten seconds is a starting number, not a measured one: it is roughly the fastest
a changing line is still worth reading rather than watching flicker, and it caps
a six-agent fleet at 36 pushes a minute. Risk 2 below is the work of finding out
whether the budget tolerates that. Whatever the answer, the tier transitions are
never throttled — those are the pushes the feature exists for.

Working updates go at `apns-priority: 5`; blocked and done at `10`. The relay
currently sends every activity push at `10`, which is correct for one card per
block and wrong for routine progress: priority 10 consumes the app's Live
Activity push budget, and the budget it consumes is the one the alert depends
on.

### Elapsed time is free

`turn_started_at` goes into the activity's attributes and ActivityKit renders
the timer natively. No push per tick, and it keeps counting while the phone is
off the network.

### The deep link is currently broken, and this fixes it

`apps/ios/FarCoolerActivity/AgentActivityWidget.swift:73` hardcodes the scheme:

```swift
.widgetURL(URL(string: "farcooler://terminal/\(context.attributes.terminal)"))
```

Every non-stable channel registers only its own scheme — `farcooler-canary`,
`farcooler-preview`, `farcooler-local`. Only stable claims bare `farcooler`. So
**tapping a canary build's card opens stable if it is installed, and nothing if
it is not.**

The app's own `Info.plist` documents this hazard for sign-in — "signing into
canary could deliver the code to stable" — and the widget was missed because
`ACTIVITY_COMMON` deliberately does not inherit `TARGET_COMMON`, and
`FARCOOLER_URL_SCHEME` is one of the settings it therefore does not get.

The fix: add `FARCOOLER_URL_SCHEME` to `ACTIVITY_COMMON`, read it from the
extension's `Info.plist`, and route `…://terminal/<id>` in the app — which today
nothing does, so the URL has never been more than a placeholder.

### Quick actions, and what is deliberately deferred

A button opens the app deep-linked to that agent's composer.

There is a path to answering without opening the app: a `LiveActivityIntent`
runs in the **app's** process rather than the extension's, so it could in
principle call the daemon. It would have to complete a cold SSH connect inside
the intent's short window, and we do not know how long that takes on a real
device on a real network.

So: deep link first, and in-place Allow / Deny as a follow-on once a cold
connect has been measured. Designing around a latency we are guessing at is how
a feature ships that works on the desk and not on the train.

## The widgets

All of them live in the existing `FarCoolerActivity` extension. No new target —
its `WidgetBundle` was written anticipating exactly this, and says so. Each
reads the snapshot and sorts by `rank`.

| Family | Shows |
| --- | --- |
| `systemSmall` | How many need you, plus the top agent's `glyph` and `headline` |
| `systemMedium` | Three rows: `glyph`, `headline`, `line` |
| `systemLarge` | About six rows, plus the top agent's `feed` |
| `accessoryCircular` | `glyph` and a count |
| `accessoryRectangular` | The top agent's `headline` and `line` |
| `accessoryInline` | `headline` |

`systemLarge` carrying `feed` is the point of the large size: three lines of
what the agent actually said answers "what did it do while I was away", which is
the question a widget is looked at to answer.

**Timeline policy.** Updates arrive from outside — the NSE reloads, the app
writes — so the timeline is a single entry. But it also schedules **one future
entry at the staleness threshold**, so the widget can go stale on its own. A
widget that could only learn it was stale from a message it is not receiving
would assert `working` indefinitely, which is the one thing none of these
surfaces may do.

Tapping a row deep-links to that terminal, through the same channel-correct
scheme.

## Verification

Unit tests in AgentKit, which are pure and need no device:

- `FleetSnapshot` round-trips, including a record from a newer daemon carrying a
  field this build does not know.
- Staleness classification: a `working` record past the threshold degrades; a
  `blocked` one does not.
- Merge: a push about one agent updates that agent and drops none of the others.
- A snapshot the app has never written is `complete == false`, and stays false
  however many pushes are merged into it.

A generator test in the style of the existing `scripts/version-test.sh` and
`scripts/icon-test.sh`, asserting the emitted project carries a per-channel App
Group and that the extension's scheme matches the app's.

Then on-device verification, because neither push behavior nor widget refresh
can be meaningfully checked in a simulator. Device build and signing are
recorded in `docs/releasing.md` and the project memory: team `H6A2TRW47J`,
`build-ios-frameworks.sh --device` first, signing overridden on the command line
rather than in the generator.

## Risks

1. **watchOS target generation is an afternoon, not a week — with one build-
   machine gotcha that has nothing to do with the generator.** Task 0 built a
   throwaway watch target on a scratch copy of `generate-project.py`: its own
   `PBXNativeTarget` (`productType = com.apple.product-type.application`,
   `SDKROOT = watchos`), a `PBXCopyFilesBuildPhase` embedding it in the app at
   `dstSubfolderSpec = 16` / `dstPath = "$(CONTENTS_FOLDER_PATH)/Watch"` — a
   different slot from the widget extension's `13`/`PlugIns`, because a watch
   app is not a foundation extension — and the `PBXTargetDependency` /
   `PBXContainerItemProxy` pair that makes the app build the watch app first.
   All of it mirrors `activityTarget` almost mechanically; nothing about the
   shape of a hand-written `pbxproj` fights a fourth native target the way the
   plan worried it might. `WKCompanionAppBundleIdentifier` — the setting this
   plan was most worried about — round-tripped cleanly as
   `INFOPLIST_KEY_WKCompanionAppBundleIdentifier` under
   `GENERATE_INFOPLIST_FILE = YES`, no checked-in watch Info.plist needed; the
   built product's plist carried it exactly, verified with `plutil` against the
   actual output.

   The one real surprise was environmental, not structural.
   `xcodebuild -scheme FarCooler … build` — a pure device build,
   `CODE_SIGNING_ALLOWED = NO`, no simulator involved — refused outright with
   *"This scheme builds an embedded Apple Watch app. watchOS 26.5 must be
   installed in order to run the scheme."* The watchOS **SDK** ships with
   Xcode and was already present; what was missing is the watchOS
   **Simulator platform**, a separate ~4 GB component
   (`xcodebuild -downloadPlatform watchOS`) that Xcode's scheme validation
   demands the moment a scheme embeds a watch app, even though this build
   never touches a simulator. Building the same target with `-target FarCooler`
   instead of `-scheme FarCooler` sidesteps that scheme-level check entirely —
   useful for confirming the generator's output is sound, but not a
   substitute, since `-scheme` is what release builds actually use. After the
   platform component was installed, the brief's exact command built clean,
   zero `error:` lines, watch app embedded and signed ad hoc alongside the
   phone app. **Any machine building this — a laptop or CI — needs the
   watchOS Simulator platform installed before a scheme build will even
   start, regardless of whether anything ever runs in that simulator.**

   What stays unknown until a device is available: everything install-time.
   Whether a real paired watch accepts the companion relationship this
   `pbxproj` declares, whether the two bundle IDs' provisioning matches, and
   whether WatchConnectivity actually pairs are all checked by the OS at
   `devicectl device install app`, not at `build`. The plist keys are
   confirmed correct in the built artifact; whether the *watch* honors them is
   the one question this spike could not reach, because the paired iPhone was
   unavailable this session.
2. **The APNs Live Activity budget under whole-run cards.** The throttle
   interval is a guess until it is measured on a device with several agents
   running. If the budget turns out tighter than expected, the fallback is to
   coalesce harder rather than to abandon the whole-run card.
3. **`MarkdownView` at 45mm** — unknown until Spec 3, and it may need a watch
   variant rather than a reuse.
4. **Cold-phone WatchConnectivity launch latency** — the same unknown that
   defers in-place Allow / Deny.

---

# Specs 2 and 3, sketched

Recorded here so they can be written from later. Not detailed, and not committed
to.

**Transport.** One `FleetClient` protocol with a WatchConnectivity
implementation behind it. The watch sends a request, iOS is launched in the
background if it is not running, the phone runs the existing Rust core and
replies. The watch holds no SSH identity and needs no authorization on any host
— which is also why it cannot work with the phone out of range, and must say so
rather than appear broken.

The snapshot reaches the watch by `updateApplicationContext`: latest-state-wins,
delivered opportunistically in the background, and stale copies do not queue up.
That is a snapshot's semantics exactly.

**Three reachability states, never blurred.** Phone reachable, so data is live.
Phone unreachable but a snapshot is present, so it is shown with its age and the
actions are disabled. Neither, so an empty state. The watch must never render a
stale value as a live one, which is the same rule that produces `LOST`
everywhere else.

**Screens.** Fleet list (`glyph`, `headline`, `line`, by `rank`) → agent detail
(`feed`, `line`, elapsed, subagents) → transcript, diff, compose, permission.
Dictation comes free from watchOS `TextField`. Diffs render as a file list and
then one hunk per screen, which is legible at 45mm in a way a side-by-side is
not.

**AgentKit gains watchOS** in `Package.swift`. It has zero UIKit and zero
AppKit, so this should be close to free; `AgentActivityAttributes` is already
`#if os(iOS)` guarded, which stays correct because activities are started on the
phone.

**Complication and Smart Stack** are a second watch target reading the
watch-side snapshot: `glyph` for circular, `headline` and `line` for
rectangular, and `rank` choosing which of six agents to show — which is the job
`rank` was built for.

Spec 2 is the shell, the proxy, dictation, reply, Allow / Deny, the complication
and the Smart Stack widget. Spec 3 is transcript and diff review.

---

# What spec 1 shipped, and what a device still has to prove

Spec 1 is implemented and merged into `worktree-watch-and-widgets`. Every
automated check passes: 92 AgentKit tests, 354 daemon tests, 61 relay tests, a
clean `tsc`, and an `xcodebuild` with no `error:` lines.

**Nothing has run on hardware.** No iOS device was available while this was
built, so every claim below is a claim about source that compiles, not about
software anyone has seen work. That distinction matters most for the two
mechanisms whose failure is silent.

## Before anything else: four App Groups must exist

`group.com.farcooler.ios`, `.canary`, `.preview`, `.local`. The entitlement is
only validated at install time, so nothing before then can catch a missing one.
The failure is quiet and misleading: `SnapshotStore.container` returns nil, every
widget reads "Open Far Cooler to see your agents" forever, and the app beside it
works perfectly — which looks exactly like a phone that has simply not polled
yet.

## Ranked by how likely it is to be wrong

1. **The App Group is not granted.** Above. Check first.
2. **The Live Activity update budget.** `NSSupportsLiveActivitiesFrequentUpdates`
   is declared and the daemon pushes at most one card update per terminal per
   10s, but the real budget is Apple's and unmeasured. Symptom: the card freezes
   partway through a run with nothing logged. This is Risk 2, still open — the
   throttle interval remains a guess.
3. **`mutable-content` and the notification service extension.** Now covered by
   relay tests, but never observed invoking the extension on a device. Symptom:
   widgets only ever update when the app is opened.
4. **The lock screen card's tap target**, and that a canary build's card opens
   canary rather than stable. Verifiable only with two channels installed.
5. **The six widget layouts.** None has been looked at. `systemLarge` (six rows
   plus a divider plus three feed lines) and `accessoryCircular` clipping are the
   plausible silent failures.
6. **The accessory staleness degradations**, which rest on three rendering claims
   that were reasoned rather than observed: that `accessoryInline` ignores
   opacity (which is why it degrades in words), that opacity survives
   `accessoryCircular`'s accented mode, and that "last seen …" fits inline.
7. **The whole-run card end to end** — that a card appears when an agent starts,
   its line advances, it alerts on blocking, and it clears on done.

## Two decisions deliberately left open

**`apps/ios/FarCooler.xcodeproj/project.pbxproj` is not committed.** CI
regenerates it before every build (`ci.yml`, `canary.yml`, `release.yml`) and
states it is "never committed as the source of truth", and the branch was
verified to build in CI as committed. The cost is that a person who clones and
opens Xcode without running `generate-project.py` gets a project missing the new
files. That is an onboarding matter, and reversing it is a reasonable call.

**A dismissed card is remembered for one hour per terminal.** Dismissing a card
just before a new run therefore suppresses that run's card. The alternative —
forgetting immediately — reinstates a card the person just swiped away, roughly
every ten seconds for the length of the run.

## Known, accepted, and worth re-reading before spec 2

On a phone whose app is not running, the relay starts a card it has no update
token for. A later `blocked` push cannot move that card, so it starts one fresh
card instead and lets the stale one expire, which briefly shows two. The
duplicate is bounded three ways: one replacement per row, the one-hour stale
date, and `LiveActivities.reapDuplicates` collapsing them at next launch. The
alternative was a lock screen reading "Working" while the banner said the agent
needs you — a surface stating something false, which this design consistently
ranks as the worse outcome.
