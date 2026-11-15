# ov-1284: Usage lines come from Rust

The four ov-1048 commits (4b3ab0c24) were cherry-picked in order onto main 19bf27fd9.
The third was already on main in effect (an empty summary commit), so it was kept
with `--allow-empty` because it carries the net-lines count. No conflicts: ov-1054's
`plan_view.rs` additions had already been merged by the ov-1048 fix round.

## Generated files

`scripts/gen-wire-types.sh --check` is clean, and regenerating changes nothing, so
the Swift, Kotlin and TS WireTypes match Rust on today's main.

## Hand-written lines (non-test, non-generated, against origin/main)

About +180 / -298, **net -118** (Swift, Kotlin and Rust together; the original card
said -127 on an older base). Generated: +94 lines of WireTypes. The web has no Usage
section; it only gets the generated types.

Lint: `shared-core-lint.py` ok, `--baseline-not-grown origin/main` ok. The baseline
does not grow (the commits shrink it; I left unrelated shrinks from other work out).

## Tests (all foreground, -j4)

- `cargo nextest run -j4 -p farcooler-client -p farcooler-core -p farcooler-cli -E 'test(usage) | test(the_bridge)'`: 41/41.
- `cargo nextest run -j4 -p farcooler-client` (full): all pass except 4 hook tests in
  `against_a_real_daemon` (`a_permission_*`, `a_held_ask_*`). They **fail identically on
  plain origin/main** (checked in a scratch worktree), so they are not from this change.
- `cargo clippy -j4 -p farcooler-client -p farcooler-core -p farcooler-cli --all-targets -- -D warnings`: clean.
- `cd apps/shared/AgentKit && swift test --filter TaskUsageTests`: 6/6 (needs `cargo build --release -p farcooler-client` first).
- `cd apps/android && ./gradlew --max-workers=4 testInstrumentedUnitTest --tests com.farcooler.model.TaskUsageTest --tests '*PlanCostTest'`: BUILD SUCCESSFUL.
- `scripts/file-size-budget.py`, `copy-lint.py`, `doc-comment-check.py`: ok.
- Not run: the Mac display captures and the iOS build (heavy, unlocked screen); ov-1048
  saw 36/36 Mac captures identical and iOS building before the rebase, and the Mac
  `TaskUsageView` and iOS `TaskUsageSection` changes are the same files here.

## A Rust test seen failing

Changing the row-title separator in `said()` (`usage_json.rs:106`, `" · "` to `" - "`)
turned `every_case_says_the_fixture_lines` and `the_shown_golden_is_current` red.
Reverted afterwards.

## Review nits (ov-1048 review.md)

1. Rebase conflict with ov-1054: already resolved in the fix round; nothing left.
2. en_US numbers in this section: deliberate, recorded on ov-1048 (a5e8c194).
3. iPhone harness quiet fallback: fixed (commit "stops loudly").
4. Three hand-copied sentences: left, as ov-1048 decided. `NEEDS_UPDATE` and
   `COULDNT_READ` have no answer to ride on; `API_EQUIVALENT` is still read by the
   plan cost views. The cut belongs with the plan cost move.
5. Tie ordering and a zero-token unpriced row: accepted; Rust matches Kotlin's old
   order and real data cannot produce the second case.
6. Gradle not tracking fixtures: pre-existing, CI builds fresh; not changed.
7. Long doc line in `crates/cli/src/task_usage.rs`: rewrapped.
