# Actual writer owners and allocated regions

Anchors without a pin are e853d9c4. This is a bounded source inventory, not a
claim that installed processes have been enumerated or stopped. Every actual
owner found later must enter the same closed inventory; absence of source proof
blocks transfer. Source hashes and exact pins are in source-inventory.json.

| Writer / actual source | Existing owner and missing proof | Future bounded region owner |
| --- | --- | --- |
| Normal daemon startup/shutdown, main.rs:230-339,446-476 | Main retains check_in_task and joins it; Service stops workflows. Watcher, CI/PR/review/landing readers and paste sweep are spawned without a common retained join. Socket shutdown alone does not join their in-flight writes/children. | ov-940 integration after allocation: main.rs + Service ownership fields; reuse937 root selection |
| Stream/standalone stdio,937 main.rs:483,612 | Separate Service opens; standalone spawns unretained Watcher. Not covered by normal exclusive bootstrap. | ov-940 main/shared Service admission; preserve transport/auth |
| Store normalization/direct writers, store.rs:57-123; engine_cutover.rs:160,386; workflow/effects/wire writes | Store owns connection mutex and captured engine_incarnation, not external processes. open_for_cutover rejects below65. Old Store handles, second connections and admitted claims need same transaction incarnation/freeze checks. | corrected936 S1/S2 exclusively fresh936 fixer; later ov-940 Store integration explicit handoff |
| Legacy scheduler, watch/answer_wake/check_in.rs:71-122 | Main owns watch stop channel + JoinHandle; active bounded send joins on normal shutdown. Check-in DB reservations/cleanup/Enter/target facts are retained, not proof from PID text. | ov-940 reuse existing stop/join and prevent fresh recovery/admission before gate |
| Watcher progression/line work, watch.rs:4469-4526; watch/workers.rs:163-200; collecting_scan.rs:35 | Watcher loop spawns pumps and spawn_blocking collect_ready_tickets; worker follower mutates work lifecycle. Stopping ticker alone misses spawned descendants and blocking tasks. No dedicated line-job join API established. | ov-940 Watcher::run/spawn_pumps/worker/collecting admission and retained handles; no invented line owner |
| Landing job, landing_run.rs:64-113; landing_job.rs:160,550; green_landing.rs:84 | Running::take is a workspace exclusion guard, then tokio::spawn detaches job. Job creates scratch worktrees, Git/push/gate/CI effects. It is not an owned cancellation/join receipt. Need exact child ownership through external operation and reconcile uncertain push. | ov-940 later landing_run/job spawn/admission closure only; retain landings/timeline |
| Hook and agent callbacks, service.rs:3926/3942; hook_ingress.rs:615,725; Watcher::new :2745 onward | Listener/followup callbacks and hook work may write after startup. Service/HookIngress own registries; detached spawned work is not joined by a DB fence. Close new callbacks, drain actual work and retain observation-only events. | ov-940 Service/HookIngress finite stop/drain boundary, no provider lifecycle expansion |
| Manual/automatic delivery, answer_wake.rs:381 onward; compose/{held,pending}; bring/finish/interrupt/mid_turn | wake_pump lock serializes passes but is not a cutover stop/join. pump merges answer/hold/message queues. Spawned compose/paste/Enter work and current target reservations must finish or remain uncertain. Actor/queue kind alone cannot distinguish authorized new manual work. | ov-940 retained bridge at Service/RPC + Store messages/wakes + existing delivery paths after explicit allocation |
| Workflow owner, workflow_service.rs:5-43; workflow_runtime.rs:481-513 | Service Owner retains stop, task, notices handles; stop_workflows joins both. Runtime inbox/mutations/pump/effects can be admitted before stop; retained handles and actual resource locks must bind proof, not just stop result. | ov-940 workflow_service + runtime admission;939 owns Review/reason mapping |
| Configured/gate effects, effect_runner.rs:49-88; workflow_runtime/{coding_execute,coding_landing,adapters,service_owner} | Spawned child/group, repository/configuration locks and resources belong to Runtime. Current supervise awaits child.wait before later killpg and accepts any nonzero signal0 as gone. Approved938 did not change this algorithm. Cannot certify it by reuse of Group name. | ov-940 required finite extension of reviewed owned-group ordering at this real writer, subject to source review; no host-wide process framework |
| Native workflow queue,938 workflow_notices.rs:60-145 | OwnedQueue attaches Group synchronously in Store admission. WNOWAIT pins leader; one disarmed signal, bounded reap + ESRCH required. Generic Unknown/error/drop is not a clean cutover certificate. Prepared attempt survives. | reuse938 unchanged semantics; ov-940 wire actual clean/unknown result into owner join evidence |
| Native Store admission, store/workflow_notices.rs:103,160 | Immediate preparation/admission transactions check target fingerprint, but omit engine freeze/incarnation. Candidate936 reserved-notice patch is a proposal only. | ov-940 exact reserved-region reconciliation with936/938 before code; check engine_allowed at both transactions without fallible post-spawn Store call |
| CI/PR/review/landing policy observers, main.rs:344-359; ci_watch/pr_watch/pr_review/landing_read | Observation loops also write DB/history and may drive green landing; their names do not prove read-only. Retain/join writes and any job they schedule before raw capture. | ov-940 caller closure against exact composed blobs; display/read retained, progression retired |
| Paste expiry/config/log/registry/session/resource helpers, main.rs:296,365;937 service/runtime_selection/watch | Background helpers may alter backup resources even without task authority. Registry watches need private CLAUDE_CONFIG_DIR in fixtures. Stop/join resource writers or exclude only with explicit reviewed nonresource proof. | ov-940 finite resource inventory; no app/OS/provider actions in prep |
| RPC/direct Store/task/terminal lifecycle, task_ops/rpc_landings/service.rs; Store tasks/waits/ticket_lines/check_ins | Requests and cascades can mutate status/waits/identity or start/relink task-bound terminals outside scheduler loops. Server shutdown must close admission and join active requests; triggers remain defense in depth. Fresh task content/read/history and authorized manual communication survive. | ov-940 named caller closure; main/native/platform guidance closure932/parent |

The seven synthetic categories are an evidence index, not seven sufficient
handles. In particular line-jobs includes Watcher worker/collecting operations,
delivery includes compose descendants, workflow-owner includes configured and
landing resources, and legacy-hooks includes in-flight callback tasks. Auxiliary
DB/resource writers must also be covered. Keep per-instance/resource evidence,
not one success string per category. No recorded PID discovery or signaling is
authorized now or accepted later as replacement ownership proof.

Only reports/ov-940 is writable in phase1. Above assignments describe necessary
future boundaries, not grants to edit them. Fresh936 owns S1/S2; fresh939 owns
CLI destination binding. On exact corrected-base allocation reconcile main.rs,
service.rs, effect_runner.rs, workflow_notices.rs, Store notice/open/cutover and
939's unchanged-by940 wire_commands/types. Any overlap gets one explicit writer;
do not edit sibling trees or silently substitute manager summaries for blobs.
