# Engine: close a failed run as landed (ov-1231)

Adds `workflow landed <workspace> <item> --run <run> --pin <pin> --sha <commit>`.
The orchestrator uses it on a `Failed` run whose work reached the target branch
another way. The runner reads the remote first. If the commit is an ancestor of
the target there, the run closes as `Done` with that commit, the plan shows it
as landed, and the ticket leaves Needs You.

## Every phase the verb can be called in

The reducer (`Command::Land`) checks the caller is the orchestrator, then the
phase. Only `Failed` is accepted, and only with the extra conditions below.

| Phase | Result |
| --- | --- |
| `Failed` | Accepted only for an eligible run (no pending effect, not a group, not a follower, not a ticket at rest) with a full 40-hex SHA that the remote read shows on the target. The run becomes `Done`, the SHA is recorded as landed, and the failure is cleared. |
| `Blocked` | Refused; the run is not closed. |
| `WaitingApproval` | Refused; the run is not closed. |
| `WaitingReview` | Refused; the run is not closed. |
| `Running` | Refused; the run is not closed. |
| `Uncertain` | Refused; the run is not closed. Only a person or `Resolve`, with evidence, settles it. |
| `Cancelling` | Refused; the run is not closed. |
| `Done` | Refused; the run is not closed. |
| `Cancelled` | Refused; the run is not closed. |
| `WaitingJoin` | Refused; the run is not closed. |
| `Joined` | Refused; the run is not closed. |

Other refusals, all leaving the run untouched:

- Archived or missing item: refused.
- A group, or a ticket that follows another run (it lands with its lead): refused.
- A SHA that is not 40 hex characters: refused.
- A commit not on the target branch, or a remote that cannot be read: refused.
- A caller who lost authority while the remote read was pending: refused (the
  authority is rechecked after the read, with no await before the mutation).

## Tests

Reducer, CLI, projection and daemon `landed` tests, including the real
`Inbox::Wire` admission path with authority withdrawn mid-read.

## Docs

`docs/engine-hooks.md` names the verb, and `workflow landed --help` documents it.
