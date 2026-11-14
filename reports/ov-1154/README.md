# ov-1154 engine stack

Base: `wave-a-preview` (`b954ea25f`). The existing ticket branch was a direct
ancestor, so it was fast-forwarded to that base rather than replaced. Applied
all 31 commits, in the requested order: ov-1146 (8), ov-1153 (6), ov-1147 (10),
ov-1142 (7). The empty ov-1142 rebase checkpoint is retained as an empty commit.
No feature work is included.

| Ticket | Reviewed tip | Applied tip |
| --- | --- | --- |
| ov-1146 | d378f70a9 | 6cf9ca72e |
| ov-1153 | e70363f0d | 1e27b3719 |
| ov-1147 | 2dd82817c | d60e1f581 |
| ov-1142 | 724cea500 | 58cb262ba |

## Conflict resolutions

| Source commit | Conflicting files | Resolution |
| --- | --- | --- |
| c2388612b | daemon workflow_runtime/coding_native.rs | Add visible-screen capture while retaining the settlement's startup-failure return value and execution tracking. |
| 71d298e56 | daemon workflow_runtime/coding_native.rs | Keep activity settlement, session closure and failure reason; continuously capture PTY bytes and drain after quiescence, retaining output across terminal clears. |
| 91023f222 | daemon workflow_runtime.rs; store workflow_effects.rs, workflow_overlay/tests.rs; workflow lib.rs, tests/groups.rs | Keep both modules and both regression suites. Decision effects may run outside Running; ordinary effects retain WaitingSlot eligibility. Claims for hooks use generation zero; ordinary claims use the resource generation. Execute hooks inside the existing activity scope. Keep execution fields in fixtures. |
| 8f67fa552 | cli workflow.rs; daemon workflow_runtime.rs, workflow_runtime/coding.rs, coding_pr_tests.rs, coding_verbs_tests.rs; store workflow_overlay/tests.rs; workflow definition.rs, reducer.rs, templates.rs, tests/followers.rs, gates.rs, pr_verbs.rs, send_back.rs, tracker.rs, verbs.rs | Preserve ResolveCancellation and describe the published retry limit. Carry script policies, admission policy and execution limits together through definitions and fixtures. Keep runtime activities and test-only effect cap; both hooks and landing groups bypass build slots. Retain all CI regressions and auto-queue tests. Preserve optional u32 group caps, unlimited groups and the current default back-edge allowance of five. Published execution limits control build followers independently of landing batch limits. |
| c47a0ce26 | cli workspaces.rs; daemon landing_templates.rs, workflow_runtime.rs, workflow_runtime/landing_templates_tests.rs, workflow_summary.rs; store workflow.rs; workflow definition.rs, reducer.rs, templates.rs, tests/groups.rs, templates.rs | Keep the current Option<u32> group representation (None is unlimited, Some(0) invalid), adapting consolidation to it. Batched templates default to unlimited as requested by ov-1108. Refresh groups using the current workspace list and advancing admission clock. Keep hook commands in store/reducer matches, execution fields in new groups and both sets of tests. Joins use the published grouping pin while active work retains its own pin. |
| e26c69559 | daemon workflow_runtime/coding_verbs_tests.rs; workflow reducer.rs | Keep all CI/admission tests and add diagnosis tests. Combine exact member diagnosis with existing conflict-file evidence and bounded rebase briefs. Diagnosed bad tickets with a build step return there without spending join review iterations; healthy members remain together. Old path-only fold outcomes retain their existing replay behavior. Keep split termination based on decreasing member count, independently of review back-edge limits. |
| fc6d775aa | daemon workflow_runtime/coding_verbs_tests.rs | Retain CI/admission tests and apply the expanded diagnosis cases, including mixed base/member conflicts and rollback of partially applied rejected members. |

## Integration adaptations

Fold outcomes carry optional diagnosed members alongside conflict paths. An absent
member list preserves legacy path-only events; an explicitly empty diagnosis is
invalid. Fold receipts keep both member identities and the union of conflict paths.
New fixtures use the current batch representation and complete definition fields.
Script failure inputs advertise the run's published retry allowance rather than
the former constant. The new `failure_rule_advertises_the_published_retry_allowance`
regression was run before that fix: it failed because a zero-allowance policy
still advertised `retry`. It also covers raising the allowance beyond three.
The diagnosed-member regression checks that paths survive in the brief and no
join back edge or review iteration is spent. Two incoming daemon tests expected
bad tickets to stay Failed after diagnosis. The preview instead rebuilds those
tickets; the isolated failing run showed all three bad tickets at WaitingJoin
and the healthy batch Done. Their assertions now require that combined behavior:
exactly one rebuild and check per bad ticket, no spent join/review budget, and
unchanged fold/gate/CI-watch counts for the healthy batch.

The full socket suite exposed another interaction: refreshing every ungrouped
publication created an unnecessary state revision, making four safety tests'
commands stale. Refresh now leaves a never-grouped workflow alone, but still
replaces a previous grouping when a publication removes its groups. All 18
socket tests pass after the fix; publication/removal tests cover both cases. The new regression was also seen
failing with the former unconditional refresh restored at its call site, then
passing with the fix. All 32 publication tests pass.

A final reducer regression also exposed consolidation undoing a split script's
`own_group` choice. It failed before the correction, showing the ticket moved
into the existing waiter. Consolidation now preserves a settled script's own
group choice on later sweeps as well. Built-in splits still consolidate, and a
new publication can replace the script policy. This preserves ov-1153's decision
authority alongside ov-1142's default consolidation rule.

## Validation

- `cargo clippy --workspace --all-targets -- -D warnings`: passed.
- File-size budget, protocol lint, doc-comment check against `b954ea25f`, and
  `git diff --check`: passed.
- Pure workflow suite after the final consolidation correction: 175 passed.
- Landing-verb integration cases: 34 passed.
- Publication/refresh cases: 32 passed.
- Socket workflow service cases: 18 passed, 1 ignored.
- Full four-crate test command below: passed, exit 0, including the tmux leak
  wrapper. Default ignored tests remained ignored.
- After the final script/consolidation correction: workflow suite 175 passed,
  daemon script-hook suite 6 passed, and workspace/all-target clippy passed again.

The full suite used:

```sh
PATH=/opt/homebrew/bin:/usr/bin:$PATH env \
  -u FARCOOLER_NATIVE_HOOK_SOCKET -u FARCOOLER_NATIVE_SESSION \
  -u FARCOOLER_WORKFLOW_FENCE -u FARCOOLER_WORKFLOW_ITEM \
  -u FARCOOLER_WORKFLOW_MODEL -u FARCOOLER_WORKFLOW_RUN \
  /usr/bin/python3 scripts/tmux-leak-check.py -- \
  cargo test -p farcooler-workflow -p farcooler-store \
    -p farcooler-daemon -p farcooler-cli --no-fail-fast -- --test-threads=6
```

All builds use this worktree's own `target/`. The installed stable CLI symlink
was broken, so ticket descriptions were read through `farcooler-canary task show`.
Native compiler and linked-worktree Git metadata operations required access
outside the filesystem sandbox. The commit hook uses system Python because the
machine's `/usr/local/bin/python3` has an incompatible CPU architecture.

The first full test run inherited this engine agent's
`FARCOOLER_NATIVE_HOOK_SOCKET`, redirecting CLI hook fixtures to the live socket.
Verification clears that and the inherited native session and workflow run
variables; no product change is needed for this test-environment issue.
