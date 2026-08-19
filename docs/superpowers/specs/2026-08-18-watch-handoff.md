# The watch, and what hardware still has to prove

Spec 2 is implemented on `worktree-watch-and-widgets`. Every automated check
passes: 398 daemon tests, 113 AgentKit tests, 91 relay tests, a clean `tsc`, 49
generator assertions, and an `xcodebuild` of the scheme — which now embeds the
watch app — with no `error:` lines.

**No iPhone and no Apple Watch have been available at any point.** Nothing here
has run on hardware. The watch's *layouts* have better evidence than the phone's:
Tasks 4, 5 and 6 rendered their screens in the watchOS 26.5 simulator and
screenshotted them at 46mm and 40mm. Everything else below is unobserved.

## Do these first, in this order

**1. Provision two new bundle identifiers.** `{BUNDLE_ID}.watchkitapp` and
`{BUNDLE_ID}.watchkitapp.widgets` — a fifth and sixth identifier, for each of
four channels. Nobody has provisioned them. Ad-hoc signing accepts anything, so
this fails at install and not at build.

**2. Grant the App Group on the watch.** A separate entitlement from the phone's,
per device. Without it the watch app and its complication are permanently empty
against a working phone, and it is indistinguishable from "no context has
arrived yet" — the same silent failure the phone's App Group has.

**3. Measure `sendMessage`'s reply deadline against the phone's budgets.** The
phone allows itself 8 seconds to connect plus 10 to replay a transcript, so a
`pendingPermission` can take **18 seconds** and a prompt or an answer 16. Nobody
knows WCSession's reply deadline. If it is shorter, every watch action reports a
failure for work that actually landed. This is the single measurement that
decides whether the feature works.

**4. Find out whether a phone woken by `sendMessage` has a connection at all.**
`Connection` is a `@StateObject` owned by `FleetView`, so a background launch
that never builds a scene has no SSH session, and `WatchLinkHost` cannot make one
— it would need runner selection, identity handling, and a human to trust a host
key. It says so honestly ("Open FC Local on your iPhone, then try again"), which
is not what the feature promises. **If this is the common case, connection
ownership has to move from the screen to the app** — a refactor touching the
whole app, which is why it was not attempted on evidence nobody has.

## Then these

5. **The `.appex` running as a real complication.** Every layout was rendered
   inside the watch app, never by the extension process. Untested: the appex
   launching, `SnapshotStore` resolving a container from inside it,
   `reloadAllTimelines()` reaching it, face-gallery and Smart Stack listing, and
   how watchOS draws `.containerBackground` on a face.
6. **Whether `updateApplicationContext` launches a not-running watch app.** The
   complication depends on `WatchLinkClient.receive` executing. If the system
   will not launch the app to deliver, the complication only updates while
   somebody has the app open.
7. **The companion relationship**, and the `dstSubfolderSpec 16` / `Watch` embed
   — verified only through the project's object graph and a `plutil` read of the
   built bundle.
8. **The permission screen's header** after a failed answer. It is the one
   cleanup change nobody rendered, on the screen where a displaced reject option
   is an integrity failure rather than a style one. The simulator tooling exists;
   this should be the first thing pointed at it.
9. **A staleness transition actually firing.** Every stale state on every surface
   came from an aged fixture. Nobody has watched a row flip to "last seen …".
   If a wake never arrives the failure is silent: the face keeps saying what it
   said an hour ago.

## A limit that is documented rather than fixed

**The watch is only ever as fresh as the last time the phone app ran a fleet
poll.** The notification service extension writes the snapshot the phone's own
widgets read, but an app extension cannot reach `WCSession`, so a push-refreshed
snapshot never reaches the watch. The phone's widgets stay current from pushes;
the watch does not. On hardware this will look like a bug, and it is a known
consequence of where the two writers live.

## Deliberately not built

**No deep link from the complication.** A tap opens the fleet, not the agent. The
watch app registers no URL scheme, and a `widgetURL` for an unregistered scheme
is a tap that looks right and does nothing — and it is the one thing the
simulator cannot verify at all. Whoever adds it also owns `AgentDetailView`'s
"not in the fleet" sentence, which becomes reachable that moment and currently
asserts a fleet was sent.

**No subagent names on the agent screen**, which the spec originally promised.
`FleetSnapshot.Agent` carries no subagent list, and the host already folds the
*count* into `line`. Inventing a list on the watch would break the rule that a
client never re-derives what the daemon decided; adding a field changes the
projection all five surfaces share and should be decided once, on the host.
