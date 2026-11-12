| CI job | Runs | Median seconds | Min–max seconds |
| --- | ---: | ---: | ---: |
| Android | 5 | 1321.0 | 1310–1438 |
| Changes | 5 | 17.0 | 15–18 |
| Relay | 5 | 61.0 | 55–64 |
| Rust (macos-latest) | 4 | 2514.5 | 2293–2905 |
| Rust (ubuntu-24.04) | 4 | 1310.5 | 1079–1383 |
| Site | 5 | 24.0 | 23–35 |
| Swift (AgentKit + Mac tests) | 4 | 1443.0 | 1287–1487 |
| Swift (app build + bundle checks) | 4 | 1127.5 | 910–1354 |
| Tunnel (Go) | 5 | 106.0 | 85–107 |
| Web | 4 | 719.0 | 696–1005 |
| Wire compatibility | 5 | 173.0 | 167–174 |
| iOS | 4 | 426.0 | 386–466 |
| iOS UI (smoke) | 3 | 541.0 | 522–634 |
| iOS UI plan | 5 | 18.0 | 16–20 |

| Historical local job | Sum of step seconds |
| --- | ---: |
| wire | 193 |
| doc-comments | 0 |
| relay | 22 |
| rust | 911 |
| swift-tests | 722 |
| swift | 341 |
| android | 564 |
| web | 53 |
| Total executed steps | 2806 |

| Cold compilation unit | Seconds | Frontend / codegen seconds, when exposed |
| --- | ---: | --- |
| aws-lc-sys 0.43.0 build-script (run) | 46.59 | not separately exposed |
| farcooler-daemon 0.1.0 | 16.27 | frontend: 9.06; codegen: 7.21 |
| syn 2.0.119 | 14.21 | frontend: 1.41; codegen: 12.80 |
| syn 3.0.3 | 13.40 | frontend: 1.01; codegen: 12.39 |
| russh 0.62.4 | 10.82 | frontend: 2.41; codegen: 8.41 |
| serde_derive 1.0.229 | 9.16 | not separately exposed |
| libsqlite3-sys 0.30.1 build-script (run) | 8.94 | not separately exposed |
| rustls 0.23.43 | 8.84 | frontend: 1.18; codegen: 7.66 |
| tokio 1.53.1 | 8.38 | frontend: 2.47; codegen: 5.91 |
| clap_derive 4.6.4 | 8.34 | not separately exposed |
| regex-automata 0.4.16 | 8.32 | frontend: 1.18; codegen: 7.14 |
| tracing-attributes 0.1.31 | 8.01 | not separately exposed |
| prost-derive 0.13.5 | 7.78 | not separately exposed |
| thiserror-impl 2.0.19 | 7.70 | not separately exposed |
| zerofrom-derive 0.1.7 | 7.08 | not separately exposed |

### Slowest 20 observed individual tests (Swift and Rust)

| Job | Seconds | Test |
| --- | ---: | --- |
| Swift (AgentKit + Mac tests) | 102.906 | "The card waits for the hover delay, and moving off first cancels the wait" |
| Swift (AgentKit + Mac tests) | 101.273 | "In a tiled pane the message box keeps its bottom margin at every window height" |
| Swift (AgentKit + Mac tests) | 92.259 | "Scrolled up a long session, the knob moves only with the scroll where the stack resizes" with 2 test cases |
| Swift (AgentKit + Mac tests) | 89.871 | "A hover or pressed plate holds the icon and the count with even room, on every button" |
| Swift (AgentKit + Mac tests) | 86.400 | "Unread's header button asks before it reads" |
| Swift (AgentKit + Mac tests) | 82.549 | "One line, empty or typed, is centered in the box with the paperclip, within 1 pt; two lines leave equal room above and below (ov-470)" |
| Swift (AgentKit + Mac tests) | 81.495 | "The switcher is as wide as its label, whenever the label arrives" with 2 test cases |
| Swift (AgentKit + Mac tests) | 81.126 | "Opening and closing the field leaves the status item's width alone" with 3 test cases |
| Swift (AgentKit + Mac tests) | 80.494 | "Dragging a rule resizes its pane, and the window keeps what's drawn" |
| Swift (AgentKit + Mac tests) | 79.866 | "The text leads: a scheduled task says so, a run of calls is one line, a task list is a checklist" |
| Swift (AgentKit + Mac tests) | 79.480 | "At every narrow width the buttons keep the size they have in a wide pane, and the decision wraps instead" |
| Swift (AgentKit + Mac tests) | 76.376 | "A card lights its ticket, and a ticket lights its card; nothing else is lit" |
| Swift (AgentKit + Mac tests) | 76.370 | "A card's line shows its pull request's stage, and shows none for a building card or a runner that said nothing" |
| Swift (AgentKit + Mac tests) | 73.236 | "The window's request gives the tree the keyboard; ↓ chooses rows, and ↑ stops at Needs You" |
| Swift (AgentKit + Mac tests) | 73.181 | "Arrowing from Tasks into Terminals crosses the panes and keeps the row in sight" |
| Swift (AgentKit + Mac tests) | 71.721 | "A report, a task notice and a reminder each have their own row, and the reminder opens" |
| Swift (AgentKit + Mac tests) | 71.642 | "The toggle beside the filter collapses the whole tree, then expands all of it" |
| Swift (AgentKit + Mac tests) | 71.544 | "A theme opened only to show the selection closes again when the selection moves to another theme" |
| Swift (AgentKit + Mac tests) | 70.344 | "Switching views keeps the draft and never makes the terminal anew" |
| Swift (AgentKit + Mac tests) | 69.886 | "Every destination is paper inside the gutter and the plane outside it" with 2 test cases |

### Slowest 20 Rust test observations

| Job | Seconds | Test |
| --- | ---: | --- |
| Rust (macos-latest) | 18.134 | farcooler-daemon workflow_runtime::coding_remote_tests::a_download_that_outlasts_its_limit_fails_the_ticket_with_a_sentence |
| Rust (ubuntu-24.04) | 16.436 | farcooler-daemon workflow_runtime::coding_remote_tests::a_download_that_outlasts_its_limit_fails_the_ticket_with_a_sentence |
| Rust (macos-latest) | 15.474 | farcooler-daemon workflow_runtime::coding_remote_tests::a_candidate_off_the_target_is_refused_before_the_push_even_when_the_shared_repository_is_grafted_to_say_otherwise |
| Rust (macos-latest) | 15.210 | farcooler-daemon::agent_rows_over_every_transport rows_page_follow_and_survive_a_restart_on_every_transport |
| Rust (macos-latest) | 14.067 | farcooler-daemon::workflow_service_over_socket workflow_service_ordinary_socket_config_pins_rework_restart_and_offline_completion |
| Rust (ubuntu-24.04) | 13.804 | farcooler-daemon::agent_rows_over_every_transport rows_page_follow_and_survive_a_restart_on_every_transport |
| Rust (macos-latest) | 12.419 | farcooler-tmux::live_tmux a_server_born_during_a_read_is_never_skipped_from_round_2 |
| Rust (macos-latest) | 12.259 | farcooler-daemon workflow_runtime::coding_landing::security_tests::workflow_coding_security_fifo_receipts_are_bounded_and_never_replayed |
| Rust (macos-latest) | 12.022 | farcooler-daemon workflow_runtime::gate_tests::an_agent_that_never_gives_a_verdict_fails_the_run_and_one_that_rejects_stops_it |
| Rust (macos-latest) | 11.869 | farcooler-tmux::live_tmux a_server_born_during_a_read_is_never_skipped_from_round_1 |
| Rust (macos-latest) | 11.668 | farcooler-daemon watch::answer_wake::tests::notice_wait_tests::a_runner_notice_left_by_a_long_turn_is_sent_when_it_ends |
| Rust (macos-latest) | 11.603 | farcooler-daemon::a_ticket_shows_its_pr_stage an_update_that_leaves_the_state_alone_does_not_wake_the_watch |
| Rust (macos-latest) | 11.514 | farcooler-daemon workflow_runtime::coding_native::tests::workflow_coding_native_real_pty_normalized_hooks_and_quiescence |
| Rust (ubuntu-24.04) | 11.023 | farcooler-daemon::a_ticket_shows_its_pr_stage an_update_that_leaves_the_state_alone_does_not_wake_the_watch |
| Rust (macos-latest) | 10.886 | farcooler-daemon::a_cutover_survives_every_crash a_raw_schema60_runner_survives_a_crash_at::release_in_transaction |
| Rust (macos-latest) | 10.768 | farcooler-tmux::live_tmux a_server_born_during_a_read_is_never_skipped_from_round_0 |
| Rust (macos-latest) | 10.645 | farcooler-daemon fs_watch::exclusion_tests::tracked_edits_and_new_untracked_source_still_surface |
| Rust (macos-latest) | 10.482 | farcooler-daemon workflow_runtime::landing_templates_tests::a_per_ticket_push_template_lands_each_ticket_on_its_own |
| Rust (macos-latest) | 10.381 | farcooler-daemon::a_read_fills_pr_state_without_waiting_for_github a_read_answers_from_memory_and_the_fill_arrives_as_an_event |
| Rust (ubuntu-24.04) | 10.373 | farcooler-daemon watch::answer_wake::tests::notice_wait_tests::a_runner_notice_left_by_a_long_turn_is_sent_when_it_ends |
