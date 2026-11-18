# ov-1350: Android relay connection

## Fix round 2: preview sign-in setup corrected

Newest ticket finding `accdd821` requires the authenticated preview account and
paired Mac connection capture. The native interface fix is retained.

Opening the previous round's normal sign-in flow exposed an additional setup
failure, captured in [round2-invalid-redirect.png](round2-invalid-redirect.png):
WorkOS rejects `farcooler-local://auth` and suggests `farcooler-preview://auth`.
`preparePreview` had changed only the account relay/client preferences on a local
APK. `Account.REDIRECT_URI` and the manifest callback still come from the build
channel; preferences cannot change them. Thus the previous instructions could
not sign into preview even with an authorized test account.

The test now rejects that mixed setup before writing a fixture or using the
account, and requires a preview-channel APK with its public WorkOS client ID.
It no longer writes channel preferences. Build from a **clean checkout**:
`scripts/version.sh` deliberately resolves a dirty checkout to local even with
`FARCOOLER_CHANNEL=preview` set. These are local artifact settings, not remote
relay flags. Do not change WorkOS redirects to accommodate the local APK.

The authenticated connection acceptance remains open until an authorized preview
test account and paired Mac are available. The sign-in setup correction alone
does not establish that connection.

## Fix round 1: preview verification is still blocked

The review rejected the missing authenticated preview account connection. Its
workflow output says the public DERP connection and separate unauthenticated 401
do not establish that path. `task show ov-1350` (including JSON) had only creation
and status-change notes, so the finding was recovered from the review output.

The Android interface fix is retained. The device test now supports
`previewAccount=true`, requires the app's relay to be the preview origin, obtains
its session through production `Account.accessToken()`, requires an authenticated
200 from `/v1/account`, and checks that `previewMachineId` is listed before
attempting the tunnel. It records the successful account response metadata but
never credentials. The obsolete `previewProbe` argument fails explicitly instead
of silently continuing after a 401. `preparePreview=true` configures only the
disposable app's public relay URL and matching WorkOS client ID; it does not sign
in or change any remote relay setting. This setup was superseded in round 2
because it could not change the APK callback scheme.

The UI fixture also now replaces its previous `ov-1350-relay` entry. Previously
each run appended the same ID, and `RunnerStore.selected` chose the first entry,
so a new harness could pass the core test while the UI dialed a stale token.

Fresh checks on Nov 18:

- [round1-device-after.txt](round1-device-after.txt): real emulator-to-Mac tunnel
  succeeds with a new disposable SSH endpoint and the retained native fix.
- [round1-android-connected.png](round1-android-connected.png): fresh app capture.
- [round1-android-logcat.txt](round1-android-logcat.txt): fresh handshake/fleet
  response bytes and the UI's first contact through DERP, followed by direct UDP.
- [round1-preview-blocked.txt](round1-preview-blocked.txt): strict preview run
  fails because the disposable emulator has no signed-in preview session.
- Go format check, vet and tests pass with cgo enabled and disabled. Gradle
  `assembleInstrumented assembleAndroidTest` passes with the updated test.
  [round1-checks.txt](round1-checks.txt) records the commands and outcomes.

**Remaining acceptance work:** sign the disposable app into an authorized preview
test account, pair the disposable Mac to that account, and capture the strict
preview run below plus the connected UI. No account/token preferences were
provisioned in the emulator. A test-account sign-in route/session location was
requested; none was available during this run. No authenticated preview success
is claimed. The earlier red/green native regression evidence remains below.

The preview account Worker does not proxy SSH: `Account.pairDaemon` pairs push
notifications, and `crates/tailcat/go/tailcat.go:newClient` carries SSH through
DERP. The strict check verifies the account service and then the tunnel in one
run; the tunnel still uses DERP, as in production. It does not discover a tunnel
route from the account response, which has no such field.

## Original root cause and evidence

Root cause: Android blocks the `NETLINK_ROUTE` bind used by Go's `net.Interfaces` in apps targeting SDK 30 or later. This app targets 37. Tailcat's first `Ping` constructs Tailscale's network monitor, which enumerates interfaces before opening the relay connection. No Android interface getter had been registered. The connection therefore stopped locally, before SSH or a daemon request, with the generic `tunnel=io` result.

The emulator capture names the failing operation:

```text
tailcat: dial ping failed: netmon.New: route ip+net: netlinkrib: permission denied
connect stopped: trouble=tunnel tunnel=io
```

`tailscale.com/net/netmon/state.go` documents this exact Android SDK restriction beside `RegisterInterfaceGetter`. The fix registers an Android-only getter backed by Bionic `getifaddrs`, including copied addresses and masks so `Interface.Addrs` does not re-enter Go's forbidden netlink path. It enumerates afresh on each poll, rather than caching a startup snapshot. Other platforms retain their existing getter. Android's Go diagnostics now reach logcat, and failed pings retain the underlying diagnostic.

This is a platform initialization failure, not evidence that today's Kotlin changes broke the transport. The captured failure occurs before those views or Needs You reads run.

## Evidence and limits

- [android-logcat.txt](android-logcat.txt): failing netlink operation; real Android response from `https://relay-preview.farcooler.com/v1/account`, `HTTP 401 {"error":"unauthorized"}`; successful DERP contact; pinned SSH connect; `probe` and `fleet` response bytes.
- [device-before.txt](device-before.txt): the regression test fails with only `RegisterInterfaceGetter(androidInterfaces)` disabled. The same run establishes the two existing terminal-test failures.
- [device-after.txt](device-after.txt): the final relay test passes with the getter restored.
- [mac-sshd.txt](mac-sshd.txt): real OpenSSH authentication, forced daemon sessions, and transmitted/received byte counts on the Mac.
- [android-connected.png](android-connected.png): the production Android UI displaying the disposable Mac's successfully read, empty fleet.
- [bridge-after.txt](bridge-after.txt): 13 device tests, with only the same two terminal-size failures present before the fix. `aGridTooSmallToBeATerminalIsClampedRatherThanHonoured` expects 20 columns but gets 3; `aNegativeDimensionSaturatesRatherThanWrapping` expects 20 but gets 1. VT code is unchanged by this ticket.
- [checks.txt](checks.txt): all 1,627 Android JVM tests pass; Go tests pass with cgo enabled and disabled; Android arm64 JNI/tunnel and x86_64 tunnel compile; debug, instrumented and test APKs build.

The device was an API 37.1, 16 KB page, arm64 emulator. The Mac endpoint was a disposable loopback OpenSSH server plus the shipped Go tailcat helper and the installed Canary daemon (`0.1.0+a2d96bc54`). The client's host key was pinned. The first successful connect completed before the logged upgrade from DERP to direct UDP; both used the same tailcat route. The production UI subsequently connected with the emulator app's own enrolled keys.

The preview **account** relay and tailcat's **DERP** relay are separate services. The preview probe deliberately has no account token and verifies only reachability and its actual refusal bytes. Signed-in preview enrollment was not exercised. The tunnel test uses disposable, explicitly admitted node/SSH keys against a real Mac and public DERP, with no mock networking. No remote relay flags or deployments were changed.

The emulator console packet recorder produced only a PCAP header; it is not included or presented as packet evidence. The retained evidence is logcat response bytes, Mac SSH byte counts and the actual app screenshot.

## Reproduce

Use an isolated emulator, not a person's installed app. Build the native libraries and APKs:

```sh
./scripts/build-android-libs.sh
cargo build -p farcooler-jni
export FARCOOLER_CHANNEL=preview
export JAVA_HOME=/opt/homebrew/opt/openjdk@17/libexec/openjdk.jdk/Contents/Home
export ANDROID_HOME="$HOME/Library/Android/sdk"
apps/android/gradlew -p apps/android testInstrumentedUnitTest \
  -Pfarcooler.nativeLibDir="$PWD/target/debug" \
  -Pfarcooler.workosClientId=client_01M03MXE27CHCZJJ0TMY7KCMAH \
  assembleInstrumented assembleAndroidTest
(cd crates/tailcat/go && CGO_ENABLED=0 go build -o /tmp/ov-1350-helper .)
```

The installed emulator 37.1.11 reported working host acceleration but ran the guest in software and was unreliable. A fresh AVD under `/tmp`, using Google's stable emulator 37.2.13 / build 16740729, booted successfully. The downloaded archive SHA-1 was `6c5e61b873e11c7d1d08683bea089f762baee070`; the installed SDK was left unchanged. Select your working emulator with `ANDROID_SERIAL`.

```sh
export ANDROID_SERIAL=emulator-5560
export PATH="$ANDROID_HOME/platform-tools:$PATH"
adb install -r apps/android/app/build/outputs/apk/instrumented/app-instrumented.apk
adb install -r apps/android/app/build/outputs/apk/androidTest/instrumented/app-instrumented-androidTest.apk
```

For the optional production UI capture, first create the emulator's real device keys and export **only** their public halves:

```sh
adb shell am instrument -w \
  -e class com.farcooler.core.RelayConnectionDeviceTest#prepareThePhone \
  -e prepareRelay true -e preparePreview true \
  com.farcooler.android.preview.test.test/androidx.test.runner.AndroidJUnitRunner
adb exec-out run-as com.farcooler.android.preview.test cat files/relay-offer.json > /tmp/ov-1350-offer.json
```

Keep the following harness running in another terminal. It creates its own keys, SSH configuration, allowlist and runtime; it does not edit the owner's SSH configuration or runner. Choose a new scratch directory. `config.json` contains throwaway private keys and must not be committed.

```sh
python3 reports/ov-1350/runner.py \
  --directory /tmp/ov-1350-repro \
  --helper /tmp/ov-1350-helper \
  --daemon '/Applications/Far Cooler Canary.app/Contents/Resources/farcoolerd' \
  --openssl /opt/homebrew/bin/openssl \
  --offer /tmp/ov-1350-offer.json
```

Run the tunnel regression and UI setup (this alone does not verify the preview account):

```sh
adb push /tmp/ov-1350-repro/config.json /data/local/tmp/ov-1350-config.json
adb shell am instrument -w \
  -e class com.farcooler.core.RelayConnectionDeviceTest#aTunnelCarriesTheAndroidClientToTheRunner \
  -e relayConfig /data/local/tmp/ov-1350-config.json \
  -e showRunner true \
  com.farcooler.android.preview.test.test/androidx.test.runner.AndroidJUnitRunner
adb shell am start -W -n com.farcooler.android.preview.test/com.farcooler.ui.MainActivity
adb logcat -d -s Tailcat:I RelayConnection:I
```

Omit `--offer`, `prepareThePhone` and `showRunner` for a core-only integration run. The test is skipped unless `relayConfig` is supplied. Stop the harness with Ctrl-C; it terminates only its own helper and sshd. All verification here used the separate `.preview.test` application ID (rounds 0/1 used `.local.test`).

For authenticated preview verification, first sign the disposable app into a
preview test account through its normal sign-in UI. Pair the disposable Mac to
that account using the app's enrollment flow, then use its actual registration
ID from `/v1/account` (never a placeholder):

```sh
adb shell am instrument -w \
  -e class com.farcooler.core.RelayConnectionDeviceTest#aTunnelCarriesTheAndroidClientToTheRunner \
  -e relayConfig /data/local/tmp/ov-1350-config.json \
  -e previewAccount true -e previewMachineId "$PREVIEW_MACHINE_ID" \
  -e showRunner true \
  com.farcooler.android.preview.test.test/androidx.test.runner.AndroidJUnitRunner
```

Only a run with the authenticated account response, matching Mac registration,
successful tunnel/SSH and fleet reads completes this verification. A missing
session, refused request or missing registration fails the test.

Status: **BLOCKED** on authenticated preview test sign-in. The local native
failure is reproduced and fixed, its regression is red/green, and an actual
emulator-to-Mac DERP connection is captured. The preview acceptance gap remains
open; the two unrelated VT assertions are recorded above.
