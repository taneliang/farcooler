# CLI refusal-only delta for SAME review

This signed commit is stacked directly on frozen821194faedd23fec2b639667034b6a8ba979f827.
Manager d8b34fd7 selects the refusal-only disposition in SAME review addendum
SHA-2568512335e424ba28a7fc35c5cf5a64dbfbce06a149e3c105b9aea4869080b0f58.
It supersedes the CLI retry behavior/claims in the original frozen handoff;
original source evidence/reports remain unchanged for audit.

The only changed product path is crates/cli/src/workflow.rs. workflow delegates
to private workflow_with_connect with a lazy closure over connect_to(runner).
Retry returns unsupported-recipient before file access or closure invocation.
This applies to local/default and remote routes, every schema/file content and
missing paths; it never auto-starts a daemon, launches SSH or sends a mutation.
No aliases, install IDs, PID or synthetic identity are accepted as authority.
The original prepared file and exact reason/identity/inner JSON are untouched.

Initial explicit Rework remains the existing first attempt: validate/serialize,
create-new0600/nofollow, sync file+parent, connect, send retained JSON. It now warns
before connecting/sending that durable CLI retry is unsupported and an uncertain
outcome requires inspecting the original runner state, without issuing a fresh
command. Rework/Retry help carries the same uncertainty warning. The loader is
test-only; shared prepared schema/request validation and first-send bytes remain
unchanged. The older test named prepared_retry verifies serialization only, not
a working durable recovery route.

## Focused proof

* rust-red.log/json: old Retry branch, with only private connector extraction and
  tests added, compiles and exits101 on three intended assertions. Local and
  remote connector counters each become1 instead of0; help lacks the unsupported
  recovery warning. Connector returns an error without constructing Link, so
  actual daemon/SSH/network/sender execution is impossible even in this red.
* old-retry-with-private-seam.txt is the full exact red source; control.json
  records its byte-exact restoration before the correction. corrected-source.txt
  and red-to-fix.diff bind the only subsequent behavioral/help changes.
* rust-green.log/json:7 selected CLI workflow tests pass, including both new
  no-connect controls, malformed/unbound-schema/missing-file refusal with byte
  preservation, honest help and four existing producer/file regression tests.
  Zero connector calls imply zero mutation calls because send requires Link;
  no real sender is mocked into claiming receipt or lost-reply recovery proof.
* rust-lint.log/json: CLI-only all-target Clippy -Dwarnings exits0. Dependencies
  compile as required, but no other packages' test suites or native/Web checks run.
* light-checks.json: product whitespace, existing file-size and doc-comment
  rules exit0. Original raw red stdout/help whitespace remains verbatim.

The three runs use canonical normal FIFO, original923 target with actual
post-lock Cargo-holder/profile/source proof, native arm64 Python, jobs6,
dev/test-debug0, incremental0 and offline/locked. preflight.jsonl,
cache-holders.jsonl and the final post-lock tool-identities.jsonl retain actual
observations. Supervisors enforce1800s and sampled10GiB reserve; all completed
without cancellation, child reaped and owned process groups absent afterward.
verification-summary.json gives exact argv, exits, duration, sampled reserve,
source-hash equality and limits. No owner config/data/keys/provider/GUI actions,
network/real runner, global settings, cleanup or other-tree source edits.

scope.json checks all25 original product entries:24 unchanged and this sole CLI
delta; every original reports/ov-939 tracked byte matches821194. Check helpers
write only into this new report directory, preserving original accepted evidence.
evidence-sha256.json seals all files here except itself, plus the changed source.

## Open acceptance

This correction is ready only for SAME refusal-only delta/evidence review. It
closes neither durable CLI retry nor full Feature B acceptance. ov-941 explicitly
owns the missing authenticated destination/recovery capability. The prior shared
and native/Web closures stand; their46 Rust passes/1ignore and native results are
reused rather than repeated or re-counted. Full root/app/device navigation, true
feature exercises, composition and release evidence remain parent/ov-932 scope.
No same-destination CLI lost-reply recovery is claimed by these refusal tests.
Stop product changes at this signed pin for the allocated SAME review.
