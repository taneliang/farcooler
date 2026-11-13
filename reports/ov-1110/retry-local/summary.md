# Controlled baseline retry

Source: `1fe54243d8cd030693484c635aee2678715195d0`.

Outer wall times include up to two seconds of polling latency.
A load-interrupted command is excluded from controlled baseline comparisons.

| Command | Outer wall seconds | Exit | Controlled below 20 |
| --- | ---: | ---: | --- |
| cold | 182.44 | 0 | True |
| warm | 2.01 | 0 | True |
| nextest | 787.02 | -2 | False |

Stopped `nextest` at 2026-11-13T01:14:21.986256+00:00, load **20.98 / 11.87 / 9.43**.
Only the newly created capture process group was signaled.


cold: Finished `dev` profile [unoptimized] target(s) in 3m 00s

warm: Finished `dev` profile [unoptimized] target(s) in 0.45s

nextest: Finished `test` profile [unoptimized] target(s) in 2m 11s; Summary [ 654.028s] 5157/6928 tests run: 5142 passed, 15 failed, 33 skipped

## Cold build compilation units

Durations overlap; final compiler invocations include linking.

| Unit | Seconds |
| --- | ---: |
| aws-lc-sys 0.43.0 build-script (run) | 50.37 |
| farcooler-daemon 0.1.0 | 31.17 |
| russh 0.62.4 | 20.56 |
| clap_derive 4.6.4 | 14.84 |
| syn 2.0.119 | 14.13 |
| syn 3.0.3 | 13.34 |
| async-trait 0.1.91 | 13.27 |
| farcooler-client 0.1.0 | 12.56 |
| serde_derive 1.0.229 | 10.08 |
| rustls 0.23.43 | 9.97 |

## Warm build compilation units

Durations overlap; final compiler invocations include linking.

| Unit | Seconds |
| --- | ---: |
| adler2 2.0.1 | 0.00 |
| aead 0.6.1 | 0.00 |
| aes 0.9.2 | 0.00 |
| aes-gcm 0.11.0 | 0.00 |
| ahash 0.8.12 | 0.00 |
| ahash 0.8.12 build-script (run) | 0.00 |
| ahash 0.8.12 build-script | 0.00 |
| aho-corasick 1.1.4 | 0.00 |
| alacritty_terminal 0.26.0 | 0.00 |
| anstream 1.0.0 | 0.00 |

Observed 5157 of 6928 test results.

## Slowest 20 observed Rust tests

| Test | Seconds | Result |
| --- | ---: | --- |
| `farcooler-client::against_a_real_daemon a_permission_denied_from_the_mac_says_so_to_the_model` | 21.482 | FAIL |
| `farcooler-client::against_a_real_daemon a_permission_answered_at_the_keyboard_releases_the_held_hook_and_the_phone` | 21.469 | FAIL |
| `farcooler-client::against_a_real_daemon a_permission_answered_from_a_phone_reaches_the_held_hook` | 21.442 | FAIL |
| `farcooler-client::against_a_real_daemon a_held_ask_reaches_needs_you_as_the_shared_fixture_spells_it` | 21.429 | FAIL |
| `farcooler-daemon workflow_runtime::coding_remote_tests::a_download_that_outlasts_its_limit_fails_the_ticket_with_a_sentence` | 16.833 | PASS |
| `farcooler-daemon::a_cutover_survives_every_crash a_raw_schema60_runner_survives_a_crash_at::commit_after_permit` | 13.906 | PASS |
| `farcooler-daemon::a_cutover_survives_every_crash a_raw_schema60_runner_survives_a_crash_at::commit_after_restore` | 13.347 | PASS |
| `farcooler-daemon::a_cutover_survives_every_crash a_raw_schema60_runner_survives_a_crash_at::begin_in_upgrade` | 13.300 | PASS |
| `farcooler-daemon::a_cutover_survives_every_crash a_raw_schema60_runner_survives_a_crash_at::begin_torn_baseline` | 13.170 | PASS |
| `farcooler-daemon::a_cutover_survives_every_crash a_raw_schema60_runner_survives_a_crash_at::begin_torn_intent` | 13.094 | PASS |
| `farcooler-daemon::a_cutover_survives_every_crash a_raw_schema60_runner_survives_a_crash_at::commit_after_snapshot` | 12.809 | PASS |
| `farcooler-daemon::a_cutover_survives_every_crash a_raw_schema60_runner_survives_a_crash_at::commit_after_copy` | 12.782 | PASS |
| `farcooler-daemon::a_cutover_survives_every_crash a_raw_schema60_runner_survives_a_crash_at::commit_after_journal` | 11.380 | PASS |
| `farcooler-daemon::a_cutover_survives_every_crash a_raw_schema60_runner_survives_a_crash_at::commit_after_mapping` | 11.349 | PASS |
| `farcooler-daemon::a_cutover_survives_every_crash a_raw_schema60_runner_survives_a_crash_at::begin_before_intent` | 11.249 | PASS |
| `farcooler-daemon::a_cutover_survives_every_crash a_raw_schema60_runner_survives_a_crash_at::begin_before_baseline_db` | 10.670 | PASS |
| `farcooler-daemon watch::answer_wake::tests::notice_wait_tests::a_runner_notice_left_by_a_long_turn_is_sent_when_it_ends` | 10.583 | PASS |
| `farcooler-daemon::a_cutover_survives_every_crash a_current_runner_survives_a_crash_at::verify_after` | 10.577 | PASS |
| `farcooler-cli::a_hook_names_its_panes_launch a_hook_outside_a_launched_pane_names_none` | 10.016 | FAIL |
| `farcooler-cli::a_hook_names_its_panes_launch a_hook_names_the_launch_its_pane_carries` | 10.010 | FAIL |

## Failed or interrupted tests

- `farcooler-cli::a_hook_is_never_in_the_way a_big_verdict_is_not_lost_to_the_exit_that_follows_it` (0.045 s; FAIL)
- `farcooler-cli::a_hook_is_never_in_the_way a_gating_hook_hands_the_verdict_to_the_agent_on_stdout` (0.041 s; FAIL)
- `farcooler-cli::a_hook_is_never_in_the_way a_verdict_that_misses_the_deadline_leaves_the_agent_to_ask` (0.009 s; FAIL)
- `farcooler-cli::a_hook_is_never_in_the_way a_wider_deadline_waits_for_the_same_late_verdict` (0.010 s; FAIL)
- `farcooler-cli::a_hook_names_its_panes_launch a_hook_names_the_launch_its_pane_carries` (10.010 s; FAIL)
- `farcooler-cli::a_hook_names_its_panes_launch a_hook_outside_a_launched_pane_names_none` (10.016 s; FAIL)
- `farcooler-cli::a_hook_names_its_panes_launch a_launch_that_is_not_a_uuid_is_dropped` (10.009 s; FAIL)
- `farcooler-cli::a_native_hook_follows_its_private_socket a_hook_without_that_variable_reports_where_its_file_says` (3.009 s; FAIL)
- `farcooler-client::against_a_real_daemon a_held_ask_reaches_needs_you_as_the_shared_fixture_spells_it` (21.429 s; FAIL)
- `farcooler-client::against_a_real_daemon a_permission_answered_at_the_keyboard_releases_the_held_hook_and_the_phone` (21.469 s; FAIL)
- `farcooler-client::against_a_real_daemon a_permission_answered_from_a_phone_reaches_the_held_hook` (21.442 s; FAIL)
- `farcooler-client::against_a_real_daemon a_permission_denied_from_the_mac_says_so_to_the_model` (21.482 s; FAIL)
- `farcooler-daemon::a_cutover_survives_every_crash a_raw_schema60_runner_survives_a_crash_at::commit_before_copy` (7.522 s; SIGINT)
- `farcooler-daemon::a_cutover_survives_every_crash a_raw_schema60_runner_survives_a_crash_at::commit_in_mapping` (0.676 s; SIGINT)
- `farcooler-daemon::a_cutover_survives_every_crash a_raw_schema60_runner_survives_a_crash_at::commit_torn_snapshot` (0.554 s; SIGINT)
