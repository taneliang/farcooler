# ov-1350: Android relay connection

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
export JAVA_HOME=/opt/homebrew/opt/openjdk@17/libexec/openjdk.jdk/Contents/Home
export ANDROID_HOME="$HOME/Library/Android/sdk"
apps/android/gradlew -p apps/android testInstrumentedUnitTest \
  -Pfarcooler.nativeLibDir="$PWD/target/debug" \
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
  -e prepareRelay true \
  com.farcooler.android.local.test.test/androidx.test.runner.AndroidJUnitRunner
adb exec-out run-as com.farcooler.android.local.test cat files/relay-offer.json > /tmp/ov-1350-offer.json
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

Run the regression and optional preview probe/UI setup:

```sh
adb push /tmp/ov-1350-repro/config.json /data/local/tmp/ov-1350-config.json
adb shell am instrument -w \
  -e class com.farcooler.core.RelayConnectionDeviceTest#aTunnelCarriesTheAndroidClientToTheRunner \
  -e relayConfig /data/local/tmp/ov-1350-config.json \
  -e previewProbe true -e showRunner true \
  com.farcooler.android.local.test.test/androidx.test.runner.AndroidJUnitRunner
adb shell am start -W -n com.farcooler.android.local.test/com.farcooler.ui.MainActivity
adb logcat -d -s Tailcat:I RelayConnection:I
```

Omit `--offer`, `prepareThePhone` and `showRunner` for a core-only integration run. The test is skipped unless `relayConfig` is supplied. Stop the harness with Ctrl-C; it terminates only its own helper and sshd. All verification here used the separate `.local.test` application ID.

Status: **DONE_WITH_CONCERNS** — relay failure reproduced and fixed, regression proven red/green, actual emulator-to-Mac connection captured. The two pre-existing VT assertions and authenticated preview enrollment limit are recorded above.
