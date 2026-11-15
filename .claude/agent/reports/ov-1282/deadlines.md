# ov-1282 deadline sweep

Changed waits (all poll immediately and return on success):

- `gate_tests`: verdict receipt, parallel reviewer and gate startup; the ignored stage timing experiment's fixed pump count.
- `coding_tests`: owned coding process startup.
- `coding_native_delivery_tests`: native prompt submission and each delivered message, pumping wakes on every poll.
- `coding_verbs_tests`: CI/work-place watcher; the enclosing sweep and held-build script have compatible bounds.
- `concurrency_tests`: completion after the later landing and the state watcher; the held-build script allows the watcher to use its deadline.
- `builder_resources/tests`: builder startup before cancellation.
- `workflow_commands_over_socket`: admission and completion over the observer connection.
- `workflow_service_over_socket`: approval progression, tracker completion, command startup, child socket startup, fast completion and recovered CI completion; `workflow_service/safety`: unaffected and drifted workspace progression.
- `workflow_draft_approval` and `engine_flow_per_skill`: card settlement; the latter also waits for workflow state with a deadline.

The remaining short waits in `admission_tests` and `landed_tests` receive in-process inbox messages; `review_tests` waits for a terminal frame; `coding_tests` cancellation/reconciliation, `build_cleanup_tests` cleanup/release, `tests.rs` journal/state waits, and `workflow_service` adoption and CI recovery waits observe transitions after startup. Their timeouts test response or cleanup, not process startup. `coding_verbs_tests` reaper waits and zombie observation, `gate_tests` orphan reaping, and integration service shutdown waits test stopping an already-running process. The five pumps in `tests.rs` exercise repeated reads rather than waiting for a condition; the five samples in the ignored gate timing experiment are measurement repetitions. Other fixed pumps use logical workflow steps and simulated timestamps, not sleep-based startup polling. The remaining `engine_flow_per_skill` 1200/1800 iteration waits allow at least 60 seconds; the longer live native probe and remote-object waits already allow at least 60 seconds. Short transport quiet-period and performance assertions elsewhere in integration tests intentionally test timing or absence rather than startup.

Validation: `TMPDIR=/tmp cargo nextest run -p farcooler-daemon workflow_runtime -j4` passed 448 tests (after building the real CLI). The ignored timing experiment passed separately. The four affected integration binaries passed 31 tests, with one child fixture intentionally skipped. The full workflow run preceded the change to the ignored experiment; that experiment was then run explicitly. Initial sandboxed execution failed because local fixture operations were denied, and an earlier unrestricted run identified the missing CLI prerequisite.
