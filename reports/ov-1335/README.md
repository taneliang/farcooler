The three largest proposed changes are: move the exhaustive crash matrix to nightly (**405.8 summed test-seconds** removed from main); replace the remote timeout and PR-watch real waits with controlled time or signals (**about 15 s**, exploratory); and construct the long-history fixture in bulk (**about 10 s**, exploratory). These are estimates of summed test-process time, not wall-clock savings. The first sacrifices per-landing coverage; retain a representative crash smoke test in main, so its net saving would be smaller. The latter estimates are hypotheses, not measured breakdowns.

# ov-1335 — daemon test time

Source: `/tmp/lane-gate-logs-j3pvafx3/rust-7988707.log`, first nextest run, 7,417 PASS records; SHA-256 `981dff427c13053e51bca43ab6e807ad32da49b1f7a42be25d7b7b6516390fae`. Parse `PASS [ seconds s] (ordinal/7417) binary test`, selecting binary `farcooler-daemon` or its `::` children. There are 3,875 daemon passes, totaling **2,893.353 s**; top 60 total **601.336 s (20.78%)**. The entire workspace totals 3,133.370 s; nextest wall time was 533.608 s. Durations overlap, include contention and startup, and are not CPU time or additive wall savings. No build, gate or tests were run for this report.

## Slowest 60

Binary is the complete nextest ID; `daemon` abbreviates `farcooler-daemon`, and `daemon::X` abbreviates `farcooler-daemon::X`. Seconds are the log's recorded values.

| # | Seconds | Binary | Test |
|---:|---:|---|---|
| 1 | 30.637 | `daemon` | `workflow_runtime::coding_verbs_tests::enospc_during_cherry_pick_retries_the_fold_without_split_or_rework` |
| 2 | 25.809 | `daemon` | `workflow_runtime::coding_verbs_tests::prelaunch_gate_enospc_retries_boundedly_without_split_or_rework` |
| 3 | 20.040 | `daemon` | `workflow_runtime::process_logs::tests::workflow_get_with_a_long_history_carries_live_tails_only` |
| 4 | 19.183 | `daemon` | `workflow_runtime::coding_verbs_tests::a_ci_wait_waits_on_every_workflow_it_names_that_the_push_started` |
| 5 | 17.140 | `daemon` | `workflow_runtime::coding_remote_tests::a_download_that_outlasts_its_limit_fails_the_ticket_with_a_sentence` |
| 6 | 14.744 | `daemon` | `workflow_runtime::coding_verbs_tests::a_fold_conflict_isolates_all_bad_tickets_and_keeps_the_healthy_batch_together` |
| 7 | 14.551 | `daemon::workflow_service_over_socket` | `workflow_service_ordinary_socket_config_pins_rework_restart_and_offline_completion` |
| 8 | 14.462 | `daemon::agent_rows_over_every_transport` | `rows_page_follow_and_survive_a_restart_on_every_transport` |
| 9 | 13.466 | `daemon` | `workflow_runtime::coding_verbs_tests::next_group_gates_during_ci_and_pushes_over_a_red_one` |
| 10 | 13.061 | `daemon` | `workflow_runtime::coding_verbs_tests::tickets_that_apply_alone_but_conflict_together_return_the_conflicting_member` |
| 11 | 12.819 | `daemon` | `workflow_runtime::coding_setup_rerun_tests::nov16_runner_outage_reruns_2895s_ci_and_2924_pushes_on_its_green_without_a_second_gate` |
| 12 | 12.275 | `daemon` | `workflow_runtime::coding_setup_rerun_tests::a_real_test_failure_in_the_previous_ci_is_not_rerun_and_the_gated_group_goes_as_before` |
| 13 | 11.996 | `daemon` | `workflow_runtime::coding_native::tests::workflow_coding_native_real_pty_normalized_hooks_and_quiescence` |
| 14 | 11.389 | `daemon` | `workflow_runtime::gate_tests::a_review_runs_with_the_model_its_changed_paths_earn_and_the_workflows_words` |
| 15 | 11.213 | `daemon` | `workflow_runtime::coding_red_base_tests::nov16_group2924_pushes_over_the_red_ci_it_gated_on_and_sends_nobody_back` |
| 16 | 11.047 | `daemon::a_ticket_shows_its_pr_stage` | `an_update_that_leaves_the_state_alone_does_not_wake_the_watch` |
| 17 | 10.806 | `daemon` | `workflow_runtime::tests::workflow_runtime_interrupted_base_publication_recovers_before_admission` |
| 18 | 10.465 | `daemon` | `watch::answer_wake::tests::notice_wait_tests::a_runner_notice_left_by_a_long_turn_is_sent_when_it_ends` |
| 19 | 10.386 | `daemon` | `workflow_runtime::coding_verbs_tests::a_rejected_gate_splits_a_batch_and_both_halves_land` |
| 20 | 10.231 | `daemon` | `workflow_runtime::coding_gate_tests::nov14_gate_output_ejects_the_planted_breaker_and_lands_the_healthy_batch` |
| 21 | 9.959 | `daemon` | `workflow_runtime::coding_remote_tests::a_candidate_off_the_target_is_refused_before_the_push_even_when_the_shared_repository_is_grafted_to_say_otherwise` |
| 22 | 9.721 | `daemon` | `workflow_runtime::coding_landing::security_tests::workflow_coding_security_fifo_receipts_are_bounded_and_never_replayed` |
| 23 | 9.430 | `daemon::a_read_fills_pr_state_without_waiting_for_github` | `a_read_answers_from_memory_and_the_fill_arrives_as_an_event` |
| 24 | 9.277 | `daemon` | `workflow_runtime::coding_verbs_tests::a_wait_leaves_the_work_pool_alone` |
| 25 | 9.181 | `daemon::a_forced_command_still_streams` | `twenty_idle_panes_opened_and_closed_on_one_connection_all_open` |
| 26 | 8.832 | `daemon` | `watch::answer_wake::tests::compose_held_tests::compose_pending_tests::a_message_sent_during_a_long_turn_with_a_dialog_is_sent_when_it_ends` |
| 27 | 8.799 | `daemon` | `workflow_runtime::coding_tests::workflow_coding_real_git_commands_rework_multiple_checks_and_repeated_base` |
| 28 | 8.668 | `daemon` | `workflow_runtime::landing_templates_tests::a_per_ticket_push_template_lands_each_ticket_on_its_own` |
| 29 | 8.334 | `daemon` | `workflow_runtime::coding_verbs_tests::a_fold_tree_deleted_mid_gate_retries_the_gate_without_split` |
| 30 | 8.316 | `daemon::a_cutover_survives_every_crash` | `a_current_runner_survives_a_crash_at::release_in_transaction` |
| 31 | 8.138 | `daemon::a_ticket_shows_its_pr_stage` | `the_watch_reads_nothing_until_a_ticket_enters_review_and_a_ticket_kicks_once_in_fifteen_seconds` |
| 32 | 8.091 | `daemon` | `workflow_runtime::gate_tests::an_agent_that_never_gives_a_verdict_fails_the_run_and_one_that_rejects_stops_it` |
| 33 | 7.955 | `daemon` | `workflow_runtime::publication_bounds_tests::ov1201_orchestrator_cannot_unguard_an_owner_published_command` |
| 34 | 7.704 | `daemon` | `workflow_runtime::landing_templates_tests::published_unlimited_batch_merges_old_waiters_on_restart_and_lands_once` |
| 35 | 7.618 | `daemon` | `watch::answer_wake::tests::compose_held_tests::compose_pending_tests::compose_parked_tests::a_dialog_up_at_the_turns_end_gets_no_enter_until_it_goes` |
| 36 | 7.573 | `daemon` | `workflow_runtime::landing_templates_tests::staggered_ready_tickets_join_one_waiter_while_an_active_landing_keeps_its_pin` |
| 37 | 7.491 | `daemon::a_cutover_survives_every_crash` | `a_raw_schema60_runner_survives_a_crash_at::release_in_transaction` |
| 38 | 7.411 | `daemon` | `workflow_runtime::coding_verbs_tests::a_head_with_no_ci_run_ends_the_base_step_after_the_grace_with_a_sentence` |
| 39 | 7.367 | `daemon` | `workflow_runtime::tests::workflow_runtime_approval_policies_and_authentication` |
| 40 | 7.358 | `daemon` | `watch::answer_wake::tests::compose_held_tests::compose_pending_tests::compose_parked_tests::two_bad_reads_while_parked_do_not_end_the_wait` |
| 41 | 7.334 | `daemon` | `workflow_runtime::coding_ci_rerun_tests::ci_rerun_red_then_green_lands_and_red_twice_routes_rejected` |
| 42 | 7.233 | `daemon::a_cutover_survives_every_crash` | `a_current_runner_survives_a_crash_at::commit_in_mapping` |
| 43 | 7.232 | `daemon::a_cutover_survives_every_crash` | `a_current_runner_survives_a_crash_at::finish_before_rename` |
| 44 | 7.143 | `daemon::a_cutover_survives_every_crash` | `a_current_runner_survives_a_crash_at::verify_after` |
| 45 | 7.137 | `daemon` | `workflow_runtime::coding_verbs_tests::a_base_that_moves_under_the_gate_is_retried_from_the_new_base_and_wedges_nothing` |
| 46 | 7.094 | `daemon::a_cutover_survives_every_crash` | `a_raw_schema60_runner_survives_a_crash_at::finish_before_rename` |
| 47 | 6.966 | `daemon` | `workflow_runtime::tests::workflow_runtime_generated_crash_receipt_and_replay_properties` |
| 48 | 6.926 | `daemon::a_cutover_survives_every_crash` | `a_current_runner_survives_a_crash_at::commit_after_permit` |
| 49 | 6.922 | `daemon::a_cutover_survives_every_crash` | `a_raw_schema60_runner_survives_a_crash_at::recover_after_rename` |
| 50 | 6.898 | `daemon` | `claude_sign_in::tests::a_late_finish_ends_an_expired_sign_in` |
| 51 | 6.897 | `daemon::a_cutover_survives_every_crash` | `a_current_runner_survives_a_crash_at::finish_after_rename` |
| 52 | 6.888 | `daemon::a_cutover_survives_every_crash` | `a_current_runner_survives_a_crash_at::recover_after_rename` |
| 53 | 6.868 | `daemon::a_cutover_survives_every_crash` | `a_current_runner_survives_a_crash_at::journal_torn` |
| 54 | 6.736 | `daemon::a_cutover_survives_every_crash` | `a_current_runner_survives_a_crash_at::begin_after_baseline_db` |
| 55 | 6.720 | `daemon::a_cutover_survives_every_crash` | `a_current_runner_survives_a_crash_at::begin_after_intent` |
| 56 | 6.712 | `daemon` | `pr_review::tests::a_read_that_goes_stale_announces_the_drop_once` |
| 57 | 6.686 | `daemon::a_cutover_survives_every_crash` | `a_current_runner_survives_a_crash_at::commit_after_snapshot` |
| 58 | 6.685 | `daemon::a_cutover_survives_every_crash` | `a_current_runner_survives_a_crash_at::commit_after_copy` |
| 59 | 6.664 | `daemon` | `workflow_runtime::coding_verbs_tests::a_batch_on_a_base_whose_ci_failed_folds_gates_and_lands_and_ci_after_the_push_decides` |
| 60 | 6.652 | `daemon::a_cutover_survives_every_crash` | `a_current_runner_survives_a_crash_at::begin_before_intent` |

## Slowest binaries

Summed PASS durations, not binary wall times. The main library binary alone holds 3,320 tests.

| Binary | Tests | Seconds |
|---|---:|---:|
| `daemon` | 3320 | 2206.415 |
| `daemon::a_cutover_survives_every_crash` | 72 | 405.776 |
| `daemon::workflow_service_over_socket` | 18 | 50.666 |
| `daemon::rpc_over_socket` | 87 | 25.547 |
| `daemon::a_ticket_shows_its_pr_stage` | 6 | 23.829 |
| `daemon::agent_rows_over_every_transport` | 5 | 17.201 |
| `daemon::a_forced_command_still_streams` | 5 | 16.684 |
| `daemon::a_read_fills_pr_state_without_waiting_for_github` | 2 | 15.828 |
| `daemon::git_runs_nothing_a_repository_plants` | 10 | 9.841 |
| `daemon::workflow_draft_approval` | 9 | 9.767 |
| `daemon::needs_you_changed_is_announced` | 7 | 9.191 |
| `daemon::lfs_content_under_the_git_sandbox` | 5 | 7.021 |
| `daemon::the_daemon_log_holds_stderr` | 5 | 6.689 |
| `daemon::an_orchestrator_starts_from_its_home` | 13 | 6.365 |
| `daemon::a_change_set_tells_the_truth_about_a_worktree` | 33 | 6.140 |

## Group constraints

Apply the default overrides inherited by gate/CI in `.config/nextest.toml:45` and `.config/nextest.toml:78`. These are sums of recorded durations, not actual blocked or elapsed time; the capped groups can overlap unrelated tests.

| Group or override | Concurrency | Daemon tests | Summed seconds |
|---|---:|---:|---:|
| `four-tmux-servers` (`rpc_over_socket`) | 4 | 87 | 25.547 |
| `file-events` (watch, registry, projector filters) | 2 | 58 | 49.437 |
| Unnamed exclusive override (`a_resized_pane_says_so` or `workflow_native_review_vanished_server_keeps_output_and_recorded_verdict`) | all test threads | 4 | 4.651 |

No named serial group applies to daemon tests. `one-slow-tmux` is serial (max 1), but belongs to `farcooler-tmux`: 19 matching passes, 6.381 s in this log. The exclusive daemon tests must run alone, so their 4.651 s is a direct scheduling floor; the other sums are not equivalent to the time spent blocking a worker. Filters and exclusive override are at `.config/nextest.toml:78`, `:92`, `:107`, and `:119`.

## Top 20: source and proposed fix

Numbers refer to the ranking above. Paths below are relative to `crates/daemon/`. A timeout bound is not assumed to have expired merely because it appears in source. For process-heavy tests the cited setup is a plausible cost driver, not a measured attribution.

| # | Wait or expensive setup, file and line | Proposed fix |
|---:|---|---|
| 1 | `src/workflow_runtime/coding_verbs_tests.rs:1189`: two cases each create a lander, three items and real Git wrapper; retries run full folds. | Reuse an immutable seeded repository; keep one end-to-end variant and model the other fault closer to the fold boundary. |
| 2 | `src/workflow_runtime/coding_verbs_tests.rs:1256`: two full landers and Git wrapper, with repeated `pump` at line 1282. | Seed once and inject prelaunch faults at the boundary; preserve both retry outcomes. |
| 3 | `src/workflow_runtime/process_logs.rs:214`: 420 entries; `:162` submits each and writes receipts, then archives 400. | Bulk-seed a valid persisted history or cache a template; preserve the frame-size assertion. |
| 4 | `src/workflow_runtime/coding_verbs_tests.rs:862`: five separate real landers and CI subprocess scenarios, including two timeouts. | Share seeded Git fixture and inject CI status/clock; retain timeout semantics. |
| 5 | `src/workflow_runtime/coding_remote_tests.rs:565` invokes `slow_pack(30)`; helper at `:516` sleeps in a real pack hook, with two download attempts. | Inject a controllable download deadline or signal a blocked fake transport; keep one real cancellation smoke test. |
| 6 | `src/workflow_runtime/coding_verbs_tests.rs:1055`: six items, real clone and conflict commits at `:1068`, then rebuild/fold. | Seed a reusable conflict repository; move combinatorial diagnosis to a focused test. |
| 7 | `tests/workflow_service_over_socket.rs:134`: two configured service runs, close/reopen at `:165`, polling at `:168`. | Share seeded configuration and signal stage changes; keep a socket restart case. |
| 8 | `tests/agent_rows_over_every_transport.rs:240`: socket and stdio contracts before and after restart (`:270`); contract sleeps 300 ms at `:208`. | Signal follower readiness before writing and share fixture; retain both transports and restart. |
| 9 | `src/workflow_runtime/coding_verbs_tests.rs:907`: two full landing scenarios, with a deliberate 300 ms barrier at `:935` and 10 ms polling at `:951`. | Replace barrier sleep with an explicit push-attempt signal, reuse seed. |
| 10 | `src/workflow_runtime/coding_verbs_tests.rs:1115`: five candidates, real Git commits and multiple rebuilds/folds. | Reuse seeded conflict graph; unit-test diagnosis separately. |
| 11 | `src/workflow_runtime/coding_setup_rerun_tests.rs:129`: full replay and land; helper creates all previous/gated items and polls at `:89–111`. | Share a seeded replay and signal the CI barrier; retain outage regression. |
| 12 | `src/workflow_runtime/coding_setup_rerun_tests.rs:156`: same expensive replay/land as #11. | Share immutable seed with #11; test CI classification independently. |
| 13 | `src/workflow_runtime/coding_native.rs:1189`: four native-agent fixtures run through real PTY and workflow. | Keep one PTY smoke in main; run full agent matrix nightly or use a lower-level hook fixture for variants. |
| 14 | `src/workflow_runtime/gate_tests.rs:242`: two `three_gates` fixtures plus a third at `:251`, each drives real workflow. | Reuse seeded fixture and test model routing without replaying every stage. |
| 15 | `src/workflow_runtime/coding_red_base_tests.rs:47`: multi-item real landing and observer polling at `:74`. | Share seed with rerun scenarios and signal CI transition. |
| 16 | `tests/a_ticket_shows_its_pr_stage.rs:273`: two 3 s waits and another 3 s wait at `:280`, after real daemon setup. | Inject watch clock and await explicit read/processing acknowledgements; verify no wake after advancing clock. |
| 17 | `src/workflow_runtime/tests.rs:491`: three full fixtures and recovery/reopen cycles. | Seed a reusable pre-publication state, copy it per corruption variant. |
| 18 | `src/watch/answer_wake/notice_wait_tests.rs:64`: 2 s + 1 s + 6 s pumping, then 200 ms polling at `:81`. | Advance an injected clock and signal watcher progress while preserving quiet-period assertion. |
| 19 | `src/workflow_runtime/coding_verbs_tests.rs:544`: real lander and three queued items; rejection runs four gates. | Reuse seed and exercise split logic with an injected gate result; retain one end-to-end case. |
| 20 | `src/workflow_runtime/coding_gate_tests.rs:16`: four joined items and full gate/rework landing, waiting for each at `:33`. | Share seeded repository, inject deterministic gate result; preserve planted-breaker integration smoke. |

## Ranked opportunities and limits

1. **Nightly crash matrix:** `daemon::a_cutover_survives_every_crash` has 72 passes totaling 405.776 s (14.02% of daemon summed time). Moving all 72 saves that much from the main sum; keeping smoke cases reduces it by their recorded durations. These crash boundaries are valuable per-landing coverage, so choose representative current/raw-schema cases and keep the full matrix in nightly. This is a tier recommendation, not an implemented change.
2. **Controlled time / signals:** #5 and #16 total 28.187 s; #16 alone contains 9 s of explicit sleeps, while #5 blocks real downloads with a sleeping pack hook. About 15 s is an illustrative combined target, below the total ceiling; actual cancellation and daemon work remain. #18 also has 9 s of intentional pumping (10.465 s total), a further candidate. Inject the clock at the behavior boundary and acknowledge the relevant event, preserving negative assertions and at least one real integration test.
3. **Bulk history fixture:** #3 totals 20.040 s; estimating 10 s saved assumes about half is removable repeated setup. Measure before/after; preserve the 420-item, 400-archived payload and on-disk receipt behavior. A shared immutable seed copied into isolated temporary directories could also reduce repeated Git setup in #1, #2, #4 and #6–#12, but the log cannot isolate setup cost, so no additive saving is claimed.

The estimates use one loaded run, not a controlled benchmark. Nightly movement removes coverage from main, whereas fixture/clock changes need equivalent assertions and isolated state. Scheduling and contention mean none of these summed seconds can be converted reliably to landing-gate or CI wall seconds without a paired run.
