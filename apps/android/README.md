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

That writes two libraries into `app/src/main/jniLibs/<abi>/`:
`libfarcooler_jni.so`, the Rust cores, and `libtailcat.so`, the Go tunnel it
links against — Android is the one platform that loads the tunnel rather than
bundling it, because Go cannot build an archive for Android at all. Both are
needed: the core will not load without the tunnel beside it. Then:

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
./gradlew assembleAndroidTest                # compiles that suite, no device
```

The third is what CI runs. The device suite was compiled by nothing at all
until it was added — not by `testInstrumentedUnitTest`, which builds
`src/test/` only, and not by `assembleDebug` — so it could stop compiling with
every check still green. CI still does not RUN it; that needs an emulator, and
`.github/workflows/ci.yml` says why the two halves are worth keeping apart.

The first two say `Instrumented` because both build against that build type,
which has its own application id. AGP uninstalls the app under test when a connected run
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

## Canary on Google Play

Every build of `main` that passes CI goes to Play's internal testing track as
`com.farcooler.android.canary`, from the `Android (Play internal testing)` job in
`.github/workflows/canary.yml`. Your phone installs it from Play and updates by
itself, the way TestFlight does for iOS: no USB debugging, no sideloading.

Play App Signing holds the key your phone verifies. CI holds only an **upload
key**, which proves a bundle came from us and which Google can reset if it is
lost. `versionCode` is the commit count (`scripts/version.sh build`), the same
number iOS uses, so it only ever goes up.

Until the steps below are done the job skips and says why in the run's
annotations. Nothing fails.

### One-time setup

1. **Generate the upload key.** Anywhere outside the repository:

   ```sh
   keytool -genkeypair -v -keystore farcooler-upload.jks -storetype PKCS12 \
     -alias farcooler-upload -keyalg RSA -keysize 2048 -validity 10000
   ```

   Store the `.jks` file and both passwords in your login Keychain, and add
   them to GitHub as secrets (below). Never put the keystore in the repository.

2. **Create the app in Play Console.** All apps, Create app. Name it Far Cooler
   Canary, then finish the dashboard's required declarations. Its package name
   is fixed by the first upload, and must be `com.farcooler.android.canary`.
   Accept Play App Signing when asked.

3. **Upload the first bundle by hand.** Play requires this before its API
   accepts anything. Build it with the upload key, then in Play Console go to
   Testing, Internal testing, Create new release, and upload it:

   ```sh
   ./scripts/build-android-libs.sh
   cd apps/android
   FARCOOLER_CHANNEL=canary \
   ANDROID_UPLOAD_KEYSTORE=/path/to/farcooler-upload.jks \
   ANDROID_UPLOAD_STORE_PASSWORD=... ANDROID_UPLOAD_KEY_ALIAS=farcooler-upload \
   ANDROID_UPLOAD_KEY_PASSWORD=... \
   ./gradlew bundleRelease -Pfarcooler.sign=true
   # app/build/outputs/bundle/release/app-release.aab
   ```

   Run it from a full clone: `versionCode` is the commit count, and the script
   refuses a shallow one.

4. **Add internal testers.** Testing, Internal testing, Testers: create an
   email list with your Google account. Copy the opt-in link from the same page
   and open it on the phone once. After that the app is in Play's updates.

5. **Create a service account.** Google Cloud console, IAM, Service accounts,
   create one, then Keys, Add key, JSON. In Play Console, Users and
   permissions, invite its email and grant it release permission for this app
   (Release to testing tracks is enough). Enable the Google Play Android
   Developer API on the same Cloud project.

6. **Add the GitHub secrets** (Settings, Secrets and variables, Actions):

   | Secret | What |
   | --- | --- |
   | `ANDROID_UPLOAD_KEYSTORE_B64` | `base64 -i farcooler-upload.jks` |
   | `ANDROID_UPLOAD_STORE_PASSWORD` | the keystore password |
   | `ANDROID_UPLOAD_KEY_ALIAS` | `farcooler-upload` |
   | `ANDROID_UPLOAD_KEY_PASSWORD` | the key password |
   | `PLAY_SERVICE_ACCOUNT_JSON` | the whole service account JSON file |

   Then set the repository **variable** `CANARY_PLAY` to `true`. That is your
   statement that step 3 is done; without it the job skips.

7. **Firebase push (optional).** In the Firebase console add an Android app with
   package `com.farcooler.android.canary` to the canary project and download its
   `google-services.json`. Add it as the secret `GOOGLE_SERVICES_JSON_CANARY`
   (the file's contents). CI writes it next to `app/build.gradle.kts` before the
   build. Without it the app builds and works, and push is unavailable. For a
   local build, put the file at `app/google-services.json`; it is gitignored.

### How signing is gated

`./gradlew assembleRelease` is unchanged: unsigned, no secrets, the compile
check CI runs. `-Pfarcooler.sign=true` signs the release build with the upload
key, read from the environment or from `farcooler.upload.keystore`,
`.keystoreB64`, `.storePassword`, `.keyAlias` and `.keyPassword` Gradle
properties. If any is missing the build fails and names it. `bundleRelease`
refuses to run at all without `-Pfarcooler.sign=true`, so the bundle that gets
uploaded can never be unsigned.

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
own color, so the scheme is the system's *dark* dynamic palette — accents and
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

- **Remove a worktree** (`worktree.remove`), with the two-phase
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
