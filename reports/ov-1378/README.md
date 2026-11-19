# ov-1378 validation

The branch was rebased without conflicts onto the supplied run-start base
`c2b141a465b62c95e103d25923d74741b66ea51d`. The tested revision was
`591e0e324` (before this evidence-only commit).

On 2026-11-19, after building simulator frameworks with
`./scripts/build-ios-frameworks.sh`, the command
`./scripts/ios-ui-tests.sh FarCoolerUITests/NativeAgentViewTests` passed on an
iPhone 17 iOS 27.0 simulator. All 25 tests ran, with no skips or failures;
`testASwipeDownClosesAnOpenImage` passed in 22.099 seconds. The full output is
in [ios-ui-test.log](ios-ui-test.log).
