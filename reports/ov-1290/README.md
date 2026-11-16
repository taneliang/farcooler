# ov-1290 — Daemon layer check counts edges per layer pair

Carries ov-1279 (candidate e4b88873d, `ov-1279-layer-limits`): the resolution rules, the fail-closed
constructs and the Known limits are unchanged. What changed is **what the check gates on**.

## The defect and the rule

ov-1279 recorded every violation by file, and the list could only shrink. Moving code between files
read as growth: ov-1287 moved existing tests out of `coding_native.rs` into
`coding_native_recovery_tests.rs` to meet the 1,500-line file budget, and the check refused the
same imports as new edges (`-> hook_install`, `-> service`). The file-size budget and the daemon
crate split ([ov-1219](../../.claude/agent/reports/ov-1219/README.md) section 5) both move code
between files, so the gate is now counts:

1. **Edges are counted per (source layer -> target layer) pair.** An edge is a (source module,
   target module) pair. A file counts under its top-level module, and a `_tests` sibling under the
   module it tests (so `coding_native_recovery_tests.rs`, which sits in `workflow_runtime/`, is
   `workflow_runtime`, and `people_actor_tests.rs` is `people`). Code and tests may be split across a
   module's files without changing a count.
2. **Unresolved constructs** (globs, aliases, calls of composition macros, macro-built paths) are
   counted per source layer, one per occurrence, as `layer -> unresolved`. Moving a glob between
   files of a layer passes; a new one fails, even in a file that already has one.
3. **A count may never rise** (a new edge, or a new construct, fails and the message names the
   edges or constructs of that pair), and **the recorded count must be lowered when edges are
   removed**: a stale higher count fails, naming `./scripts/daemon-layers.py --lower`, which only
   lowers. A higher count would let the next edge in for free.
4. **The per-file list is a generated report** (`--report-files`, saved as
   [per-file.txt](per-file.txt)), for the split's later steps. It is not read by the check.

The counts live in `scripts/daemon-layers-counts.txt` (replacing `daemon-layers-allow.txt`). In CI and
the lane gate's light `wire` job, `--counts-not-grown FETCH_HEAD` fails when a recorded count is above
main's, so a pull request cannot pass by writing a violation down.

## Counts per layer pair

Recomputed against current main (d724f8d4e): 409 edges and 239 unresolved constructs in 55 layer pairs.
ov-1279's per-file entry count was 924; the number is lower because edges are now per module, not
per file, which is the point. "Cleared by step" is the step of ov-1219's table that removes the
pair in production code; test files clear when the test moves (40–45) or stops needing the aggregate.

| From | To | Count | Cleared by step |
| --- | --- | ---: | --- |
| access | agents | 6 | 16–20 (agents) |
| access | board | 5 | 21–25 (board) |
| access | composition | 15 | 2 (contracts: wire/rpc types, verdicts, DTOs) |
| access | delivery | 2 | 32–36 (delivery) |
| access | engine | 1 | 8–10 (engine adapters), 7 (workflow runtime owner) |
| access | service | 34 | 3 (context: paths, proc_table, push, test_support, runtime) then the terminal slices 26–31 |
| access | unresolved | 32 | 40–45 (the test move) or write the path in full |
| access | watch | 4 | 37–39 (watch) |
| agents | access | 9 | 11–15 (access) |
| agents | board | 9 | 21–25 (board) |
| agents | composition | 9 | 2 (contracts: wire/rpc types, verdicts, DTOs) |
| agents | delivery | 1 | 32–36 (delivery) |
| agents | service | 22 | 3 (context: paths, proc_table, push, test_support, runtime) then the terminal slices 26–31 |
| agents | unresolved | 32 | 40–45 (the test move) or write the path in full |
| agents | watch | 4 | 37–39 (watch) |
| board | access | 16 | 11–15 (access) |
| board | agents | 12 | 16–20 (agents) |
| board | composition | 23 | 2 (contracts: wire/rpc types, verdicts, DTOs) |
| board | delivery | 2 | 32–36 (delivery) |
| board | engine | 3 | 8–10 (engine adapters), 7 (workflow runtime owner) |
| board | service | 57 | 3 (context: paths, proc_table, push, test_support, runtime) then the terminal slices 26–31 |
| board | unresolved | 39 | 40–45 (the test move) or write the path in full |
| board | watch | 17 | 37–39 (watch) |
| delivery | access | 4 | 11–15 (access) |
| delivery | board | 5 | 21–25 (board) |
| delivery | composition | 2 | 2 (contracts: wire/rpc types, verdicts, DTOs) |
| delivery | engine | 3 | 8–10 (engine adapters), 7 (workflow runtime owner) |
| delivery | service | 8 | 3 (context: paths, proc_table, push, test_support, runtime) then the terminal slices 26–31 |
| delivery | unresolved | 5 | 40–45 (the test move) or write the path in full |
| delivery | watch | 2 | 37–39 (watch) |
| engine | access | 11 | 11–15 (access) |
| engine | agents | 8 | 16–20 (agents) |
| engine | board | 8 | 21–25 (board) |
| engine | composition | 7 | 2 (contracts: wire/rpc types, verdicts, DTOs) |
| engine | delivery | 3 | 32–36 (delivery) |
| engine | service | 18 | 3 (context: paths, proc_table, push, test_support, runtime) then the terminal slices 26–31 |
| engine | unresolved | 62 | 40–45 (the test move) or write the path in full |
| engine | watch | 8 | 37–39 (watch) |
| git | access | 1 | 11–15 (access) |
| git | agents | 3 | 16–20 (agents) |
| git | board | 4 | 21–25 (board) |
| git | composition | 2 | 2 (contracts: wire/rpc types, verdicts, DTOs) |
| git | service | 10 | 3 (context: paths, proc_table, push, test_support, runtime) then the terminal slices 26–31 |
| git | unresolved | 15 | 40–45 (the test move) or write the path in full |
| service | board | 20 | 21–25 (board) |
| service | composition | 5 | 2 (contracts: wire/rpc types, verdicts, DTOs) |
| service | delivery | 2 | 32–36 (delivery) |
| service | engine | 3 | 8–10 (engine adapters), 7 (workflow runtime owner) |
| service | unresolved | 46 | 40–45 (the test move) or write the path in full |
| service | watch | 5 | 37–39 (watch) |
| watch | access | 3 | 11–15 (access) |
| watch | composition | 2 | 2 (contracts: wire/rpc types, verdicts, DTOs) |
| watch | engine | 2 | 8–10 (engine adapters), 7 (workflow runtime owner) |
| watch | service | 9 | 3 (context: paths, proc_table, push, test_support, runtime) then the terminal slices 26–31 |
| watch | unresolved | 8 | 40–45 (the test move) or write the path in full |

Steps 4 to 6 (git) move git code and clear nothing here directly; git's outward edges clear by 2
and 3. Step 1 is this check. The per-file list behind each count is in [per-file.txt](per-file.txt),
grouped by source layer, each line `file -> target  [target layer]`.

## Acceptance evidence

- **Fixtures** (`./scripts/daemon-layers.py --self-test`): moving a forbidden edge to a new file in the
  same layer passes; adding one fails; removing one without lowering the count fails ("cleared,
  lower it"); a new glob in a layer fails, and so does a second glob in a file that has one;
  moving a glob between files of the layer passes; a module's tests split into a sibling file keep
  the module's edge, while a non-test sibling that repeats an edge counts it again.
- **On current main, with ov-1287's stack on top** (ov-1287's commit dbac9e3db is stacked on
  e480d0e5e and bd4ae3a6f; all three cherry-picked onto this branch in a scratch worktree, which
  was removed afterwards):

  ```
  $ git cherry-pick e480d0e5e bd4ae3a6f dbac9e3db
  $ ./scripts/daemon-layers.py
  daemon-layers: ok (648 recorded edges in 55 layer pairs, none new)
  ```

  Before the per-module definition the same run failed: `engine may not use agents: 15 edges, 14
  recorded` and `engine may not use service: 40 edges, 39 recorded`, the two edges the ticket names.
- **Gate:** the check runs in the CI `wire` job and the lane gate's light jobs
  (`./scripts/lane-gate.py --self-test` lists all three steps). With the default cargo home the
  self-test passes. With `CARGO_HOME` set to an empty directory, 2 of its 16
  `landing-policy-test.py` tests fail on `cargo metadata --offline` ("no matching package named
  `bytes` found"): offline resolution needs a populated registry cache, which an empty directory
  cannot have. Those tests and that command are unchanged by this ticket.
  `copy-lint.py`, `actionlint.sh`, `ci-changes.py --self-test` and
  `workflow-review-timing-test.py` pass.

## Known limits

Unchanged from ov-1279 (the scanner reads text, not the compiler's name resolution): procedural
macros, macros from other crates, paths built by nested macro expansion, re-exports, and external
crates are not seen. New with counts:

- **Splitting a module into non-test siblings repeats its edges.** `foo.rs` and `foo_more.rs` each
  importing `service` count twice, because they are two top-level modules. A split that moves tests
  or nested files does not.
- **Several references to one module from one module are one edge.** A second `crate::service::X`
  line in a file that already imports `service` does not raise a count. A reference to a *new*
  target module does.
- **A pair's count is a total.** Adding an edge to engine -> board while removing another from it nets
  to zero and passes, which is the move the rule allows; the gate cannot tell a moved edge from a
  swapped one in the same pair.

## Reproduce

```sh
./scripts/daemon-layers.py --self-test
./scripts/daemon-layers.py
./scripts/daemon-layers.py --report
./scripts/daemon-layers.py --report-files > reports/ov-1290/per-file.txt
./scripts/daemon-layers.py --lower          # after clearing edges; never raises
```
