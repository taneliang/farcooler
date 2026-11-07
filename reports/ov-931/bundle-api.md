# Configured workflow publication API

Shared types: `farcooler_protocol::workflow_config`; command vocabulary:
`farcooler_protocol::workflow_commands::Action`. Coding configuration is a
closed `coding/v1` tagged value. The optional boxed Rust field changes no JSON.

The existing command accepts a bounded JSON file:

```text
workspace set-workflow <workspace> --file <file>
  --command-id <stable-id> --revision <observed-revision>
  --epoch <observed-epoch> --sequence <sequence> [--expected-pin <current-bundle-pin>]
```

Coding file envelope:

```json
{
  "definition": { "schema": 1, "source": "adapted-local-policy/v1", "stages": [], "max_reworks": 0, "rework_target": null },
  "adapter": { "version": "coding/v1", "config": {} }
}
```

The empty structures above show the envelope only. Supply a valid pure Definition
and the exact CodingConfig fields in `crates/protocol/src/workflow_config.rs`:
repository, base, landing_ref, optional landing, harness, checks, optional reviewer,
and CI identity/command. Commands pin argv, environment and timeout. Native launch
files also pin their digest; landing pins its local bare repository/config digest.
No wire field conveys a principal, authority grant or effect result.

`CodingConfig::pin()` is the configuration identity embedded in each supported
operation. `CodingConfigExt::definition()` in the daemon remains an optional
archetype; adapted IDs, policy, multiple checks and rework destination are valid.
The pure engine's built-candidate prerequisite remains authoritative. Adapter
validation additionally requires a provisioned worktree, exact supported operation
bindings and landing before CI; it permits post-landing checks. Rework after
irreversible landing is refused. Checks and agent review may be omitted for fast
ship, and approval progression remains configurable on every stage.

`workflow get <workspace>` returns `configuration.pin`, the immutable **bundle**
pin. Pass that to `workflow enqueue --pin`. Publication CAS `--expected-pin` also
uses that bundle pin. An item's `policy_pin` retains it across reconfiguration,
restart, rework and effect reconciliation. The item's separate `pin` identifies
its pure Definition; approval/cancel/review commands retain that existing definition
pin/run/evidence contract. Replaying a command requires unchanged arguments and
identity; fresh configuration must use a fresh command ID.

Definition-only files remain supported for the minimal no-Git tracker. Ordinary
Service refuses coding operations without versioned adapter configuration. Only
existing server-authorized person publication is allowed; enqueue/decision
principals continue to come from authenticated Session/caller admission.
