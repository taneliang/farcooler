# ov-1243: a bot commit on main stalled the landing line

Incident note, Nov 15 2026 (SGT). Evidence for ov-1243; no code change.

## Timeline

| Time | Event |
| --- | --- |
| 05:30:31 | Landing group 2174 folds four tickets onto `4a0fee927`. |
| 05:31 | `canary-baseline.yml` pushes `f9afc6472` ("chore: canary wire baseline at 4a0fee9278") to main. |
| 06:09:24 | The group's gate passes after 39 minutes. |
| 06:09:28 | Its push is refused as `base_moved`; the group returns to its base step. |
| 06:10 on | The base step waits for a CI verdict on main's head, `f9afc6472`. |

## Why it would not have recovered

- The bot pushes with the default `GITHUB_TOKEN`, which starts no workflow
  (`canary-baseline.yml`, "NO LOOP"), and `ci.yml` ignores pushes that touch only
  `proto/baseline/canary.proto`. `f9afc6472` therefore never gets a CI run.
- The engine's base step (`await_base` in
  `crates/daemon/src/workflow_runtime/coding_verbs.rs`) polls until CI on the head
  has passed or failed. With no run it polls for its full three hours, then times out.

## What unblocked it

- An empty commit (`20e568b0d`) did not start CI: a push that changes no files does
  not trigger a workflow with a `paths-ignore` filter.
- This note is the fix: a docs-only commit under `reports/`, which `scripts/ci-changes.py`
  classifies as docs, so CI runs its light jobs and gives main's head a verdict.

## Cost

One wasted 39-minute gate and about an hour of landing time. It recurs whenever
Canary ships a protocol change.
