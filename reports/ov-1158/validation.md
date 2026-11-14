# ov-1158: landing waits and infrastructure retries

Base: local `wave-a-preview`, `b954ea25f1b3040582b86b8fd3c1e55d495ef868`.
The required RCA is committed at `reports/ov-1157/landing-rca.md`.
No landing definitions, wire schema, or database schema changed.

## Behavior

- Split halves clear inherited wait timestamps. Completion and recovery observations use elapsed real time from the supplied clock origin, so a long preceding stage cannot spend the next stage's patience.
- Interrupted waits retain their original stage deadline and candidate. AwaitBase records the observed base SHA under the read's retained run identity, so resumption cannot silently switch to a newer main.
- Landing infrastructure failures get at most two retries of the same stage. Missing gate trees are restored on the owned branch at the pinned candidate; missing trees, ENOSPC, killed gates, and unfinished folds do not produce a code rejection or split.
- The CLI marks private workflow trees in text and JSON lists. Both the CLI and daemon refuse manual removal, including with confirmation; the daemon checks before dirtiness confirmation as well as before removal.

## Regression evidence

Each regression was observed failing before its fix or with the relevant fix disabled at its call site:

| Case | Failure observed | Required result |
| --- | --- | --- |
| Split half queued past patience | Inherited `Some(1000)` | Fresh timestamp when admitted after the queue |
| Repeated landing SafeFailure | Failed on the first infrastructure failure | Two retries, then Failed, with no split |
| Restart during CI after a slow push | Failed when Observe used the sweep timestamp | Done on the pushed SHA, within the original CI stage deadline |
| Fold tree deleted mid-gate | Four folds instead of the two original groups | Only the gate retries; a subsequent simulated ENOSPC spends the second retry, and both tickets finish |
| Real CLI removal against a scratch daemon | Engine tree was actually removed with protection disabled | Plain refusal, tree retained, ownership marked in both list formats |

The CLI integration test uses a short FARCOOLER_HOME and cleans up its own daemon by the PID holding its lock.

## Verification

Final acceptance run: **PASS**. The full five-crate test command, including doctests and the process/tmux leak wrapper, exited 0. Workspace clippy with warnings denied also exited 0. File-size and whitespace checks passed.

Implementation commits: `724bd168f` and `6a9d743f5`.

Commands, with inherited live `FARCOOLER_*` variables removed, `TMPDIR=/tmp`, `CARGO_BUILD_JOBS=4`, and the normal system tools on PATH:

```sh
python3 scripts/tmux-leak-check.py -- cargo test \
  -p farcooler-workflow -p farcooler-store -p farcooler-daemon \
  -p farcooler-cli -p farcooler-core --no-fail-fast -- --test-threads=4
cargo clippy --workspace --all-targets -- -D warnings
python3 scripts/file-size-budget.py
git diff --check
```

All builds use this worktree's own `target/`. Clearing the inherited workflow variables is necessary here: otherwise hook fixture subprocesses select the running task's private hook socket instead of their test sockets.
