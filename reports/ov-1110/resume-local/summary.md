# Resumed local measurements

Captured 13 November 2026 (Singapore). These are observations of the existing source,
including its failures; they do not establish a passing gate.

| Command | Outer wall seconds | Exit | Under-20 controlled |
| --- | ---: | ---: | --- |
| cold | 146.62 | 0 | True |
| warm | 2.00 | 0 | True |
| nextest | 811.51 | 100 | True |
| landing-gate | 464.77 | 130 | False |

The outer timer polls every two seconds, so successful-command wall times
include up to two seconds of observation latency. Cargo reports 0.31 s for
the warm build. Nextest reports 1m47s compilation and 700.841 s execution:
6,916 passed, 12 failed, 33 skipped. The wrappers are included in outer wall time.
All 6,928 executed test results are present; repeated failure-summary rows are deduplicated by test index.

## Landing gate: incomplete due to load

Stopped at 2026-11-12T17:20:55.018492+00:00, load **21.37 / 11.70 / 8.65** (1/5/15 minutes).
Only the capture-created process group was interrupted. No full-gate latency or passing receipt was obtained.

Completed-step seconds below are rounded by the gate, exclude orchestration, and are not total gate latency.

| Job | Recorded completed-step seconds | Coverage |
| --- | ---: | --- |
| wire | 162 | completed |
| doc-comments | 0 | completed |
| relay | 18 | completed |
| tunnel | 26 | completed |
| rust | 255 | partial; interrupted in cross-target cargo check |
| swift-tests | unmeasured | not reached |
| swift | unmeasured | not reached |
| ios | unmeasured | not reached |
| android | unmeasured | not reached |
| web | unmeasured | not reached |

## Fresh cold build: longest compilation units

Unit times overlap; frontend/codegen/link interpretation remains in the main investigation.

| Unit | Seconds |
| --- | ---: |
| aws-lc-sys 0.43.0 build-script (run) | 47.56 |
| farcooler-daemon 0.1.0 | 16.82 |
| syn 2.0.119 | 12.37 |
| syn 3.0.3 | 11.53 |
| serde_derive 1.0.229 | 9.46 |
| russh 0.62.4 | 9.45 |
| libsqlite3-sys 0.30.1 build-script (run) | 8.98 |
| rustls 0.23.43 | 8.88 |
| regex-automata 0.4.16 | 7.93 |
| zerofrom-derive 0.1.7 | 7.18 |

## Slowest 20 local Rust tests

| Test | Seconds | Result |
| --- | ---: | --- |
| `farcooler-client::against_a_real_daemon a_permission_answered_at_the_keyboard_releases_the_held_hook_and_the_phone` | 21.580 | FAIL |
| `farcooler-client::against_a_real_daemon a_permission_denied_from_the_mac_says_so_to_the_model` | 21.561 | FAIL |
| `farcooler-client::against_a_real_daemon a_held_ask_reaches_needs_you_as_the_shared_fixture_spells_it` | 21.523 | FAIL |
| `farcooler-client::against_a_real_daemon a_permission_answered_from_a_phone_reaches_the_held_hook` | 21.521 | FAIL |
| `farcooler-daemon workflow_runtime::coding_remote_tests::a_download_that_outlasts_its_limit_fails_the_ticket_with_a_sentence` | 16.734 | PASS |
| `farcooler-daemon::agent_rows_over_every_transport rows_page_follow_and_survive_a_restart_on_every_transport` | 14.384 | PASS |
| `farcooler-daemon::a_ticket_shows_its_pr_stage an_update_that_leaves_the_state_alone_does_not_wake_the_watch` | 11.036 | PASS |
| `farcooler-daemon watch::answer_wake::tests::notice_wait_tests::a_runner_notice_left_by_a_long_turn_is_sent_when_it_ends` | 10.547 | PASS |
| `farcooler-cli::a_hook_names_its_panes_launch a_launch_that_is_not_a_uuid_is_dropped` | 10.011 | FAIL |
| `farcooler-cli::a_hook_names_its_panes_launch a_hook_names_the_launch_its_pane_carries` | 10.010 | FAIL |
| `farcooler-cli::a_hook_names_its_panes_launch a_hook_outside_a_launched_pane_names_none` | 10.009 | FAIL |
| `farcooler-daemon::a_read_fills_pr_state_without_waiting_for_github a_read_answers_from_memory_and_the_fill_arrives_as_an_event` | 9.438 | PASS |
| `farcooler-daemon::a_forced_command_still_streams twenty_idle_panes_opened_and_closed_on_one_connection_all_open` | 8.995 | PASS |
| `farcooler-daemon watch::answer_wake::tests::compose_held_tests::compose_pending_tests::a_message_sent_during_a_long_turn_with_a_dialog_is_sent_when_it_ends` | 8.834 | PASS |
| `farcooler-daemon::a_ticket_shows_its_pr_stage the_watch_reads_nothing_until_a_ticket_enters_review_and_a_ticket_kicks_once_in_fifteen_seconds` | 8.123 | PASS |
| `farcooler-daemon watch::answer_wake::tests::compose_held_tests::compose_pending_tests::compose_parked_tests::a_dialog_up_at_the_turns_end_gets_no_enter_until_it_goes` | 7.612 | PASS |
| `farcooler-daemon watch::answer_wake::tests::compose_held_tests::compose_pending_tests::compose_parked_tests::two_bad_reads_while_parked_do_not_end_the_wait` | 7.386 | PASS |
| `farcooler-daemon::a_cutover_survives_every_crash a_raw_schema60_runner_survives_a_crash_at::release_in_transaction` | 7.364 | PASS |
| `farcooler-daemon workflow_runtime::coding_native::tests::workflow_coding_native_real_pty_normalized_hooks_and_quiescence` | 6.971 | PASS |
| `farcooler-daemon::workflow_service_over_socket workflow_service_ordinary_socket_config_pins_rework_restart_and_offline_completion` | 6.917 | PASS |

## Failed tests

- `farcooler-cli::a_hook_is_never_in_the_way a_big_verdict_is_not_lost_to_the_exit_that_follows_it` (1.105 s)
- `farcooler-cli::a_hook_is_never_in_the_way a_gating_hook_hands_the_verdict_to_the_agent_on_stdout` (1.105 s)
- `farcooler-cli::a_hook_is_never_in_the_way a_verdict_that_misses_the_deadline_leaves_the_agent_to_ask` (0.016 s)
- `farcooler-cli::a_hook_is_never_in_the_way a_wider_deadline_waits_for_the_same_late_verdict` (0.011 s)
- `farcooler-cli::a_hook_names_its_panes_launch a_hook_names_the_launch_its_pane_carries` (10.010 s)
- `farcooler-cli::a_hook_names_its_panes_launch a_hook_outside_a_launched_pane_names_none` (10.009 s)
- `farcooler-cli::a_hook_names_its_panes_launch a_launch_that_is_not_a_uuid_is_dropped` (10.011 s)
- `farcooler-cli::a_native_hook_follows_its_private_socket a_hook_without_that_variable_reports_where_its_file_says` (3.011 s)
- `farcooler-client::against_a_real_daemon a_held_ask_reaches_needs_you_as_the_shared_fixture_spells_it` (21.523 s)
- `farcooler-client::against_a_real_daemon a_permission_answered_at_the_keyboard_releases_the_held_hook_and_the_phone` (21.580 s)
- `farcooler-client::against_a_real_daemon a_permission_answered_from_a_phone_reaches_the_held_hook` (21.521 s)
- `farcooler-client::against_a_real_daemon a_permission_denied_from_the_mac_says_so_to_the_model` (21.561 s)
