# Light review for small tickets

A schema-2 coding definition can put a `Route` step immediately after `build`.
Use `coding-route/v1`, with the adapter pin as argument 0 and this JSON as
argument 1:

```json
{"under_lines":100,"report_paths":["reports/**","docs/**"],"full_paths":["private/**"]}
```

Before enabling this route, approve `security_reviews` in the coding adapter
configuration. It pins the existing full sequence by step id, allowed models,
and exact review-options text. For example, a full gate with no options:

```json
{"security_reviews":[{"step":"full","models":["claude:opus"],"options":null}]}
```

The adapter pin covers this policy; definition authors cannot redefine it through
route labels. Admission checks every graph path to joining, landing, branch
publication, or completion. Each path must pass the entire pinned sequence in
order, or carry a `light` classification from the diff classifier. Legacy
`PathRules` provide no such exemption. Rebuilding resets both review coverage
and the classification. A missing or changed full gate, a misrouted `full`, or a
bypass route is refused before any agent launches. Adapters without this trusted
policy retain their existing behavior and cannot enable `ReviewRoute`.

Route `light` to one agent review with a fast default in `definition.models`
(for example, `claude:sonnet`), and `full` to the existing full review sequence.
Set the light step's `max_attempts` to 1 for a single launch per candidate.
Route the light review's `passed` directly to `join`, skipping the full reviews;
route rejection to `Fail` or the existing bounded build rework edge. Each new
candidate goes through the route again, including after rework. The definition
chooses the threshold, light-review model, and rework budget; the adapter pins
the required full review sequence.

The threshold is exclusive and counts additions plus deletions across the
base-to-candidate diff. Renames count as deletion plus addition. A change is
report-only when every changed path matches a report glob. Report paths can
qualify regardless of size. Binary changes always take `full`.

Security takes precedence over size and report paths. The runtime always sends
Far Cooler's implementation trees (`apps/**`, `crates/**`, `services/**`,
`proto/**`, and `scripts/**`) to full review, including their tests. It also
protects automation and build configuration (`.github/**`, `.cargo/**`,
`.config/**`, `.claude/**`, `.codex/**`, `.agents/**`, `Cargo.toml`, `Cargo.lock`,
`Cross.toml`, `typeshare.toml`, `.gitmodules`, and `.gitattributes`). This broad
repository policy avoids guessing which implementation files handle credentials
or cross trust boundaries. Small changes outside these paths can take light
review; report-only changes can qualify regardless of size.

`full_paths` must contain at least one glob and adds protection to that mandatory
policy. The example adds `private/**`. Omitting a mandatory path or matching it
with `report_paths` cannot downgrade its review. Binary changes also take full
review. Unreadable Git output fails the route rather than reducing review.

To overlap a ticket's gate and review, add `check` to the agent review's third
operation argument (the existing review-options JSON):

```json
{"prompt":"Read the lane rules.","check":0}
```

The index names a check in the pinned coding adapter, validated when the
workflow is published. Remove the corresponding sequential ticket check step.
Both processes start in the candidate worktree and must finish before the
review step settles. A passing reviewer cannot override a failed check. Their
separate process receipts are bound into the successful evidence. Recovery
reads both receipts; an interrupted gate's owned process must end before retry
or cancellation can release the ticket. Commands must allow concurrent reads
of the candidate; use the landing group's existing gate for checks that edit
source files. The landing group's gate still judges the folded result.

## Measuring

The numbers below are an artificial scheduling experiment, **not** the requested
real before/after outcome. No production definition in the inspected event
store has enabled light review or concurrent ticket checks, so there is no
after cohort to measure yet. A real comparison remains pending publication and
completed runs. See `reports/ov-1194-stage-timing.md` for the observed baseline
and the reproducible extraction command.

Run the controlled runtime experiment:

```sh
python3 scripts/heavy -- cargo test -p farcooler-daemon --lib light_review_stage_timing_before_and_after -- --ignored --nocapture
```

It runs five serial and five parallel samples with native-agent and gate
stand-ins doing 500ms of work apiece. It subtracts the first post-build effect's
`created_ms` from the observation timestamp that enters `WaitingJoin`, using
the stored workflow stage events. Setup and build are excluded. This isolates
scheduling overhead; it does not estimate live model latency or replace a
production before/after cohort once the new definition is published.

Measured on November 14, 2026, with the command above (milliseconds):

| Scheduling | Five samples, sorted | Median |
| --- | --- | --- |
| Before: sequential ticket gate, then review | 1766, 1800, 1819, 1901, 2082 | 1819 |
| After: ticket gate alongside review | 985, 1084, 1096, 1216, 1281 | 1096 |

The simulated median fell by 723 ms (39.7%). Both variants use the same single fast-model
review definition; this measures the overlap, without attributing any simulated
latency benefit to a model change.
