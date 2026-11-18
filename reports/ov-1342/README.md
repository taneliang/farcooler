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
