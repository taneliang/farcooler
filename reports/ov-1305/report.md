# ov-1305: gates use the fold base

## Receipt diagnosis

Receipt directory: `/Users/e-liang/Library/Application Support/com.farcooler.FarCoolerCanary/workflow/receipts/`.

For real group 3520, candidate `c3d51caf2bd873ec327b9d08d4908e37a75f5b1e`:

- `wf1:34effad81de6a6152ac7dd5be6425d09f93c5e8e99a36f2302f222af85d55c7b.process.json:1` records `binding.base = 3f78754f936855710dfec920fe3d492f879b610d`, the same value in `environment.FARCOOLER_BASE`, and argv `["/bin/sh", "-c", "scripts/landing-gate"]`. The environment is present. This real configuration bypasses `engine-gate.py` entirely.
- `wf1:1b267f589d3d2d81d41fb76389d6c7832a0363026d56ec0e7e886ceb758ea4e4.verb.json:1` is the adjacent fold operation for that group. Its binding still has the October base, but its incoming candidate and `detail.base` are `8628aecc07b8eec9b8fe0502b62d3e48df5625e6`; its output candidate is `c3d51caf2…`. The gate is a process, so it has no `.verb.json` of its own.
- `fold-wf1:366909b5892edb7677510f89ae29f237e19ca94caef7475ffa71dd7c79d80030-c3d51caf2bd873ec327b9d08d4908e37a75f5b1e.json:1` independently records that same base and candidate. The push receipt `wf1:5313ed256b157dd9709d2944fabcb788a6d203a64c3d66bdbb4c281e1514de28.verb.json:1` uses that base too.

The bad SHA comes from the runner/item binding, not a missing environment variable or a fallback inside landing-gate. Group stages have no Provision; `crates/daemon/src/workflow_runtime/adapters.rs:148` (`pin_base`) falls through to the runner’s configured baseline `self.base` (loaded from `config.base` at `crates/daemon/src/workflow_runtime.rs:382`), and `item_base` retains that binding. AwaitBase fetches current main into its candidate, then Fold records the actual base separately. Before this fix `crates/daemon/src/workflow_runtime/coding_execute.rs:331` (`pin_context`) always exported `binding.base`, ignoring the fold record. The local `main` ref in this checkout also resolves to `3f78754f…`, while `origin/main` resolves to `c3d51caf2…`; changing a remote ref would not fix the explicit stale environment.

The gate output `wf1:34effad81de6a6152ac7dd5be6425d09f93c5e8e99a36f2302f222af85d55c7b.output:8` prints 4,332 changed files and all eleven families against `3f78754f…`; line 6 lists unrelated historical shared inputs, and line 241 prints the same wrong base in the summary. The real group diff is 22 files.

## Change and safety

Group Run commands now obtain FARCOOLER_BASE from the durable fold index for their exact workspace, item and candidate. Missing records, foreign item/candidate records and malformed bases refuse execution. The admission Binding remains stable. Builders, reviews, CI provider hooks and ungrouped commands retain their existing base semantics.

The planner requires a resolved FARCOOLER_BASE under the engine and refuses an explicit override. Every diff base must resolve to a commit and be an ancestor of HEAD. It fails before loading workflows or starting work; it no longer silently substitutes a merge base for divergent history. The optional engine-gate adapter uses the same validation. Ordinary terminal invocation still defaults to origin/main.

CI workflows and path/shared-input classification are unchanged. Shared inputs still select all families and full Rust coverage. Rust selection still includes changes since the verified green ancestor; the fold base is not silently treated as green.

## Last ten landings on main

Snapshot: remote main `c3d51caf2bd873ec327b9d08d4908e37a75f5b1e`. Ordered by first-parent ancestry of unique folded gate candidates present on main, not gate completion time or group creation time. Groups 3304 and 3227 have no retained successful Push verb fact; their inclusion is established by main ancestry, so they must not be omitted from the last ten. “All” means `wire, doc-comments, relay, site, tunnel, rust, swift-tests, swift, ios, android, web`. These are planned job families; a fail-fast run may stop before executing every planned command.

| Group | Candidate | Fold base | Group files | Correct families | Correct Rust tests | Actual families | Actual Rust tests | Actual diff files |
| --- | --- | --- | ---: | --- | ---: | --- | ---: | ---: |
| 3520 | `c3d51caf2` | `8628aecc0` | 22 | All | 7,399 | All | 7,399 | 4,332 |
| 3484 | `8628aecc0` | `525b6d590` | 19 | All | 7,393 | All | 7,393 | 4,331 |
| 3402 | `525b6d590` | `d014f61ee` | 1 | wire, doc-comments | 0 | All | 7,383 | 4,327 |
| 3376 | `d014f61ee` | `3cdb5b23e` | 6 | All | 7,383 | All | 7,383 | 4,326 |
| 3359 | `3cdb5b23e` | `c10961168` | 14 | All | 7,382 | All | 7,382 | 4,323 |
| 3352 | `c10961168` | `a2d96bc54` | 7 | All | 7,375 | All | 7,375 | 4,321 |
| 3304 | `a2d96bc54` | `31646caf5` | 49 | All | 7,372 | All | 7,372 | 4,321 |
| 3264 | `31646caf5` | `8953bb06c` | 43 | All | 7,361 | All | 7,361 | 4,312 |
| 3263 | `8953bb06c` | `d2e88a5a1` | 4 | All | 4,982 | All | 7,352 | 4,302 |
| 3227 | `d2e88a5a1` | `d724f8d4e` | 4 | All | 7,349 | All | 7,349 | 4,299 |

Method: enumerate successful group gate processes whose candidates occur on pinned remote-main first-parent ancestry, deduplicate by candidate, sort by that ancestry, then join their matching Fold facts and available successful Push facts. Diff each fold base to its candidate. A missing Push fact is recorded as null in the JSON, not used to exclude a landing proven by main. Run the existing family selector and reverse Cargo dependency closure on those paths. Preserve the green ancestor recorded in each gate’s rust-selection.json, adding its intervening paths where it precedes the fold base. 4 of these ten gates retain an earlier green ancestor; this safety widening is included in the counts. Use the current source graph at the pinned main snapshot; no builds or gates were run for this analysis. Count historical tests from each gate’s default-host nextest log by unique test ordinal, after the gate profile’s exclusions. Narrow counts sum those recorded test cases by selected package. Full counts equal the recorded suite size; reruns are not double-counted. Feature suites, doctests and lints are outside these nextest counts and remain unchanged.

The accompanying `landing-selection.json` records exact commits, changed paths, packages, fallback reasons, per-package counts, and original gate/fold receipt filenames and retained push receipt filenames and nextest log paths for every row. Eight of these ten groups still conservatively need broad proof for shared inputs in the group or since green; group 3402 becomes a docs-only gate, and group 3263 runs 4,982 instead of 7,352 Rust tests.

## Verification

The three-file replay in `scripts/landing-policy-test.py:366` runs the real wrapper planner against synthetic history. A stale origin/main points at runner startup, then main advances before the group base is captured. The folded candidate changes `crates/android/src/lib.rs`, `README.md`, and `REPORT.md`. Its plan reports exactly three files against the group base and selects wire, doc-comments, site, tunnel, rust, android. Rust selects leaf and its consumer (one test each), skipping the unrelated crate and its test.

The wrapper tests cover a missing engine FARCOOLER_BASE, an attempted engine base override, and a divergent sibling commit with a common ancestor: all refuse with clear messages. The Rust regression uses two real folds around a moved base, inspects both process receipts, and verifies each gate got its own fold base; it also removes/replaces fold evidence to prove there is no startup-base fallback.

The lane self-test copies source contents (filenames from the index, never real HEAD/history) into a temporary repository with synthetic commits and a private green-candidate fixture. All planning, summaries and history-dependent fixtures run there. It clears inherited engine context and avoids network green-history lookups. This prevents the old self-test from touching the caller’s Git summary or depending on the caller’s last commit.

Final results (all commands in the foreground; Cargo build jobs and nextest test concurrency capped at four):

- `./scripts/lane-gate.py --self-test` with CARGO_HOME unset: PASS (`/tmp/ov-1305-self-test-default.log`). Git status was clean before and after.
- The same command with CARGO_HOME set to a newly created empty directory: PASS (`/tmp/ov-1305-self-test-empty.log`). Git status was clean before and after.
- `python3 scripts/gate-evidence-test.py`: 18 tests passed. Its fixture now clears inherited FARCOOLER context before creating its own engine binding, so a real engine invoking these tests cannot impose its base on the synthetic repository.
- `python3 scripts/engine-gate-test.py`: 9 tests passed.
- All 15 `scripts/*-lint.py` programs: PASS (`/tmp/ov-1305-lints-final.log`).
- `python3 scripts/file-size-budget.py`: PASS (2,168 files, 27 grandfathered).
- `python3 scripts/daemon-layers.py`: PASS (648 recorded edges, 55 layer pairs, none new).
- Complete daemon and workflow nextest run: **4,122 passed, 19 skipped, no failures and no retries**. Both leak guards passed; test-leak-check reported nothing left in TMPDIR or preferences (`/tmp/ov-1305-nextest.log`). Both crates’ doctest targets also passed in the initial Cargo run.

The complete Rust command, after `cargo build -p farcooler-cli --bin farcooler` supplied the native-hook test prerequisite, was:

```sh
CARGO_BUILD_JOBS=4 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 \
NEXTEST_TEST_THREADS=4 CI=true FC_LEAK_SHARED=1 \
GATE_FLAKY_LOG=/tmp/ov-1305-flaky.tsv \
env -u FARCOOLER_NATIVE_HOOK_SOCKET \
python3 scripts/test-leak-check.py -- python3 scripts/tmux-leak-check.py -- \
python3 scripts/gate-flaky.py run -- \
cargo nextest run --profile gate --no-fail-fast \
  -p farcooler-daemon -p farcooler-workflow
```

The initial raw Cargo run found a missing hook CLI prerequisite and two watcher/native readiness failures. After building the CLI and using the repository’s normal gate environment isolation and nextest runner, the entire two-crate run passed on its first attempt. No unrelated Rust code was changed. An initial lane self-test also exposed its old 0.8-second light-job timing assertion; the self-test now observes the lock boundary directly, proving light work never enters the lock without depending on process-startup speed.

No full landing gate, cargo fmt, stash, reset, clean, amend, or blanket staging command was run against the working repository. Tests use synthetic Git history; real history was read only for the requested receipt/landing analysis.
