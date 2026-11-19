# ov-1367 validation

The branch was rebased without conflicts onto `origin/main` at
`a323b6576` (which includes the supplied base `5aebef044`).

On 2026-11-19, `./scripts/ios-ui-tests.sh FarCoolerUITests/NativeAgentViewTests`
ran against rebased revision `792c2b5bcca2348ed390deb7656e91175f0df078` on an
iPhone 17 iOS 27.0 simulator. All 25 tests ran and passed, with no skips.
`testASwipeDownClosesAnOpenImage` passed in 23.294 seconds. The complete output
is in [ios-ui-test.log](ios-ui-test.log).

Fix-round 3 verification: `git merge-base --is-ancestor 5aebef044 HEAD`
and `git merge-base --is-ancestor a323b6576 792c2b5bc` both succeeded. Thus
the tested revision contains both the supplied run-start base and the main
revision named in the latest review. The log records 25 collected, 0 skipped,
and 25 actually run.
