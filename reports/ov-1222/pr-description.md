A failed batch gate now attributes paths in failed-command output to the member
candidates that changed them. The engine returns those members to build with the
failed checks and error lines in their brief, then folds and gates the remaining
members together. If no member can be identified, the published half-split route
still applies; its singleton rejection also carries the diagnostic brief.
A cancelled group returns an unrouted `dropped` member to waiting at its join,
retaining its candidate so the orchestrator can queue it again. An explicitly
published `dropped` route still decides as written.

Attribution reads the process receipt's digest-verified output bytes and compares
NUL-delimited Git paths against each member's own pinned base and candidate.
Lane-gate's changed-file header and passing commands are excluded. Diagnostic
briefs are bounded to 4096 UTF-8 bytes; the full receipt remains available.
`test/fixtures/landing-gate/nov14-group1781.output` is byte-identical to the failed
Nov 14 canary receipt, with provenance beside it.

Lifecycle states touched (retained reducer names):

- Failed gate group: `Running` (or `Uncertain` when a controller settles the
  authenticated result) stays in the same group and restarts at its published
  split target, normally `AwaitBase`: `Running`, or `WaitingApproval` if that
  target requires approval. Removing members bounds retries without consuming
  review/back-edge budget. If every member is ejected, the empty group becomes
  `Failed` and releases the landing resource.
- Diagnosed member: `Joined` -> `Blocked` at its first `Build`, detached from the
  group, candidate cleared, diagnostic brief retained. Ordinary readiness then
  gives `Running` or `WaitingApproval`; a busy resource leaves it `Blocked`.
  A definition without `Build` uses its existing rejection route instead.
- Undiagnosed member: remains `Joined` in the same group. A successful retry
  follows the existing passed/landed join routes.
- Anonymous failure: existing split transitions are retained: original group
  restarts, new sibling waits `Blocked` for the landing resource; both contain
  `Joined` members. A singleton/non-splitting fallback can become `Failed`;
  member rejection routes still decide `Blocked`/`Running`, `WaitingApproval`,
  `Failed` or `Done`, with the brief retained.
- Cancellation: active group `Running` -> `Cancelling` -> `Cancelled` after its
  stop receipt (a waiting `Blocked` group can cancel immediately), then existing
  archival. Members stay `Joined` until cancellation settles, then become
  `WaitingJoin` at the same landing step, detached and with candidate retained.
  A subsequent explicit Queue changes them to `Joined` in a new group.

No schema migration or published-definition rewrite is required. ov-1213 is in
this branch's ancestry. No stash, reset, amend or cargo fmt was used.

Validation results are recorded in `validation.md` beside this description.
