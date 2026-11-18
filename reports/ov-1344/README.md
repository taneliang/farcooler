# ov-1344 validation

The watch and its widget compile the snapshot model without the Rust-calling adapter. The phone extensions also have no Rust link, matching origin/main. The phone app computes the aggregate before publishing the snapshot.

- `xcodebuild -project apps/ios/FarCooler.xcodeproj -target FarCoolerWatch -configuration Release -sdk watchos -jobs 2 CODE_SIGNING_ALLOWED=NO build` succeeded for the watch app and widget, including arm64_32. Log: `/private/tmp/ov1344-watch-target.log`.
- `TMPDIR=/private/tmp cargo test -p farcooler-client -j 2 needs_you_view` passed five tests. The standalone `test/ios-needs-you-bridge` passed its bridge, ownership and duplicate-recovery checks after a host client build.
- `PATH=/usr/bin:/bin:/usr/sbin:/sbin TMPDIR=/private/tmp sh scripts/lint-gate.sh` passed. Log: `/private/tmp/ov1344-lint.log`.
- After `TMPDIR=/private/tmp CARGO_BUILD_JOBS=2 ./scripts/build-ios-frameworks.sh`, `xcodebuild -project apps/ios/FarCooler.xcodeproj -scheme FarCooler -configuration Debug -destination 'generic/platform=iOS Simulator' -derivedDataPath /private/tmp/ov1344-ios -jobs 2 ARCHS=arm64 CODE_SIGNING_ALLOWED=NO build` succeeded. Log: `/private/tmp/ov1344-ios.log`.

Rework validation (2026-11-18): `PATH=/usr/bin:/bin:/usr/sbin:/sbin TMPDIR=/private/tmp sh scripts/lint-gate.sh` passed. A new `scripts/landing-gate` run stopped at disk admission before running checks: 48.83 GB free against its 61 GB requirement. The available previous gate logs in `/private/tmp/lane-gate-logs-uk908q8o` record passing iOS build-for-testing, Swift tests, Rust lints and 5,037 Rust tests, but do not establish a complete gate pass. The default PATH's `/usr/local/bin/python3` also fails with `Bad CPU type in executable`; the system Python works.
