# Source-only checks

- Four requested normal cherry-picks: exit0, no conflict. Exact identities in handoff/composition.
- Approved original vs replay comparison on all ten ov937 product files, NativeAgents.swift and build-app.sh: zero bytes of diff (`replay-product-diff.txt`). Comparison is scoped to each picked product set: ov937's parent predates the approved group-fix base, so comparing its entire tree to this later base would include intentional inherited differences.
- Base4c645 vs replay HEAD on effect_runner.rs, answer_wake/workflow_notice_tests.rs and workflow_notice.rs: zero diff; base group work preserved.
- Source identity manifest: 44 files, SHA256 checked against current source. Includes both Rust lockfiles, normal startup/CLI, shared display/session/bridges and actual app roots/build scripts. No native artifact produced or inspected.
- `git diff --check`: exit0. No builds/tests or application code execution, no toolchain/capacity/OS probes, no cache copying, no runtime data access, no network fetch or GUI launch.
- Reports are new only under rebind-prep. Old reports keep their recorded meaning. Current executable checks0, current full-app passes0, current native/UI captures0, current end-to-end receipt rows0. These zeros describe this prep phase, not retroactive counts for historical runs.

Independent same-ports review is requested through orchestrator at the signed handoff pin. It has not been performed by this builder. Final936 composition, runtime phase, app-profile/auth dispositions and the task's full acceptance remain open.
