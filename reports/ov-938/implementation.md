# ov-938: bounded owned native queue cleanup

Base is exact signed `fae3f30d0c3d1c475f9dc4463243f293fd88b007` on
`ov-938-notice-group-ownership`. This is only the SAME933 P2 group-supervision
fix. Review of the final signed delta by the SAME933 reviewer remains required.

## Ownership and settlement

`workflow_notices::invoke` attaches the existing `effect_runner::Group` guard
in the synchronous spawn result, inside the existing Store admission closure.
`OwnedQueue` declares the guard before `Child`, so unwind/drop signals the group
before the child's destructor/reaper can release the leader identity. There is
no await between spawn and attachment. Store's admission return has no fallible
operation after spawn; its target authority and original attempt are unchanged.

`queue_has_exited` uses waitid(WNOWAIT|WEXITED|WNOHANG) on that direct spawned
child. It observes exit without reaping; the unreaped leader pins the PID/PGID
until the guard's single checked kill attempt. The synchronous peek keeps
macOS's non-Send siginfo_t out of the async state. The finite queue wait polls
only this direct child, not a provider transcript or notice availability.

The guard is disarmed after the pinned signal attempt, including errors, before
any leader reap. No later drop can send another signal to a reused numeric ID.
After the signal attempt, Tokio is the sole leader reaper with a one-second
reap deadline. Then up to 100 probes, separated by 10ms sleeps, must observe
killpg(pgid,0) failing specifically with ESRCH. EPERM and every other error
remain unproved and are polled within the same bound. A failure to reap or prove
disappearance returns explicit Unknown evidence. Signal-plus-cleanup failure is
logged and has its own Unknown evidence. The original attempt/target is retained;
there is no pending downgrade, rebinding, resend or fallback.

MacOS returns EPERM when a group contains only zombies, even for SIGKILL to our
own exit0 leader. `kernel-probe*.json` records actual exists -> EPERM -> ESRCH
transitions and the exit0 leader-only signal-EPERM/reap/ESRCH case. A signal error
alone never permits settlement. Independent disappearance after reaping can
conclusively prove cleanup despite that signal error. Only observed exit0 plus
leader reap plus group disappearance yields the existing **local handoff only**
receipt. This neither claims provider consumption nor recalls admitted bytes.

The guard's preexisting fixture/landing/configured-runner behavior is unchanged.
Its added checked signal/disappearance methods are used by queue supervision;
its existing Drop primitive is reused on queue future drop/unwind. No new
process framework, detached cleanup task or target discovery mechanism is added.

## Private regression evidence

The five-mode native regression forks a real same-group descendant and writes
leader/PGID/descendant/descendant-PGID receipts before leader exit. Normal exit,
shutdown, timeout, aborted future and panic unwind check both processes and the
group disappear within a bound. Every mode starts an independent-group private
backend sentinel, confirms it survives queue cleanup, then kills/reaps that
explicitly owned test child. Delivery is handed_off only for normal exit and
Unknown otherwise (prepared drop/unwind attempts pass through existing recovery).
Original attempt and target survive; a newly discovered endpoint cannot resend.

The retained-zombie regression forks a separately grouped private reaper and a
same-group child. The reaper deliberately postpones waitpid until fixture
release; after the queue exits0 and cleanup is attempted, the group's zombie
keeps disappearance unproved. The reaper survives, the outcome must be Unknown,
and the original attempt/target cannot resend. Release occurs before assertions,
including against old source, so the private reaper can reap and exit naturally.
All helpers have finite private lifetimes; none is a real provider/backend.

`negative.py` substitutes the exact base queue source only, saves both byte
snapshots, demands actual behavioral assertions for false handoff and a live
owned descendant surviving future drop, and restores exact fixed source in a
finally block. Old drop helpers expire under their own 30s private bound; the
controller records disappearance without followup signals to recycled IDs.
Compile errors and fixture errors are preserved and never receive negative
credit. Final source/evidence hashes and gate results are in freeze.json.

## Reused evidence and limits

`reuse.json` binds SAME933's independent report and prior freeze. All 127 prior
evidence hashes and the 29 prior product inputs outside the two changed notice
files match. Prior 140 main passes and nine behavioral guard controls are reused,
not rerun or added to new delta counts. The three prior ignores include a direct
helper invoked by parents and two older log_join ignores. Six early negative
graphs predate the inventory/closed-owner extension; the prior final sealed
suite/lint covers that extension. Nested child summaries are excluded.

Every delta Cargo route uses native /usr/bin/python3, normal heavy FIFO,
original923 warm target, offline/jobs6/dev-debug0/test-debug0/incremental0,
load5 below20, a retained 1800s supervisor, and 10GiB reserve. Admission checks
actual cargo-lock holders, validates prior warm-target provenance, records
relative dep-info and binds local source roots. Saved before/after input maps
prove bytes stayed fixed during each run. No parallel or duplicate target,
owner terminal/provider/DB/app/config/security write, cutover, publication,
main integration or combined canonical CI claim is made.
