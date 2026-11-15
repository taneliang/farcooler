# ov-1254: engine train on current main

Base: `71e5fd1502ca7390c27294cd44f717f6283c73bf` (`origin/main`).
Source: `ov-1244-engine-train-2` at `512231f386661ab042d35058da12c01dfcbbfe48`.

Replayed the 127 `git cherry origin/main ov-1244-engine-train-2` plus entries,
in source order with `git cherry-pick -x`. All plus entries belong to the engine
stack and its integration corrections. The 94 minus entries are the already
landed CI prefix and were skipped. `source-commits.txt` records every source
commit and subject. No commit from the CI prefix was imported.

Tickets carried: ov-1213 (ov-1154, ov-1155, ov-1164, ov-1146, ov-1153,
ov-1147, ov-1142, ov-1139, ov-1145, ov-1152, ov-1159, ov-1203), ov-1231,
ov-1235, ov-1236, ov-1233, ov-1178, ov-1220, and ov-1238/ov-1244 integration
corrections. Their supporting engine ancestry includes ov-1143/ov-1138,
ov-1009, ov-1127/ov-1128, ov-1114/ov-1132, ov-1106/ov-1131, ov-1108,
ov-1105, ov-1158, ov-1217, ov-1170/ov-1214, ov-1179/ov-1201/ov-1206,
and ov-1171. The imported reports retain the original conflict ledgers.

## Reconciliation

All 127 picks applied without textual conflicts. Main's `review_agent.rs`,
`git.rs` and `git_guard.rs` remain byte-identical to the base, preserving the
`\\x00` literal and `git_guard::strip_inherited` and its call sites. `git.rs`
has 1,494 lines. The imported terminal-view extraction keeps `rpc.rs` at 2,394
lines, below its existing ceiling; no size-budget number was raised.

The base contained only the ov-1220 provision quarantine. Main's ov-1250 had
removed the ov-1248 git-sandbox entry originally added by ov-1249. Restored that
exact entry from `f977e3fc7` as explicitly required by ov-1254; both now remain.

## Validation

Commands run sequentially in the foreground, with inherited `FARCOOLER_*`
session variables removed from child environments, `CARGO_BUILD_JOBS=4`,
`CARGO_TARGET_DIR=/tmp/ov-1254-target`, `TMPDIR=/tmp`, and
`PATH=/opt/homebrew/bin:$HOME/.cargo/bin:/usr/bin:/bin:/usr/sbin:/sbin`.
No full lane gate or prohibited formatter/Git command was run.

Clippy initially rejected inherited warnings. Behavior-preserving integration
corrections remove an unused revision variable/assignment in the landed store
fixture, remove a redundant PlanCard default, use `then_some(async { ... })`
for two lazy future values, and narrowly allow the explicit yielding function's
argument count and the test-only workspace callback map's type complexity.
The futures still perform work only when polled; no engine behavior changed.
Initial failing logs are retained alongside the final passing Clippy log.

The first wire/doc-comments run completed with one environment failure:
`macos-rust-tests.py` requires stdlib `tomllib`, unavailable in the system
Python 3.9 selected by the required PATH. All other rows passed. For the rerun,
temporarily linked the installed `/opt/homebrew/bin/python3.14` as
`/opt/homebrew/bin/python3`, keeping the exact requested PATH. The launcher
removes its link on completion. No repository Python code was changed.

| Command | Result | Evidence |
| --- | --- | --- |
| `cargo clippy --workspace --all-targets -- -D warnings` | Passed, exit 0 | `clippy.log` |
| `./scripts/file-size-budget.py` | Passed: 2,138 files, 27 grandfathered | `file-size.log` |
| `CI=true ./scripts/lane-gate.py --base origin/main --only wire,doc-comments < /dev/null` | Every selected row passed, exit 0; 278.628 s | `wire-doc-comments.log` |

The lane gate did not select builds or other job families. This is the requested
partial gate, not a full lane-gate claim.

Nextest command (all workflow/store/CLI tests are selected; daemon/client/protocol
tests are filtered to the required and supporting integration scopes):

```sh
cargo nextest run \
  -p farcooler-workflow -p farcooler-store -p farcooler-cli \
  -p farcooler-daemon -p farcooler-client -p farcooler-protocol \
  --test-threads 4 --no-fail-fast \
  -E 'package(farcooler-workflow) | package(farcooler-store) | package(farcooler-cli) | (package(farcooler-daemon) & (test(workflow_runtime) | test(rpc) | test(hook_install) | test(git_sandbox) | test(effect_runner) | test(yielding) | test(answer_wake::engine) | test(engine_steering) | test(message_send))) | (package(farcooler-client) & (test(plan) | test(workflow) | test(every_route_has_an_arm))) | (package(farcooler-protocol) & test(workflow))'
```

Nextest completed with **2,280 passed, zero failed**, 3,632 outside the selection
(`nextest.log`). Compilation took 4m03s; the test run took 250.512s, and the
complete foreground command exited 0 after 495.416s. Both quarantined tests were
included in this direct run and passed; the checked-in quarantine remains as
required by this ticket.

Built CLI checks both exited 0:

```sh
/tmp/ov-1254-target/debug/farcooler workflow --help
/tmp/ov-1254-target/debug/farcooler workflow resolve --help
```

`workflow-help.log` retains the complete verb list, including `resolve`,
`resolve-cancellation`, `close`, `landed`, `rollback`, `pause`, and `resume`.
`resolve-help.log` retains ov-1220's arguments and recovery description.

Final history/preservation checks passed (`final-history.log`):

- `git merge-base --is-ancestor origin/main HEAD` succeeds; main remains the
  recorded base even after the gate's fetch.
- `git rev-list --merges --count origin/main..HEAD` prints **0**.
- `git cherry origin/main HEAD` contains only plus entries: no patch duplicates
  of main. All 127 imported source trailers match the manifest in order.
- Main's three specified source files are byte-identical; line counts are
  `git.rs` 1,494 and `rpc.rs` 2,394. Both required quarantine entries remain.
- `git diff --check` passes, and the temporary Python symlink is removed.

All four acceptance items are satisfied. No textual conflicts needed resolution
on this rebuild; the only additional reconciliation is the explicitly required
quarantine restoration and the behavior-preserving lint corrections documented
above. All foreground validation commands finished before the final report commit.
