# JSON contracts between the runner, the relay and the apps

Golden JSON for every JSON payload that crosses between two languages on the
way to a phone. Protobuf has `proto-lint`; these payloads are written by hand
on each side, so each side's tests used to assert only its own spelling, and a
key renamed on one side left both suites green (ov-102 review, finding 4).

Each fixture is written by exactly one producer, whose test asserts it writes
exactly the fixture. Each consumer's test reads every fixture in a directory
and asserts the fields it uses. Renaming a key on either side fails a test.

| Directory | What | Producer, and its test | Consumers, and their tests |
| --- | --- | --- | --- |
| `notify/` | `POST /v1/notify` body, one per kind: agent `working`, `blocked` (with its ask), `blocked` with `alert: false`, failed `done`, `ask`, `count`, the legacy `decision` carrying ov-94's task notice fields, and `task` for a decision and a review | Daemon, `crates/daemon/src/push.rs` `wire_body`. Test: `push::contracts` | Relay `notify` in `services/relay/src/index.ts`. Test: "the shared contract fixtures" in `services/relay/test/relay.test.ts` posts each fixture |
| `push/apns/` | The APNs alert body the relay sends for a notify fixture of the same name | Relay `sendApns`, `services/relay/src/push.ts`. Test: the relay suite, from the `notify/` fixture of the same name | AgentKit `TaskNotice(userInfo:)` and `PushTap`. Test: `PushContractTests` in `apps/shared/AgentKit/Tests/AgentKitTests/ContractTests.swift` |
| `push/fcm/` | The FCM message the relay sends for a notify fixture of the same name | Relay `sendFcm`. Test: the relay suite, as above | Android `TaskNotice.of` and `NotificationCopy.channelForPush`. Test: `apps/android/app/src/test/java/com/farcooler/ContractTest.kt` |
| `live-activity/` | The Live Activity push (`aps`, with `content-state`) the relay sends for a notify fixture of the same name | Relay `sendLiveActivity` and `withFleet`. Test: the relay suite, as above | AgentKit `AgentCardState` and `AgentCardRow`. Test: `LiveActivityContractTests` in `ContractTests.swift` |
| `registration/` | `POST /v1/devices` body: `ios.json`, `macos.json`, `android.json` | `ios`, `macos`: AgentKit `Account.registration`. Test: `RegistrationContractTests` in `ContractTests.swift`. `android`: `Account.registration` in `apps/android/.../account/Account.kt`. Test: `ContractTest.kt` | Relay `registerDevice`. Test: the relay suite files each one and checks every column it stores |

A notify fixture with no `push/apns/`, `push/fcm/` or `live-activity/` fixture
of the same name must produce no such push: the relay suite asserts that too.

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
stamps are exact. Build stamps differ per build: each producer checks `version`
is its own build's and compares it as the fixture's value.

## Changing a fixture

Change the producer, then regenerate its fixtures and read the diff:

- Daemon: `FARCOOLER_WRITE_CONTRACTS=1 cargo test -p farcooler-daemon --lib push::contracts`
- AgentKit: `FARCOOLER_WRITE_CONTRACTS=1 swift test --filter RegistrationContractTests`
- Android: `FARCOOLER_WRITE_CONTRACTS=1 ./gradlew testInstrumentedUnitTest --tests com.farcooler.ContractTest`
- Relay: the suite runs inside workerd and can't write files. A missing
  fixture fails with the JSON the relay wrote; copy it in.

Then run every consumer's suite. A consumer that fails is an app or relay
already shipped that would read the new payload wrong.

## Not here

- The Go tunnel helper's commands (`crates/tailcat/go/helper.go`,
  `crates/tailcat/src/helper.rs`) are a line protocol, not JSON.
- The iOS notification service extension reads `status`, `label` and `failed`
  itself, outside AgentKit, so only the `terminal` it shares with `PushTap` is
  checked here.
