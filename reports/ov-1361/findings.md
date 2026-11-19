# ov-1361 findings

The JDK fixture previously launched shell executables through the real preflight's
0.5-second per-candidate and 2-second aggregate deadlines. A delayed fake probe
can therefore be rejected as a non-working JDK; the failure message does not
record whether the particular observed probe timed out. The self-test now returns
synthetic process results for only its fake Java and Node paths, retaining the
production validation logic and deadlines.

The saved pre-queue record (`/private/tmp/claude-502/-Users-e-liang-Library-Application-Support-com-farcooler-FarCoolerCanary-workspaces-01a0e3c7-3e2c-7ed0-b31c-0cc0d7401571/01a1b521-e194-79c3-9ed0-fe3c32b5b418/scratchpad/pq-1115.log`) contains only `Ran 24 tests` and `FAILED (errors=1)`, without a test name or traceback. Its cause cannot be established from that record.

During validation, a distinct benchmark test failed: its trial's ready file can
appear before the collector's `Popen` returns, so the test's termination signal
can interrupt process creation outside the protected wait. The test now signals
only after the collector enters that wait. This was a nine-test suite failure,
not evidence identifying the earlier 24-test error.

The first loaded attempt also found an incompatible `/usr/local/bin/python3`
for a fixture's `env python3` shebang. Subsequent runs use the native
`/usr/bin/python3` first on PATH. Full output, including failed attempts, is in
`self-test-load.log`.

A subsequent unsandboxed run (`self-test-load-20.log`) passed five consecutive
runs at start/end one-minute loads above 15. The execution session was
interrupted during run six, so this is not a completed 20-run validation. The
sandboxed attempt could not run process-guard tests because it blocks `ps`.
