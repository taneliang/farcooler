# ov-1203 — finished runs free build output

Reused the durable cleanup implementation from ov-1126 and ov-1136, adapted to
main's current scheduler. Cleanup is selected transactionally when a run enters
review, join or landing, or ends; it preserves output between Build and Check.
Groups inherit the published folder policy. Ended group source checkouts and
Git references remain; output is no longer parked for later groups.

The dispatch loop defers deletion while any effect uses the tree, including
short effects and stops. Repository claim fences, later-build checks, open-file
checks, tracked-file checks, symlink checks and Git metadata checks guard
removal. Reports are protected, including reports nested in build output.
Interrupted attempts are recorded before deletion and never replayed against
new output. `workflow get` exposes `build_cleanup`, with item/run identity,
bytes freed and skip reasons.

## Regression and acceptance evidence

With `release_builds` temporarily disabled at its call site, the real fixture
transition test `build_cleanup_build_to_review_releases_target_and_records_bytes_once`
failed at `assert!(!tree.join("target").exists())`. Restoring cleanup makes it
pass. No disabled cleanup or test-only bypass remains.

Tests cover WaitingJoin and Failed releases of target and both Gradle build
directories; cancellation and recovery; group output release; active-effect
deferral; resumed builds; stale cleanup versus newer builds; bytes in the Get
response; preservation of source, reports, commits, open output and shared trees.

Validation on 2026-11-14:

- `cargo test -p farcooler-workflow --no-fail-fast`: 160 passed.
- `TMPDIR=/tmp cargo test -p farcooler-store --no-fail-fast`: 655 passed, 3 ignored.
- `TMPDIR=/tmp cargo test -p farcooler-daemon --lib workflow_runtime:: --no-fail-fast`: 324 passed, 4 ignored.
- Final targeted cleanup rerun: 29 passed.
- `cargo clippy -p farcooler-daemon -p farcooler-store -p farcooler-workflow --all-targets -- -D warnings`: passed.
- File-size budget, doc-comment check, swallow lint and protocol lint: passed.
- `git diff --check`: passed. Never ran `cargo fmt`.

The initial sandboxed daemon tests could not invoke Git. The successful daemon
and store tests used the approved host execution path and temporary fixtures.

## Fix round 1 — retry temporary contention

The review found that a busy repository fence or open output file settled the
cleanup outbox permanently. Results now explicitly mark those transient blockers
as retryable. Each retry atomically claims the observed result and writes the
interrupted-attempt marker before any deletion, then repeats the active-tree,
later-build, ownership and filesystem safety checks. Interrupted attempts and
permanent safety refusals remain terminal. Partial successes retain their byte
total across retries without counting deleted folders again.

The revised fence regression failed on the previous implementation with
`ended runs must retry after the fence clears`. It now passes. The cancelled
group regression closes an open file and reopens the runtime, verifies eventual
cleanup, retains unlisted files, and checks accumulated bytes after partial
Gradle cleanup. The delayed-intent regression now starts with a retryable
open-file result and verifies active-tree deferral and later-build protection.

Fix-round validation on 2026-11-14:

- Cleanup regressions: 29 passed (fence test observed failing before the fix).
- Workflow suite: 160 passed; store suite: 655 passed, 3 ignored.
- Full workflow runtime suite: 324 passed, 4 ignored.
- Clippy for daemon, store and workflow, all targets with warnings denied: passed.
- File-size budget, doc-comment check, swallow lint, protocol lint and diff check: passed.
- Never ran `cargo fmt`.
