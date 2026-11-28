# ov-1527: how far the workday walk gets, and what runs it

## The contradiction

The committed walk on main runs to the end, restarts included, and CI runs it.
The ov-1490 and ov-1491 reports were out of date when those tickets landed.

- Both measured against base `8d8a2f2b7`. There the walk did stop at
  **Checking Here**.
- `39d1142df` (ov-1520) fixed that in the shared Plan view
  (`crates/client/src/plan_view/landing_queue.rs`).
- The ov-1490 and ov-1491 commits landed on main on top of that fix. `33b4c71ef`
  and `571a7334b` both contain it. Their reports still described the old base.

`reports/ov-1490/README.md:85` and `reports/ov-1491/README.md:37-39` are wrong
about main. I left them as the record of what those lanes saw.

## How far it gets on main (`45a6fa704`)

Line numbers in this section are as of `45a6fa704`.

**The step:** `.github/workflows/ci.yml:1993`, "Walk a workday on two isolated
Rust runners", in the `web-bundles` job ("Web bundles and browser checks",
line 1868). It runs `csp-check.mjs` twice: with `--join` (line 2002), and then
the workday walk (lines 2005-2007).

**The walk:** `services/web/test/browser/csp-check.mjs:1689` calls
`remoteWorkday` (line 1351).

| Line | Step |
| --- | --- |
| 1539 | Plan shows **Checking Here** (passes) |
| 1544 | calls `terminalsWalk` (line 1551) |
| 1605 | opens a shell in a chosen worktree and types in it |
| 1612 | exits that shell and restarts it in place |
| 1633 | starts Claude Code with a chosen model |
| 1639 | restarts the running orchestrator |

It can't stop early and still pass. A step that throws becomes "the walk
stopped", and each failed scenario is added to `failures`. Any failure exits 1
(line 2016).

**CI logs:** every line above is printed, through "restarted the running
orchestrator" and "no CSP violations":

- run 46121521584 on `5960d575a`, job 140913526930
- run 46126724580 on `45a6fa704`, job 140922889466

## What this ticket changed

On main, a shell's exit and restart ran on one of the two runners only
(`build-box`). The walk now also switches to `studio`, opens a shell there,
exits it and restarts it (`csp-check.mjs:1545-1547`). The two agent scenarios
still run once. Each terminal log line now names its runner. No step was removed
or loosened, and `ci.yml` is unchanged: the same step runs the added lines.

## Breaking restart (shown once)

These ran on this Mac, with canary builds and the changed walk. Each break was a
temporary edit to `terminal.restart` in `crates/daemon/src/rpc.rs` that returned
success without relaunching. Neither is committed. CI wasn't run with either.

**1. No restart at all:** exit 1. It stopped before the shell scenarios, because
**Recover** on the lost terminal uses the same call:

```
csp-check: the walk stopped: locator.waitFor: Timeout 30000ms exceeded. at remoteWorkday (…/csp-check.mjs:1444:74)
```

**2. No restart, except for the lost terminal's recovery:** exit 1, with all
three restart scenarios red. The other scenarios passed.

```
csp-check: workday: build-box: restarted a shell whose process exited, in place: locator.waitFor: Timeout 30000ms exceeded. … #terminal-state … /^Running$/
csp-check: workday: build-box: restarted the running orchestrator: timed out: a new epoch
csp-check: workday: studio: restarted a shell whose process exited, in place: locator.waitFor: Timeout 30000ms exceeded. … #terminal-state … /^Running$/
```

## With the break removed

The workday walk passed twice in a row on macOS, with no CSP violations. Before
the change, the walk as committed on `45a6fa704` also passed here, in 50 s.

I didn't run the join walk or the fake daemon walk locally, because the edit is
only in `remoteWorkday` and `terminalsWalk`. I also ran no Rust tests, since no
Rust change is committed. The changed walk hasn't run on Linux CI yet.

## ov-1491 line 1

It can be ticked from the committed walk alone. The line asks for a new shell in
a chosen worktree that a person types in, against a real runner. The scenario
"opened a shell in a chosen worktree and typed in it" does that. It checks the
worktree and reads the typed output back from the runner's own CLI. It passed on
Linux CI in both runs above. No temporary copy is needed.
