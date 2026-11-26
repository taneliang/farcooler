# ov-1480 workflow and terminal writer verification

The Linux CI shard exercised the pane fixture and hit a startup race in which the stand-in's seat changed from `Person` to `Agent` during a post-proposal read, and the passing macOS landing check covered the build-free quick gate and targeted suites that omitted the fixture. [CI attempt 1](https://github.com/taneliang/farcooler/actions/runs/45795667586/job/139827713806) · [CI attempt 2](https://github.com/taneliang/farcooler/actions/runs/45795667586/job/139842114609) · [ov-1454 landing report](../ov-1454/README.md#landing-gate-repair)

## Root cause

Nextest counted one failure per CI attempt. The standalone pane entry exits
when its pane environment is absent. The enclosing test starts it inside tmux,
where it logged `proposed` and panicked with `authentication required` before
`published`. The 60-second line was the slow-test notice. The assertion in
`common/mod.rs:242` reported the detached test's failed exit status.

On fcdiag with CI's tmux 3.4, temporary diagnostics held the first nonempty
inventory publication until the post-proposal Get had been admitted. The
original fixture reproduced both CI errors with its original startup sequence
and without taking an input lease. The trace showed the same caller PID
throughout.

```text
admitted workflow.get     pid=263198 orchestrator=None
admitted workflow.propose pid=263198 orchestrator=None
admitted workflow.get     pid=263198 orchestrator=None
first live inventory publication: pane PID 263174
admission seat recheck: pid=263198 captured=Person now=Agent(Some(...))
workflow.get: authentication required
the orchestrator stopped short: Ok("proposed\n")
```

`Admission::recheck` refused the change in seat. The original CI logs report
the wire error. The controlled run identifies the check by reproducing both
symptoms without injecting an error. On macOS 27.2.1 with tmux 3.8, 10 of 35 runs of the previous
ticket fixture with admission tracing and without the read retry captured
the same `Person` to `Agent` transition. The race depends on startup ordering on both platforms.

The fixture enables test-only caller attribution, which can admit its
unregistered stand-in as a person. In the Linux comparison, disabling that
stub and running the same stand-in binary under the recognized agent name
`claude` made early calls fail the `known caller` check. Once the pane appeared,
its first admitted Get and its proposal both carried the orchestrator seat,
and the ticket landed. The deployed workflow CLI uses the same socket
admission path with test attribution disabled. The writer lease governs PTY
input.

The fixture now starts its workflow RPCs after the owner receives the pane's
writer lease. Its first reply must identify the caller as `orchestrator`.
The owner keeps that connection open through landing. An authentication
refusal fails immediately and records the method that was refused.

## Verification

Both runs exercised `engine_flow_per_skill`, `rpc_over_socket`, and
`stdio_transport` with CI assertions and the repository's leak checks. The enclosing workflow
test checked proposal, publication, move, start, review, queue, and the landed
Git commit while the owner held the writer lease.

| Host | Result |
| --- | --- |
| fcdiag, Ubuntu 26.04 ARM64, tmux 3.4 | 105 passed. Nextest run `b8cde211-532f-4c9c-aa21-9b8353b1106f` |
| macOS 27.2.1 ARM64, tmux 3.8 | 105 passed. Nextest run `327242e0-16e0-4a59-8910-82ebb1f1b029` |

Both runs included ov-1454's five live regressions for exclusive typing,
claiming a vacant lease, notifying the previous native writer on takeover,
sharing a lease across stdio sessions, and retaining the writer after the
starting SSH connection closes.
