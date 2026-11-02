# ov-904 source investigation

Source: `18f269cb2`, branch `ov-904-tracing-race`. No production edits or tests run yet. Execution requires focused allocation from the orchestrator.

## Preserved original evidence

Original bundle: `/Users/e-liang/Dev/overnight/.claude/agent/reports/ov-893/`.
Read `implementation.md`, `review-implementation.md`, `green.log`, and `green-continuation.log`. Local `/tmp` copies of the logs/review were also consulted; byte comparisons are recorded in the tool transcript.

At `01f583ea3`, `cargo test -p farcooler-core session_log::projector` failed: 147 passed, 1 failed, 4 ignored. Exact failure: `session_log::projector::record_types_tests::an_unknown_type_is_a_gap_and_logged_once_by_name`, `record_types_tests.rs:104`: expected one `brand-new-record`, got zero, with an empty captured string. The preceding gap assertion passed.

The same source's exact isolated test passed (1 test); serial projector run passed (148 tests, 4 ignored). Neither pass diagnoses or waives the parallel failure. Subsequent clippy failure in the continuation is separate.

## Source trace and hypothesis

`fixtures::fold` constructs a new Projection and folds each line synchronously. `record_types.rs::Projection::unknown_type` hashes each type into a per-projection set, warns for the first 32 distinct types, then inserts a Gap. Its warning at line 60 is the single shared static tracing callsite.

Both capture tests independently construct fmt subscribers and install them with thread-local `with_default`. Other parallel projector tests fold unknown types without a subscriber. There is no async hop or global test subscriber in these paths.

Hypothesis (unconfirmed): first registration of this shared callsite on a thread without a subscriber can cache `Interest::never` despite another thread having a live local capture subscriber. Installed tracing-core's `Dispatchers::rebuilder` uses a `JustOne` optimization and `Rebuilder::for_each` then reads the registering thread's default dispatcher. This would explain successful Gap production and completely empty capture, and why isolated/serial ordering avoids the failure. It is not yet evidence of a production reducer defect.

## Bounded reproduction proposal

Use a fresh child process to avoid previously registered callsites/subscribers. Explicit barriers establish this ordering: create and hold one local capture dispatcher on thread A; thread B with no subscriber folds the unknown fixture first; thread A folds it afterward under its held dispatcher. Assert the original exact once-per-name, three-line, secret-redaction and Gap checks. No retry loop, sleeps, global subscriber, or production warning modification.

Compare a fresh-process control where the first fold occurs under the capture subscriber. Then exercise two simultaneous local capture subscribers with independent sinks, asserting complete counts and no cross-capture. Bound executions to these scenarios and the record_types_tests filter. If the hypothesized ordering does not fail, retain the result and revise the hypothesis before changing source.

Only after confirmed red: evaluate a test-local dispatcher lifetime/initialization solution. Retain the original assertions and parallel gate. Any needed fix must have meaningful red-green, concurrency coverage, signed checkpoint and independent review. No dependency/global tracing/config change is authorized by this proposal.

## Pending

Focused allocation requested through the orchestrator. Acceptance remains open. No whole-suite retry, provider run, stress loop, waiver, or blanket serialization performed.

## Allocated execution protocol

Orchestrator granted two phases: normal `scripts/heavy`, load limit 20, jobs 6; release lock while coding. First run only `cargo test -p farcooler-core --lib session_log::projector::record_types_tests::capture_registration_order -- --exact --nocapture`. This parent test starts exactly two fresh copies of the test executable (control, then unsubscribed-first); each has a ten-second deadline enforced by the parent, which kills and waits for its own child on expiry. Child threads coordinate with barriers; no timing loops determine the interleaving. The child runs only this exact test, with the scenario in a dedicated environment variable. Original assertions remain intact.

The second phase, after root cause is demonstrated and a candidate prepared, runs only `cargo test -p farcooler-core --lib session_log::projector::record_types_tests -- --nocapture`, including explicit concurrency coverage. No full projector or workspace suite is allocated. Save raw outputs and exit statuses under this report directory. No heavy lock is held between these runs.
