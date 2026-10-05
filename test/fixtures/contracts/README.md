# JSON contracts between the runner, the relay and the apps

Golden JSON for the payloads that cross between two languages on the way to a
phone. Protobuf has `proto-lint`; these payloads are written by hand on each
side, so each side's tests used to assert only its own spelling, and a key
renamed on one side left both suites green (ov-102 review, finding 4).

Each fixture is written by exactly one producer, whose test asserts it writes
exactly the fixture. Each consumer's test reads the fixtures through the code
the shipped app or relay runs, and asserts the fields it uses against values
spelled out in the test, never read back off the fixture. Renaming a key on
either side fails a test. The list below is the whole of what's covered.

## Covered

| Directory | What | Producer, and its test | Consumers, and their tests |
| --- | --- | --- | --- |
| `notify/` | `POST /v1/notify` body, one per kind: agent `working`, `blocked` (with its ask), `blocked` with `alert: false`, failed `done`, `ask`, `count`, `decision-old-runner` (a decision with no `event`, as a runner older than ov-94 sends it, which is the one fixture Android reads as a card carrying a task), `task` for a decision and a review, and `count-plan`, a count carrying the plan's glance (ov-310) | Daemon `wire_body`, `crates/daemon/src/push.rs`. Test: `push::contracts` | Relay `notify`, `services/relay/src/index.ts`. Test: "the shared contract fixtures" in `services/relay/test/relay.test.ts` posts each one |
| `runner/` | `POST /v1/heartbeat` (a beat, and the withdrawal on unpairing) and `POST /v1/notify/retire` bodies | Daemon `beat_body`, `withdraw_body`, `Retirement`. Test: `push::contracts` | Relay `heartbeat` and `retireActivities`. Test: "the runner's and the card's shared contract fixtures" |
| `pulse/` | The `/v1/pulse` answer: `response.json`, a minute and a half after `runner/heartbeat.json`, `turns.json`, how finished agents' turns ended (ov-239), and `plan.json`, the board the glance draws after `notify/count-plan.json` (ov-310) | Relay `pulse`. Test: the relay suite, from the heartbeat fixture; `turns.json` from `test/pulse-turns.test.ts`; `plan.json` from `test/plan-glance.test.ts` | AgentKit `RunnerPulse.decode`, which the widget and the watch use. Test: `PulseContractTests` in `apps/shared/AgentKit/Tests/AgentKitTests/ContractTests.swift` |
| `push/apns/` | The APNs alert body the relay sends for the notify fixture of the same name | Relay `sendApns`, `services/relay/src/push.ts`. Test: the relay suite | AgentKit `TaskNotice(userInfo:)`, `PushTap`, and `AgentPush`, which the notification service extension (`apps/ios/FarCoolerNotify`) reads an agent's push with. Test: `PushContractTests` |
| `push/fcm/` | The FCM message the relay sends for the notify fixture of the same name | Relay `sendFcm`. Test: the relay suite | Android `PushMessage.of`, which `FarCoolerMessagingService.onMessageReceived` reads every push with, and the channel Firebase is told against the one the app picks. Test: `apps/android/app/src/test/java/com/farcooler/ContractTest.kt` |
| `live-activity/` | The Live Activity `start` push the relay sends for the notify fixture of the same name | Relay `sendLiveActivity` and `withFleet`. Test: the relay suite | AgentKit `AgentCardState` (the activity's `ContentState`) and `AgentCardRow`. Test: `LiveActivityContractTests` |
| `live-activity/running/` | A running card's `update` (for `notify/agent-working.json`), its `end` (for `runner/retire.json`, with `dismissal-date`), and `plan`, the update `notify/count-plan.json` moves it with (ov-310) | Relay, as above. Test: the relay suite, after filing `activity/running.json`; `plan` from `test/plan-glance.test.ts` | As above |
| `registration/` | `POST /v1/devices` body: `ios.json`, `macos.json`, `android.json` | `ios`, `macos`: AgentKit `Account.registration`. Test: `RegistrationContractTests`. `android`: `Account.registration` in `apps/android/.../account/Account.kt`. Test: `ContractTest.kt` | Relay `registerDevice`. Test: the relay suite posts each as sent and checks every column it stores |
| `activity/` | `POST /v1/devices/activity` body: a card's token, a card a person swiped away, and one that ended | AgentKit `Account.activityRegistration`, which `registerActivityToken` sends. Test: `ActivityRegistrationContractTests` | Relay `registerActivity`. Test: the relay suite |

A notify fixture with no `push/apns/`, `push/fcm/` or `live-activity/` fixture
of the same name must produce no such push: the relay suite asserts that too.

## Not covered, and why

- **`/v1/auth/token`, `/refresh` and `/logout`**: the relay passes WorkOS's
  answer through; the shape is WorkOS's, not ours.
- **`/v1/devices/lookup` and `/v1/devices/verify`**: no client calls them
  (ov-102 review, finding 6).
- **`/v1/account`, `/v1/daemons`, `/v1/devices/revoke` and
  `/v1/daemons/revoke`**: request and answer bodies for the account screens.
  They reach a person at the moment they act, not on a lock screen, so a
  rename shows up the first time someone opens the screen. Not done in ov-121;
  worth their own card.
- **The relay's answers to `/v1/notify`, `/v1/heartbeat`, `/v1/notify/retire`,
  `/v1/devices` and `/v1/devices/activity`**: the daemon and the apps read
  only the status code.
- **The macOS app's `Notifications.terminalID(userInfo:thread:)`**: it's in
  the macOS app target, outside AgentKit, and runs only on a full macOS build.
  It reads `terminal` and `kind`, the keys `PushTap` is tested with.
- **The heartbeat's `beatEvery`**: the relay defaults an absent value to 300,
  which is what the runner sends, so a rename there changes nothing the relay
  stores until the runner's interval changes.
- **The Go tunnel helper's commands** (`crates/tailcat/go/helper.go`,
  `crates/tailcat/src/helper.rs`): a line protocol, not JSON.
- **`test/fixtures/needs-you.json`**: the client core's needs-you JSON, which
  has its own fixture and tests in all three languages.

## Exact or superset

**Producers are exact.** A producer's test fails on any difference, including
a key it added. Adding a field is allowed on the wire, since every consumer
ignores keys it doesn't know, but the field joins the fixture in the same
commit, so every consumer's suite runs against it.

**Consumers read a superset.** A consumer's test asserts the fields it uses and
ignores the rest, the way the shipped decoder does: an app in the App Store
must keep working when the relay adds a key.

## Fixed values

The fixtures are sampled at 2026-10-03 09:30:00 UTC (Unix 1791019800). The
daemon's test builds its trace and ask at that instant, and the relay's suite
pins `Date` to it, so trace anchors, an ask's end and every timestamp the relay
stamps are exact. Build stamps and the computer's name differ per build: each
producer checks they're its own, then compares them as the fixture's values.

## Changing a fixture

Change the producer, then regenerate its fixtures and read the diff:

- Daemon: `FARCOOLER_WRITE_CONTRACTS=1 cargo test -p farcooler-daemon --lib push::contracts`
- AgentKit: `FARCOOLER_WRITE_CONTRACTS=1 swift test --filter "RegistrationContractTests|ActivityRegistrationContractTests"`
- Android: `FARCOOLER_WRITE_CONTRACTS=1 ./gradlew testInstrumentedUnitTest --tests com.farcooler.ContractTest`
- Relay: the suite runs inside workerd and can't write files. A missing
  fixture fails with the JSON the relay wrote; copy it in.

Every producer refuses `FARCOOLER_WRITE_CONTRACTS` when `CI=true`, so CI can't
pass by rewriting. Then run every consumer's suite. A consumer that fails is
an app or relay already shipped that would read the new payload wrong.
