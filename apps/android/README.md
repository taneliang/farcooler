# Far Cooler for Android

The phone client, for machines you already reach over SSH. Same idea as
[the iOS app](../ios), same Rust cores, same daemon — nothing here re-implements
the protocol, the SSH transport, the terminal emulator, or the agent-transcript
reducer.

```
crates/android          the JNI shim: one `.so`, no behaviour of its own
apps/android/app
├── core/               ClientCore, VtCore — the Kotlin side of the two C ABIs
├── model/              Transcript, AgentEvent, Markdown, diff — ported logic
├── data/               the device's SSH key, known machines, settings
├── net/                one Connection per machine, terminal + agent streams
├── account/            WorkOS sign-in, FCM registration
├── notify/             local notifications, and the ones that arrive asleep
└── ui/                 Compose
```

## Requirements

- **Android 17** (API 37) or newer. Stated by the product, not inferred.
- A JDK 17+, the Android SDK with platform 37, and an NDK for the Rust cores.
- Rust with the `aarch64-linux-android` target.

## Build

The native cores first — Gradle does not build them, because cross-compiling
Rust from Gradle means a build that fails differently depending on which of two
toolchains is missing:

```sh
rustup target add aarch64-linux-android
./scripts/build-android-libs.sh              # arm64 — a phone
./scripts/build-android-libs.sh --emulator   # plus x86_64
```

That writes `libfarcooler_jni.so` into `app/src/main/jniLibs/<abi>/`. Then:

```sh
cd apps/android
./gradlew assembleDebug
./gradlew installDebug     # onto a connected device
```

The APK carries no core for an ABI the script was not run for, and `abiFilters`
in `app/build.gradle.kts` is what turns that into a build-time fact rather than
an `UnsatisfiedLinkError` on the first screen.

## Tests

```sh
./gradlew testInstrumentedUnitTest           # the ported logic, on the JVM
./gradlew connectedInstrumentedAndroidTest   # the JNI bridge, on a device
```

Both say `Instrumented` because both build against that build type, which has
its own application id. AGP uninstalls the app under test when a connected run
finishes, and against `debug` that is the build someone is actually using — the
uninstall takes its data with it, which here means the machine list *and* the
device's SSH identity, since the Keystore key goes when the app does. The phone
is then holding a key the machine has never authorized, which looks exactly like
a rejected connection. A separate id means the tests install, run and uninstall
something nobody was relying on. AGP generates test variants for one build type
only, which is why the JVM task is renamed too.

The unit tests are the Swift suites in
[`apps/shared/AgentKit/Tests`](../shared/AgentKit/Tests), translated case for
case — same JSON, same fixtures, same expectations, including the captured
`live_events.jsonl` from a real daemon. That is deliberate: the transcript
reducer is the one piece of this app that must agree with the Mac and iOS bit
for bit, because two clients that fold the same events differently disagree
about one conversation. Two suites asserting the same outcomes catch that; two
suites testing "the same sort of thing" would not.

The instrumented tests cannot run on a desktop JVM — there is no `.so` on its
library path, and mocking one would test the mock.

## What is different from iOS, and why

Everything below the UI is the same. These are the places the platform made a
different answer correct.

**Every machine is connected at once.** iOS has a machine picker, which makes a
remote agent something you have to go and look for — the product's whole claim
is that an agent blocked on a machine in another room is exactly as urgent as
one on this desk. This does what the Mac does
([design](../../docs/superpowers/specs/2026-08-03-every-machine-in-one-fleet-design.md)):
one `Connection` per machine, one merged fleet, the machine named on a row only
once there is more than one. A machine that stops answering keeps its rows and
says why, rather than dropping them. It is a setting rather than a rule only
because a phone pays for each extra SSH session in radio wake-ups.

**A navigation drawer, not a sheet.** The fleet is where the Mac's sidebar is:
one edge swipe from anywhere, holding worktrees, machines, "start something
new", and settings. The bottom tab strip stays, and is not a second copy of it —
that switches panes in one tap with no surface in the way, which is the thing
you do constantly.

**Dark, with Material You inside it.** The Apple apps force dark app-wide and
the reasoning transfers exactly: half the app is a terminal, a terminal is dark
whatever the device is set to, and a light list handing off to a black screen
looked like two applications. What does not transfer is declining the platform's
own colour, so the scheme is the system's *dark* dynamic palette — accents and
surfaces from the wallpaper — with only the terminal's own background pinned,
because that value is shared with the Mac and iOS.

**The keyboard is a `View`, not a text field.** A terminal has no text to hold,
no cursor to place and no selection to track. The input type says "visible
password" because that is what every Android keyboard reads as *do not help*:
no autocorrect rewriting a flag, no smart quotes, no autocapitalisation, no
learning from what is typed.

**The emulator runs on its own thread.** The Apple apps confine it to the main
actor. A reattach replays a whole screen — tens of kilobytes of escape sequences
on a busy agent — and parsing that on the thread Compose draws from is a visible
stutter at exactly the moment someone is watching.

## What this app has that iOS does not

Reachable over the same client FFI, and present on the Mac:

- **Every machine in one fleet**, above — the significant one.
- **Stop a running turn.** `terminal.agent_cancel` exists in the iOS client and
  is wired to nothing, so an agent going the wrong way can only be stopped there
  by switching the pane back to a terminal and pressing Ctrl-C — the one thing a
  chat surface is supposed to make unnecessary.
- **Context-window usage** on the composer, which is the number that tells you a
  long session is about to start forgetting.
- **Paste into a terminal**, bracketed if the program asked for it. A phone has
  no other way to get a command it did not type into a shell.

Hide/unhide and "new terminal in an existing worktree" were on this list and are
not any more: iOS gained both while this client was being written. That is the
right direction — the gap should keep closing from both ends.

## What iOS has that this app does not

Landed on `main` during the same week and not yet ported here. All three are
reachable over the client FFI, so each is a screen rather than a protocol
change:

- **Remove a worktree** (`workspace.remove_worktree`), with the two-phase
  confirmation macOS uses.
- **Add a repository root and register a repository**
  (`repository_root.add`, `repository.register`), so a machine can be set up
  from the phone rather than only from a terminal on the machine itself.
- **Hidden worktrees collapsed into a "Hidden N" section**, which is a better
  answer than this app's "N hidden" toggle.

## Notifications

Two channels, because Android lets a person silence one kind without silencing
the app and these two are genuinely different: an agent that is BLOCKED has
stopped and stays stopped until answered; an agent that FINISHED is news that
can wait. One channel would force the same answer for both, and the answer
people give to "too noisy" is to turn everything off.

Pushes to a sleeping phone go through the relay, which already speaks FCM
(`services/relay/src/push.ts`) — nothing on the server changed for Android.
They need a Firebase project:

1. Create one, add an Android app with the id `com.farcooler.android`.
2. Drop `google-services.json` into `apps/android/app/`.
3. Give the relay the matching `FCM_SERVICE_ACCOUNT`.

Without that file the Gradle plugin is not applied at all, the app builds and
runs, `PushRegistration` reports itself unavailable in Settings, and every local
notification keeps working. That is the same shape the iOS app has on the
simulator.

Sign-in needs a WorkOS client id, which is public by design:

```sh
./gradlew assembleRelease -Pfarcooler.workosClientId=client_...
```

## Security

- The **SSH private key** is encrypted with an AES key generated in the Android
  Keystore — on a Pixel, inside the Titan M chip — and only the ciphertext is in
  preferences. A file lifted off a rooted device is bytes nobody can decrypt.
- The **account's refresh token** is encrypted the same way, under a *different*
  Keystore alias: signing out must not be able to reach the device key, and
  losing the device key must not sign you out.
- Neither key can leave the hardware it was generated on, and backup and
  device-transfer are both excluded (`res/xml/data_extraction_rules.xml`), so a
  phone restored from another phone has to be authorized on the machine again —
  which is the behaviour you want the day a phone is lost.
- The private key never leaves the device. A machine authorizes it exactly the
  way it authorizes any other SSH client, and revokes it by deleting one line
  from `authorized_keys`.
