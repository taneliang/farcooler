# ov-1087: blocked, no bundle built

The bundle has to be the live definition with only the model lists changed. I could not read the live definition:

- `farcooler-canary workflow get <workspace>` returns `scope denied: known caller required` for this engine agent.
- `workspace workflow-draft-status` is refused the same way.
- `workspace show` has `landing: null` and no definition.
- The only other copy is under the app's `workflow/` directory, which the ticket forbids touching.

A bundle rebuilt from `BUILD_MODELS`/`REVIEW_MODELS` in `crates/daemon/src/workflow_starter.rs` would not be "the live definition unchanged", so I did not make one.

## What is needed

Someone with a known caller (the owner or the orchestrator) saves the output of
`farcooler-canary workflow get d7401571 --json` for the Main workspace, and hands this ticket the definition. Then the work is:

1. In `models.build` and `models.gate-0`, add `codex:gpt-6.1-sol`, `codex:gpt-6.1-terra`, `codex:gpt-6.1-luna`; in `models.build` also add `claude:haiku`. Leave every other field alone.
2. In a scratch daemon (short `FARCOOLER_HOME`), run `workspace set-workflow` with the bundle, then `workflow start --model build=codex:gpt-6.1-sol`.
3. Publish with the command below, in the owner's shell:

```
farcooler-canary workspace set-workflow d7401571 --repo overnight \
  --file .claude/agent/reports/ov-1087/workflow-codex.json \
  --expected-pin wf1:13df7232… \
  --command-id <fresh-uuid> --revision <R> --epoch <E> --sequence <S>
```

`<R>`, `<E>`, `<S>` come from the same `workflow get` output: the state's current revision, epoch and next sequence. The `--expected-pin` is the full current pin (the ticket gives only its prefix).

## Re-checked (second pass)

Still blocked. `workflow get 01a0e3c7-3e2c-7ed0-b31c-0cc0d7401571 --json` (the full id; the 8-character short id is rejected) still returns `scope denied: known caller required`. The workspace home holds only `charter.md`, no definition. No acceptance line is met; the ticket should stay open until someone with a known caller supplies the definition.
