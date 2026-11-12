# Prospective lane-gate evidence (ov-903)

Status: implementation and fixture validation only; independent review is required
before any real gate uses this protocol. Landing 369 gate4 is v1 and cannot be
imported, upgraded, reconciled, or resumed.

## Contract

`scripts/gate_evidence.py` is an opt-in runner using `lane-gate.py`'s canonical
job selection and step translation. Legacy gate execution and its v1 push
contract stay unchanged. V2 has no filters, environment-gap success, waiver, or
manual pass input. A failed job is retried in full so its environment-producing
steps run again. Only complete, successful jobs from an identical captured scope
can be reused. A run interrupted before sealing is unusable.

At run start require clean tracked, index and nonignored untracked inputs, and
capture HEAD, tree, resolved base and merge base, index/status state,
content/modes of all tracked files, the complete canonical
plan (including declared CI skips), runner implementation, selected public
environment and explicit prerequisite fingerprints. Refuse drift at completion.
HEAD is sampled at both boundaries; finish-time HEAD cannot substitute for start.
Dirty/untracked inputs are refused even if they would match a saved snapshot:
an uncommitted fix cannot attest the committed SHA that would be pushed.
Ignored build outputs are outside the source snapshot and must be declared when
they are inputs. Native outputs built by the job are not prior input evidence.

Execute with a restricted environment rather than saving arbitrary ambient
variables. Never collect credentials, arbitrary environment dumps, global config
files, or secret files. Explicit prerequisites are public SDK/toolchain files,
immutable cache inputs and native dependencies, selected by a reviewed input
manifest. Missing prerequisites fail before the heavy queue. Fingerprinted
prerequisites must remain unchanged; mutable cache directories are unsuitable.
Primary tools are mandatory per job, including resolved rustup/Apple compiler
binaries. Android pins its effective NDK to the package parsed from CI, refuses
a conflicting override, and fingerprints the discovered wrapper/compiler/archiver,
CI SDK files and selected JDK executables/modules. Toolchain version metadata
alone is not a complete dependency inventory.

Every attempted step has its canonical index, full shell command, working
directory, effective public environment, numeric exit (or unknown), and log
digest. A skipped step has its canonical skip reason and no execution claim.
Commands/environments normalize the per-run temporary directory for plan matching;
the bound `runner_temp` field reconstructs literal commands and `GITHUB_ENV` /
`GITHUB_PATH` locations. Known exits remain numeric even when environment-output
parsing fails; a separate protocol error makes that attempt red.
Unknown/unexecuted steps cannot pass. Preserve earlier failed attempts in a
parent chain, including the original summary and every output log. A new attempt
uses a fresh private directory and never overwrites a parent's files. The latest
attempt for a job determines its result; an older green cannot cover a newer red.

The v2 summary is a pointer plus digest, not an assertion of success. The push
checker revalidates the entire chain, current source/plan/environment/prerequisite
identity, and exact row coverage. A missing parent, log, summary or receipt is red.
An exclusive per-worktree capture lock prevents overlapping writers; an atomic
pending pointer replaces the previous pointer before execution. SIGKILL/unsealed
attempts remain red after lock release. The persistent worktree-local lock file
also marks v2 activation: missing pointers and legacy v1 downgrades cannot use
legacy success/waiver handling after an attempted v2 run. No global configuration
is edited. `--plan` does not activate v2 or execute any CI steps.

## Invocation and prerequisite declarations

After independent source review **and review of the concrete input declaration**,
on a new reviewed clean exact SHA, the prospective flow is:

```sh
python3 scripts/gate_evidence.py --worktree /path/to/checkout --base BASE_SHA \
  --inputs /private/path/public-inputs.json --plan
python3 scripts/gate_evidence.py --worktree /path/to/checkout --base BASE_SHA \
  --inputs /private/path/public-inputs.json --run --evidence-dir /private/path/run-1
```

To retry, supply `--resume /path/to/saved-v2-pointer.txt` and a fresh evidence
directory with the same base, inputs and options. Copy the pointer before the
next attempt if needed; every attempt also privately archives its preceding
pointer. `--resume` accepts only a sealed v2 pointer. Failed/incomplete jobs are
derived automatically; there is no `--only` or `--skip` option.

The input file has schema `1` and exactly the jobs selected by the canonical
lane planner. Each job has `tools`, `sdk`, `cache`, and `native` string arrays.
`tools` lists additional executable names/paths; the other arrays list explicit
public **files**, not directories, credentials or global configuration. Mandatory
tools are fingerprinted even if absent from the declaration. Every heavy job
requires nonempty SDK, immutable cache and native file inventories. These must
cover relevant public dependencies; a placeholder file in each category does
not satisfy review. Light jobs may use empty inventories where appropriate.
For example, a tiny light-job fixture declaration is:

```json
{"schema":1,"jobs":{"wire":{"tools":["sh"],"sdk":[],"cache":[],"native":[]},"doc-comments":{"tools":["sh"],"sdk":[],"cache":[],"native":[]}}}
```

This is a fixture example, not a declaration approved for real repository jobs.
Preflight also invokes read-only compiler discovery/version queries when selected
jobs need them; it never installs SDKs or writes global settings. The v2 runner
keeps build outputs and does not trim or clean `target/`.

## Trust boundary

These are private local runner receipts, not signed remote attestations. Hashes
detect accidental edits/replacements relative to the bound summary, not a
malicious owner who can rewrite the entire evidence graph and push guard. The
runner trusts checked-out CI/scripts and the reviewed public input declaration.
Independent source review does not approve a real input declaration. Each real
declaration needs separate independent review for complete relevant toolchain,
SDK, immutable cache and native dependencies. Undeclared ignored inputs or
changed global configuration invalidate authorization to reuse results even
where the checker cannot discover them automatically.
It does not claim hermeticity: HOME-based tools can consult their normal config,
network services can change, and changes that happen and are restored between
boundary checks are not detectable. Global configuration must stay stable under
operator control; it is deliberately neither read nor archived by this tool.
Do not reuse evidence across machines/checkouts, changed configuration, or
unreviewed prerequisite declarations. Logs may contain sensitive program output;
keep evidence private and do not commit or upload it automatically.

## Source and verification plan

1. Add bounded v2 writer/validator, canonical planner integration and CLI.
2. Add v2 dispatch to the push checker, retaining v1 behavior for legacy runs.
3. Use temporary repositories and tiny real shell commands for failed-job retry,
   complete coverage and raw exits. No application compilation or heavy job runs.
4. Exercise stale SHA/base/CI/source/index/untracked/environment/prerequisites,
   partial rows, unknown/nonzero exits, log/parent tampering, v1 rejection,
   later-red-over-earlier-green, and immutable prior files.
5. Mutate coverage/identity guards in disposable source copies, observe tests go
   red, then validate the unmodified source. Run existing lightweight runner and
   checker self-tests. Independent validation reviews both code and fixtures
   before use; prospective capture starts only on a new reviewed exact SHA.

## Source validation record (2026-11-04)

- `python3 scripts/gate-evidence-test.py`: 16 passing lightweight fixtures;
  independently rerun and passed. The fixture CLI calls the real writer/checker,
  executes tiny shell producers, and never queues or builds application jobs.
- `python3 scripts/gate-evidence-mutation-test.py`: four meaningful assertion
  reds observed in disposable copies for partial coverage, exit-7 promotion,
  missing prerequisite fingerprints, and trusting stat/status over tracked bytes.
- Existing `lane-gate.py --self-test` and `gate-summary-check.py --self-test`
  pass. Temporary fixture commits disable signing per invocation/repository;
  no global settings are changed.
- Independent source review found and drove fixes for dirty/hidden tracked
  inputs, v2 waiver leakage, interrupted-current-pointer handling, malformed
  environment-output receipt loss, selected NDK provenance and shared historical
  DAGs. Final review reported no blocking findings within the stated boundary.
- The final shared-lock wrapper was also checked with a focused counterexample:
  an exclusive capture writer cannot acquire the lock during verification.
  If first activation happens during an initially unlocked legacy check, discard
  that result and repeat under the new lock; its focused fixture also passes.

No real v2 gate, application build, target cleanup or landing-369 evidence reuse
was performed. This record approves source/fixture review only; each concrete
public prerequisite declaration and prospective execution still needs review.
