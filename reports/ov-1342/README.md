# ov-1342 validation

Generated the iOS project with `python3 apps/ios/generate-project.py`, built simulator frameworks using `CARGO_BUILD_JOBS=2 ./scripts/build-ios-frameworks.sh`, and ran:

```
xcodebuild -project apps/ios/FarCooler.xcodeproj -scheme FarCooler -configuration Debug \
  -destination 'generic/platform=iOS Simulator' -derivedDataPath /private/tmp/ov-1342-ios \
  -jobs 2 ARCHS=arm64 CODE_SIGNING_ALLOWED=NO build
```

The build output listed FarCooler, FarCoolerActivity, FarCoolerNotify, FarCoolerWatch and FarCoolerWatchWidgets in its dependency graph and ended `** BUILD SUCCEEDED **`. The preceding compilation output included `SwiftCompile normal arm64` of `NeedsYouAggregate.swift` for each of those five targets. The initial generic simulator build compiled the sources but failed to link x86_64 because the framework script produces an arm64-only simulator slice; the successful command matches that slice.

Built the host client with `cargo build -p farcooler-client -j 2` and linked it at the AgentKit test package's expected release path. `swift test --package-path apps/shared/AgentKit --jobs 2 --filter <NeedsYouTests, NeedsYouWhoseTests and NeedsYouRunnerCountTests function names, plus PlanViewTests>` passed 29 tests, including the real Rust aggregate and plan bridge goldens.

## Review fix: bridge in extensions and watch

`TMPDIR=/private/tmp CARGO_BUILD_JOBS=2 ./scripts/build-ios-frameworks.sh` built the iOS and watch simulator client slices. The generated project was then built with the same `xcodebuild` command above, using `/private/tmp/ov1342-fix-build` as derived data. `/private/tmp/ov1342-fix-build.log` ends `** BUILD SUCCEEDED **` and includes the app, both phone extensions, watch app and watch widget. `nm -g` on each of the four extension/watch debug dylibs shows a defined `_farcooler_client_plan_view` symbol.

The standalone regression in `test/ios-needs-you-bridge` passed with the same Swift adapter files and forced static symbol link. It verifies a populated other-person ask, the ownership lookup, and duplicate-ID recovery and count. This host executable does not execute inside the watch simulator or an extension process; the target build and symbol checks exercise their generated link configuration.

## Rebase onto main

Rebased onto `origin/main` at `c62e46f2c`, retaining main's baseline entries and adding this ticket's three diagnostic exemptions. `PATH=/usr/bin:/bin:/usr/sbin:/sbin TMPDIR=/private/tmp sh scripts/lint-gate.sh` passed every lint, including shared-core and iOS AgentKit subset checks.

Rebuilt the iOS and watch simulator frameworks with `TMPDIR=/private/tmp CARGO_BUILD_JOBS=2 ./scripts/build-ios-frameworks.sh`, then ran the app scheme build above with derived data at `/private/tmp/ov1342-rebase-build2`. `/private/tmp/ov1342-rebase-build2.log` ends `** BUILD SUCCEEDED **`; its `SwiftCompile normal arm64` output names `NeedsYouAggregate.swift` in FarCooler, FarCoolerActivity, FarCoolerNotify, FarCoolerWatch, and FarCoolerWatchWidgets. The build ran outside the sandbox because Xcode's preview macro plugin could not run inside it.

`TMPDIR=/private/tmp cargo test -p farcooler-client -j 2 needs_you_view` passed all five targeted aggregation tests on the rebased tree. Earlier cross-platform targeted test evidence is recorded in `reports/ov-1336/README.md`.

## Watch device architecture investigation (not resolved)

The simulator evidence above does not cover the default Release watch device build, which also requests `arm64_32`. A direct `cargo build --release -p farcooler-client --target arm64_32-apple-watchos -Z build-std` with `RUSTC_BOOTSTRAP=1`, `rust-src`, the watch SDK and deployment target 26.0 failed in `aws-lc-sys` 0.43.0: `p256.c` assigns functions using 32-bit limbs to pointers using 64-bit limbs. The log is `/private/tmp/ov1342-arm32-release.log`. Setting `AWS_LC_SYS_NO_ASM=1` instead fails because that dependency permits it only for an unoptimized build (`/private/tmp/ov1342-arm32.log`). No universal device framework or Release watch build was produced. The architecture finding remains open.

`PATH=/usr/bin:/bin:/usr/sbin:/sbin TMPDIR=/private/tmp sh scripts/lint-gate.sh` passed on this tree; output is in `/private/tmp/ov1342-round2-lint.log`.
