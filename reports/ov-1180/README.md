# ov-1180: Android launch initialization

## Cause and mapping

The owner’s original crash is preserved in `crash-logcat.txt` (Pixel 10 Pro,
canary 5128, commit `49bf5fdb0125957752b8e027f0db6f88ff3f0a40`).
`qn.<init>:953` is **AppModel’s `fleet` property**, AppModel.kt:165.
Its `FleetRepository` constructor launches collection at FleetRepository.kt:114;
a saved runner starts `Connection.start` at FleetRepository.kt:149, which reads
`Identity.privateKey` at Connection.kt:934. `Identity.readOutcome` then reads
`preferences` at Identity.kt:207 before AppModel’s later init block initializes it.
Main.immediate can run this path while AppModel’s constructor is still on the stack.

The existing `/Users/e-liang/Dev/overnight/apps/android/app/build/outputs/mapping/release/mapping.txt`
was from September 1 and mapped AppModel to `yj`, so it was unsuitable for this crash.
The exact release commit’s Android sources were extracted with `git archive` into
`apps/android/build/ov1180-baseline/` **inside this worktree**, then built with
JDK 17 and `./gradlew assembleRelease` (local channel, release build type).
The regenerated mapping is at
`apps/android/build/ov1180-baseline/apps/android/app/build/outputs/mapping/release/mapping.txt`.
`baseline-mapping-excerpt.txt` preserves the relevant original and inlined frames.
It confirms `qn`, `n3.Q:282`, `n3.N:4`, `tv0.a0:106`, `ou0.t:129`,
and the `953:955` constructor range, including the property’s exact source line.
The regenerated map ID differs from the shipped ID (no private Firebase/WorkOS
configuration was used); this is not presented as the original production mapping.
The independent unminified regression reproduces the same source path.

## Fix and store audit

`FarCoolerApp.onCreate` now initializes Identity, NodeIdentity, and Themes,
before any activity, view model, service, or receiver can read them. The manifest
already selects FarCoolerApp. AppModel no longer owns global initialization.
This covers background entry points as well as launch and recreation.

A search of all main Kotlin sources for `fun initialize` and `.initialize(` found
exactly three context-initialized singleton stores: Identity and NodeIdentity in
`data/Identity.kt`, and Themes in `data/Themes.kt`. All three now share this one
application initialization point. Other initialize calls configure cryptographic
generators, not context stores. RunnerStore, Settings, Account/TokenStore and
PreferenceReviewStorage receive their context in their constructors instead.
Themes also explicitly loads the JNI library before reading built-in palettes.
Moving its initialization earlier exposed its dependency on ClientCore having
already loaded the library: without that guard the process remained on one
fallback theme. The native-backed assertion failed before the guard
(`themes-before.xml`) and verifies that more than one built-in theme is loaded.
No persistence names, encryption, key generation, or account state were changed.

## Regression evidence

`AppModelStartupTest` runs in the ordinary JVM test lane, not the optional captures
lane. Robolectric uses SDK 33 so the preferences/lifecycle ordering test runs on
JDK 17; the real release is compiled for and tested on Android API 37.
It uses the real FarCoolerApp and AppModel(Application, SavedStateHandle), seeds a
saved offline runner, and uses eager main dispatch. `runTest` reports uncaught
child-coroutine failures instead of allowing them to appear only in stderr.
The ViewModelStore is cleared so model jobs and connectivity callbacks are released.
The test runner shares only the four JNI bridge classes with the ordinary JVM
tests: the JVM permits one native-library load per class loader, so a separate
Robolectric copy could otherwise make existing native tests lose the library.

Before the fix: both tests failed with `lateinit property preferences has not
been initialized`. `startup-before.xml` records the constructor path through
AppModel.kt:165 and Identity.kt:207. After the fix: both pass, recorded in
`startup-after.xml`. The other test reads Identity/NodeIdentity and verifies a
Themes selection persists **before any model is constructed**.

## Validation

Commands run from this worktree; Gradle uses
`JAVA_HOME=/opt/homebrew/opt/openjdk@17/libexec/openjdk.jdk/Contents/Home` and
`ANDROID_HOME=/Users/e-liang/Library/Android/sdk`.

- `./scripts/build-android-libs.sh`: passed, arm64 core and tunnel built here.
- `cargo build -p farcooler-jni`: passed, supplies host JNI for existing JVM tests.
- `cd apps/android && ./gradlew testInstrumentedUnitTest -Pfarcooler.nativeLibDir=<absolute worktree>/target/debug`:
  passed, 1,621 tests, zero failures/errors/skips. Initial runs without a usable
  host JNI path failed existing native-dependent tests; supplying the built core
  resolved them. The final combined run is recorded in `final-validation.log`.
- `./gradlew assembleRelease assembleAndroidTest`:
  passed. Release has R8 minification and resource shrinking enabled.
- File-size budget, doc-comment check against HEAD, and `git diff --check`: passed.

## Emulator setup

Only explicitly selected emulator serials were used; the owner’s phone and
`~/.ssh` were never accessed. The installed emulator 37.1.11 started dummy CPU
threads with the normal flags. Testing used an APFS clone of the emulator under
`apps/android/build/ov1180-emulator`, ad hoc signed with hypervisor/JIT entitlements,
and the ARM-specific `-qemu -enable-hvf` switch. Both installed images also
crashed in SurfaceFlinger’s RegionSampling thread (`hasReadColorBufferDma`).
The API 37 `fc2` emulator completed boot after setting `debug.sf.luma_sampling=0`
and restarting SurfaceFlinger. This property is documented in the
[AOSP SurfaceFlinger source](https://android.googlesource.com/platform/frameworks/native/+/da901bf/services/surfaceflinger/SurfaceFlinger.cpp).
This is an emulator-only workaround, with no product code change. The installed SDK was not changed.
The successful emulator command is:

```sh
ANDROID_HOME=/Users/e-liang/Library/Android/sdk \
  apps/android/build/ov1180-emulator/emulator -avd fc2 \
  -no-window -no-audio -no-snapshot -port 5558 -gpu swiftshader \
  -feature -Vulkan -qemu -enable-hvf
```

The unsigned release APK was signed into `ov1180-emulator.apk` with the existing
standard Android debug keystore, without changing release configuration or source.

## Release launch result and acceptance

Installed the final minified/resource-shrunk `release` APK (local channel,
`DEBUG=false`, application id `com.farcooler.android.local`) on `emulator-5558`:
API 37, arm64, 4 KB pages. Both a fresh launch and a cold launch with a saved
offline runner succeeded. The saved-runner launch reported Status ok, COLD,
610 ms, resumed MainActivity and a live process after 15 seconds, with no
FATAL EXCEPTION or uninitialized-property error in AndroidRuntime logcat.
See `release-smoke.json`, `release-saved-runner-launch.log`,
`release-activity.txt`, `release-logcat.txt`, and `emulator-device.txt`.
Notification permission was granted to the emulator test app to keep its system
dialog from covering the activity under test.

A repeated force-stop check hit the emulator’s unrelated task-snapshot GPU
assertion in system_server; that optional repeated-launch check is not claimed
as passing. Likewise screenshot capture hits that same driver assertion.
The recorded cold-launch check was performed after the emulator recovered,
without taking a screenshot or triggering a closing-task snapshot.
The app itself reported no fatal Java exception. No claim is made about a
16 KB image launch: its system graphics stack could not finish booting.

- [x] Obfuscated constructor mapped to AppModel.fleet with source-line evidence.
- [x] Real AppModel startup test fails before the fix and passes afterward.
- [x] Release-config APK installed on an emulator and cold-launched without a FATAL EXCEPTION.
- [x] All three initialize(context) stores share the early application init point;
  readiness, persistence and native theme loading are covered by regression tests.

Final combined validation: 1,621 JVM tests, zero failures/errors/skips;
release assembly and Android device-test compilation passed. The test runner
shares real JNI declarations, without mocking AppModel, fleet, identity, or
store initialization. The owner’s phone was never accessed.
