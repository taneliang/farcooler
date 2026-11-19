# ov-1367 validation

The branch includes the supplied base `5aebef04406a26f5206e44a3f912cefc6aa9fd13`.
The workflow conflicts retained main's manual shard selection and this branch's
additional shards; the selector includes those shards. Its self-test passed.

On 2026-11-19, `./scripts/ios-ui-tests.sh FarCoolerUITests/NativeAgentViewTests`
ran against revision `f251f4adff197e2d9d54eeb95483171b223de6b2` on an iPhone 17
iOS 27.0 simulator. All 25 tests ran and passed, with no skips. In particular,
`testASwipeDownClosesAnOpenImage` passed in 23.362 seconds. The complete output
is in [ios-ui-test.log](ios-ui-test.log). This run supersedes the earlier attempt
that stopped at the machine-load guard before tests started.
