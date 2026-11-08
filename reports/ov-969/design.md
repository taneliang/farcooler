# ov-969: who decides each gate

Item 4C of the ov-964 answers: a review gate is a step, and each gate names who decides it.

## Where things stood

`StageKind::Review(Reviewer)` already had three deciders:

- `Dispatch`: runs the configured reviewer command. The command writes a receipt with `passed`; with no receipt the step can't settle. It isn't an agent with a model.
- `Orchestrator(p)` and `Person(p)`: the run waits in `WaitingReview` until principal `p` sends `Review` with the step's evidence. Nothing else can pass it: `Approve` doesn't apply in that phase, and a controller grant is refused.

## What changes

**A fourth decider, `Reviewer::Agent`.** The engine starts a real agent for the gate, the same way it starts a native build:

- It runs in a managed terminal in the run's own tree, and the hook socket reads its turn.
- It gets a fixed review brief naming the ticket, the base and the candidate.
- The model comes from the workflow's allowed list (`models`), so the orchestrator picks it at Start, as it does for a build (ov-966). With no list, the agent uses the configured preset.

**The agent's verdict is a file, never its prose.** The agent runs `farcooler workflow verdict --passed true|false`. That command writes the step's receipt at `$FARCOOLER_RECEIPT`, bound to `$FARCOOLER_OPERATION_BINDING` and `$FARCOOLER_CANDIDATE`, with identity `agent`.

The runtime trusts the receipt only if:

- the binding and candidate match,
- the tree is still clean at the candidate, so a reviewer can't edit what it judges,
- the turn ended cleanly.

**No verdict means no pass.** If the turn ends without a receipt, the result is `SafeFailure`, not a pass and not a rejection. The step retries within `max_attempts`, then the run fails. A rejection only comes from an explicit `--passed false`.

**Pinning.** An agent gate pins `coding-review/v1` with the argument `agent`. It needs a native harness. It doesn't need a configured reviewer command.

## Not changed

- **Landing** is already a step (`Land`, or `Push` in a group) with no agent. A `Person` gate before it is how a person approves landing.
- **Starting an in-review ticket at a named step** builds on adopting a branch (3A, ov-968). ov-968 is rewriting `Start` right now. I'm leaving this to a follow-up on top of ov-968, so the two lanes don't edit the same reducer arm at once.

## Wire compatibility

Adding a variant to `Reviewer` leaves every existing definition's encoding and pin unchanged. A client older than this change can't read a definition that uses `Agent`. That only matters once a board publishes one.
