# Native availability-only reconnect counterexample

Current released LiveInventory source has no subscription API. Store-backed
terminal/session/role writes and Service start/stop wake the new notification
owner, and tests prove those paths drain pending records. But publication of a
healthy inventory after an unhealthy view, without any durable target row
mutation, cannot wake an already sleeping notification owner. This is a concrete
omission in reconnect coverage, not a reason to add a notice idle timer.

Allocation requested through the orchestrator: a minimal identity/liveness watch
in crates/tmux/src/inventory.rs, plus one select arm in the existing owned notice
drain. Publish only changes to inventory availability or pane terminal/daemon/
worktree identity, pane id, tty, pane PID or life/death. Exclude screen stamps,
output, title, layout and dimensions. Repeated identical refreshes do not wake.
No additional task, delivery framework, timer or history scan is needed. A watch
is a hint, never native authority; final current Store/kernel fencing remains
mandatory. The initial drain plus retained watch generation avoids lost wakes.

Private proposed evidence: subscription unit test for health/identity changes and
unchanged refresh silence; an actual private pane respawn (no Store mutation)
wakes the sleeping native producer and sends the original pending identity.
No owner terminal, app, provider or live backlog action is involved.

## Allocated resolution

Task decisions039254bf/c2486954 allocated the narrow subscription and evidence.
Implemented explicit key comparison including stamp.pid (ordinary TaggedPane Eq
ignores it), TTY, tagged ids, pane id, life/death, health and foreground command
as a readiness hint. No output/title/geometry event. Publication releases both
inventory locks before notifying. Sender subscribes before initial draining,
retains the watch in its already owned task, and never calls refresh itself.

`inventory-native-restored.log` passes six native/metadata tests and saves actual
recovery-only queue argv. The new test starts a fresh unavailable LiveInventory
against the same private native server; after the sender sleeps, a real refresh
alone wakes it and submits the original pending id. The entire terminal row is
asserted unchanged. Three more unchanged refreshes preserve watch generation and
produce no sender-idle iteration or extra queue bytes. No timer or artificial
Store write supplies the recovery wake. Final negative/restored evidence follows
in the final evidence manifest.
