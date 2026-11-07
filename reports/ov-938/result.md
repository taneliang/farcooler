# ov-938 final builder result

Exact base/parent: fae3f30d0c3d1c475f9dc4463243f293fd88b007.
Branch: ov-938-notice-group-ownership. Stop at the signed delta for SAME933 review.

- Final restored lease: **12 passed, 0 failed, 0 ignored** (8 native notice checks,
  existing group timeout/cancel, native original-owner stop, Service join-before-
  restart, configured process cancellation). No nested summaries counted.
- Daemon all-target Clippy with `-D warnings`: exit0. Size budget: exit0.
- Supervisor exit0, stopped=None, 132.923s including FIFO wait,
  minimum free bytes 23541370880; no reserve/deadline stop.
- All 1231 before/after source inputs match each other, the earlier
  eight-pass native graph, and current bytes. The source is restored exactly to
  negative.json's fixed_sha256. Both old-source runs exited101 at actual assertions.
- Prior 140 passes / three ignores / nine controls are explicitly reused, with
  their original graph/stand-in limits in reuse.json; these are not fresh delta
  counts and are not added to 12 to manufacture a new total.

Source review anchors: workflow_notices.rs:77 attaches the reused Group at spawn;
:99 defines drop order; :108 peeks WNOWAIT; :121 gates settlement on bounded reap
and ESRCH. effect_runner.rs:8 adds checked methods to the existing retained guard;
preexisting run/fixture/configured/landing behavior is unchanged. Native lifetime
regression is workflow_notice_tests.rs:183, retained-zombie refusal :237.

Actual leader/group/descendant IDs, external-group survival, exact delivery state,
attempt and target receipts are in final-restored.log. The deterministic old
counterexample is old-unproved.log: group_unproved=true while handed_off. Old
future drop is old-drop.log; negative.json records leader ESRCH while descendant
remained live, then its bounded natural exit without further signals. The old
source hash matches exact base queue bytes; both snapshots are preserved.

Initial compile/fixture failures and the intermediate zombie-only EPERM behavior
failure remain saved. None receives behavioral-negative credit. The finite
kernel probe explains macOS EPERM until reaping and the independent ESRCH proof.

SAME933 independent review is still required to close the sole blocker and final
delta. No real provider consumption/exactly-once, app/main/data/cutover,
publication or combined CI claim is made. No sibling source tree was edited.

The first commit attempt failed before creating a commit because the hook's env
python3 selected an Intel-only binary. commit-environment.txt records the cause
and per-command native-Python PATH correction. Hooks and signing stay enabled;
product bytes and all final checks remain unchanged.
