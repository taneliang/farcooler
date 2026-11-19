# ov-1367 validation

Rebased onto the supplied base `5aebef04406a26f5206e44a3f912cefc6aa9fd13`.
The workflow conflicts retained main's manual shard selection and this branch's
additional shards; the selector now includes those shards.

`/usr/bin/python3 scripts/ios-ui-shards.py --self-test` passed.

The first `scripts/ios-ui-tests.sh FarCoolerUITests/NativeAgentViewTests` attempt
failed at build because the checkout had no simulator XCFrameworks. The
frameworks were built successfully with `scripts/build-ios-frameworks.sh`.
The second attempt used the same UI test script and class, with the native
system Python first on PATH. Its log is in [ios-ui-test.log](ios-ui-test.log).
It waited at the script's machine-load guard; the five-minute load rose from
26.7 to 37.2 (guard threshold 20), so the waiting run was stopped. No UI test
ran in that attempt, and the image-swipe pass remains unverified. Rerun the
same command when the machine is idle.
