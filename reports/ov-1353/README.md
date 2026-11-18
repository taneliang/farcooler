# ov-1353 verification

Started at origin/main `2f0b5f026` and cherry-picked ov-1198 commit `d04c743d5` (cherry-pick `f9a450c9f`).

On macOS, with `TMPDIR=/private/tmp`, the following daemon-only checks passed:

- `cargo test -p farcooler-daemon --lib workflow_runtime::concurrency_tests` — 8 passed.
- `cargo test -p farcooler-daemon --lib workflow_runtime::places::config_tests` — 1 passed.
- `cargo test -p farcooler-daemon --lib workflow_runtime::execution::tests` — 5 passed.
- `cargo test -p farcooler-daemon --lib a_landing_group_folds_while_a_build_holds_every_agent_place` — 1 passed.
- `cargo test -p farcooler-daemon --lib a_reviewed_ticket_joins_automatically_and_a_landing_gate_leaves_build_capacity` — 1 passed.
- `cargo clippy -p farcooler-daemon --all-targets -- -D warnings` — passed.

The initial sandboxed build could not create clang temporary files; setting `TMPDIR` fixed that. The sandboxed concurrency run then failed during Git fixture setup with `git invocation failed`; the checks above passed outside the sandbox.
