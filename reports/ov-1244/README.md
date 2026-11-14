# ov-1244: engine train on the CI train

Base: `cbf8a84cad5b8d2b85897f32cd563884668db083` (`origin/main` at start).

The requested fast-forward to ov-1242 was impossible: this base includes ov-1243,
which is absent from ov-1242. Instead replayed every non-merge commit in
`origin/main..ov-1242-ci-train-main` with `cherry-pick -x`, preserving the newer
main commit. The CI prefix applied without conflicts. Then replayed
`7b1b4dd5e..ov-1238-engine-train` in order, followed by the two non-merge
`origin/main..ov-1220-uncertain-starts` commits, also with `-x`.

## Tickets carried

CI prefix: ov-1186, ov-1200, ov-1188, ov-1205, ov-1207, ov-1211, ov-1216,
ov-1209, ov-1184, ov-1219, ov-1183, ov-1215, ov-1194, ov-1225, ov-1230,
ov-1223, ov-1185, ov-1218, ov-1224, ov-1189, ov-1182, ov-1190, ov-1226,
ov-1156, ov-1239, ov-1229, ov-1237, ov-1240, ov-1242.

The six waiting engine tickets: ov-1213 (its twelve-ticket stack: ov-1154,
ov-1155, ov-1164, ov-1146, ov-1153, ov-1147, ov-1142, ov-1139, ov-1145,
ov-1152, ov-1159, ov-1203), ov-1231, ov-1235, ov-1236, ov-1233, ov-1178.
Their source ancestry also carries ov-1143/ov-1138, ov-1009, ov-1127/ov-1128,
ov-1114/ov-1132, ov-1106/ov-1131, ov-1108, ov-1105, ov-1158, ov-1217,
ov-1170/ov-1214, ov-1179/ov-1201/ov-1206 and ov-1171. Retained ov-1238's
existing combined-fixture/semantic conflict resolutions and report commits.
Finally ov-1220 supplies uncertain-start recovery and the `workflow resolve` verb.

No requested engine ticket was omitted. The ov-1238 queued prefix through
`7b1b4dd5e` was deliberately excluded: ov-1157, ov-1151, ov-1008, ov-1149,
ov-1137, ov-1162, ov-1165 and ov-1172 are already on main; ov-1186, ov-1200,
ov-1205, ov-1207, ov-1211, ov-1216, ov-1209, ov-1184, ov-1219, ov-1183,
ov-1215, ov-1225, ov-1230 and ov-1223 come through the CI prefix. Their
supporting ancestry ov-1160, ov-1181, ov-1180, ov-1098 and ov-1020 is on main;
ov-1188 and ov-1194 come through CI. No duplicate prefix picks were attempted.

## Conflicts

10602945a: scripts/lane-gate.py retained CI process supervision and self-test; scripts/process_guard_test.py took expanded engine tests (superset).
adefdc89e: .config/nextest.toml retained CI archive includes and existing equivalent exclusive recovery-test override. Restored complete CI lane-gate.py after detecting an overbroad conflict-marker edit.
e86fd06d2: crates/cli/src/workflow.rs, crates/client/src/ffi/route.rs, crates/daemon/src/people_rulings.rs, crates/daemon/src/rpc.rs, crates/daemon/src/workflow_runtime.rs, crates/daemon/src/workflow_runtime/coding_execute.rs, crates/daemon/src/workflow_runtime/coding_settle.rs, crates/daemon/src/workflow_runtime/wire_commands.rs: union all verb registries/scopes/rulings; retain landed authority refusal and add resolve dispatch; retain generalized refuse_unstarted helper and route provisioning failure/resolve to it; preserve landed read hook and add provision resolution implementation.
0182e5ad1: workflow_runtime.rs retained execution-state projection plus Resolve test inbox; store/workflow.rs retained applied projection tracking plus resource-holder refusal; store/workflow/tests.rs retained both independent regressions.

## Validation

Foreground commands use a child environment without inherited
`FARCOOLER_*` task bindings, native Python on PATH, four build/test threads,
and isolated `CARGO_TARGET_DIR=/tmp/ov-1244-target`.

Imported commits: 94 CI, 121 engine (including existing integration fixes), 2 ov-1220.

Passed checks:

- `python3 scripts/lane-gate.py --self-test` (including ten process supervisor tests).
- `gate-evidence-test.py`, `engine-gate-test.py`, `nightly-dispatch-test.py`,
  `ci-concurrency-test.py`, and `ios-ui-shards.py --self-test`.
- `ios-ui-shards.py --check` and `proto-lint.py`.
- Built CLI `workflow --help` and `workflow resolve --help`; combined verbs
  `resolve`, `resolve-cancellation`, `close`, `landed`, `pause`, `resume`, and
  `rollback` appear together. Help is retained beside this report.
- All 217 source commits have their `-x` trailer and match the manifest order.
  CI prefix tree equality against ov-1242 holds except the preserved ov-1243 report.
- `git diff --check`; zero merges in `origin/main..HEAD`; main is an ancestor.

Rust compilation completed successfully (4m31s). The required foreground nextest
selection passed: **2,143 passed, zero failed** (245.114s; 3,244 outside the filter).
The two existing unused-variable/assignment warnings in store projection tests remain.

Required nextest selection (in the clean child environment described above):

```sh
cargo nextest run \
  -p farcooler-workflow -p farcooler-store -p farcooler-cli \
  -p farcooler-daemon -p farcooler-protocol -p farcooler-client \
  --test-threads 4 --no-fail-fast \
  -E 'package(farcooler-workflow) | package(farcooler-store) | package(farcooler-cli) | (package(farcooler-daemon) & (test(workflow_runtime) | test(rpc) | test(hook_install) | test(answer_wake::engine) | test(engine_steering) | test(message_send)))'
```

The post-selection check targets protocol workflow commands, client workflow/FFI
routing, and daemon member rulings, process supervision and yielding. This covers
merged registries and supporting paths outside the required selection.

No new behavior was added in integration. No cargo fmt, swift-format, stash,
reset, clean, checkout-index, amend, git add -A, pkill or killall was used.

Final supplemental checks: **69 passed, zero failed** (2.413s), plus the explicit
`ffi::phone_path_tests::every_route_has_an_arm` completeness regression:
**1 passed**. All test commands finished with exit 0 in the foreground. Logs are
retained in this directory; skipped tests are outside the selected scope.

```sh
cargo nextest run -p farcooler-protocol -p farcooler-client -p farcooler-daemon \
  --test-threads 4 --no-fail-fast \
  -E '(package(farcooler-protocol) & test(workflow)) | (package(farcooler-client) & (test(ffi::route) | test(workflow))) | (package(farcooler-daemon) & (test(people_rulings) | test(yielding) | test(effect_runner)))'
cargo nextest run -p farcooler-protocol -p farcooler-client -p farcooler-daemon \
  --test-threads 4 -E 'test(every_route_has_an_arm)'
```

Acceptance: all four items satisfied. Linear imported history is CI prefix,
engine tickets with their existing integration fixes, then ov-1220. This report
and its evidence are the final ov-1244 bookkeeping commit. The only deviation
from the requested commands is replaying the CI prefix instead of fast-forwarding
it, required to retain the newer main commit without a merge.


## Pre-landing return: file budget and shell startup

The full return reason was read from the local workflow state after the installed
Canary CLI's `workflow get` refused this caller (`known caller required`). Its
remaining instruction is to keep history linear, commit as work proceeds and
never run cargo fmt; there are no additional truncated requirements.

Moved `terminal_result` and `with_activity`, including their existing comments,
unchanged into `crates/daemon/src/rpc/terminal_view.rs`. Their visibility is now
`pub(super)` so the parent handler can call them. `rpc.rs` shrank from 2,468 to
2,394 lines. `./scripts/file-size-budget.py` passed, and its `--update` lowered
the manifest ceiling from 2,448 to 2,444. The manifest was generated by the script.

No stacked commit changed the evidence shell launch: `gate_evidence.py` before
this return fix is byte-identical to both `origin/main` and
`ov-1242-ci-train-main` (SHA-1 of file bytes:
`9f8c26fc9c1eb9692000100bf919bbef1e64a3db`). The reported test also passes when
launched directly here. A controlled socket-stdin reproduction explains the
failure: noninteractive `/bin/bash -e -c` reads the user's `.bashrc` when stdin
is a socket (Bash's remote-shell detection). It does not require `-l` or `-i`.
The same command with `--noprofile --norc` skips that file. These flags now cover
both the evidence step launcher and its Android tool-resolution shell.

Added a real socket-stdin regression with a temporary HOME whose `.bashrc`
prints a sentinel and exits 42. It fails against the original launch (42 instead
of 23), and passes with the fix, preserving the exact `raw error\n` assertion.
All 17 evidence tests pass. The existing failure/unknown-start/environment-output
test remains unchanged. Initial sandbox test attempts failed because Git's
`confstr` warning polluted fixture SHA output; reruns outside the sandbox pass.
Direct shebang execution also needed native Python first on PATH because
`/usr/local/bin/python3` has an incompatible CPU architecture on this host.

Return validation completed in the foreground:

- `CI=true ./scripts/lane-gate.py --base origin/main --skip rust,swift-tests,swift,ios,android,web`
  exited 0. Every selected job passed: wire, doc-comments, relay, site and tunnel.
  This is the explicitly requested partial gate, with the named jobs skipped;
  it is not a claim of a full application gate. See `return-wire.log`.
- Repeated the exact required nextest selection documented above with the same
  isolated target directory and four build/test threads: **2,143 passed, zero
  failed**, 3,244 outside the filter, 270.298 seconds after compilation.
  See `return-acceptance.log`. Compilation validates the combined literals and
  the extracted RPC module. Only the two existing store-test warnings remain.
- CLI help still includes `resolve`, `resolve-cancellation`, `close`, `landed`,
  `pause`, `resume` and `rollback`; refreshed help is in `return-workflow-help.txt`
  and `return-workflow-resolve-help.txt`.
- Fresh file-budget check: 2,138 files, 27 grandfathered, passed. Helper bodies
  and comments match their pre-extraction source exactly, apart from visibility.
- Final `git diff --check` passed; zero merges in `origin/main..HEAD`, and
  `git merge-base --is-ancestor origin/main HEAD` succeeds. The gate refreshed
  origin/main and it remains `cbf8a84cad5b8d2b85897f32cd563884668db083`.

The shell fix and RPC extraction were committed separately before this evidence
commit. No prohibited formatter or Git command was used.
