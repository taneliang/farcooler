# ov-939 CLI refusal correction

Manager answer d8b34fd7 supersedes identity persistence for this fix. SAME review
addendum SHA-256 8512335e424ba28a7fc35c5cf5a64dbfbce06a149e3c105b9aea4869080b0f58
confirms the CLI Link has no durable authenticated recipient identity.

Only crates/cli/src/workflow.rs changes. workflow delegates to a private
workflow_with_connect closure seam; production still uses connect_to(runner).
Retry must return an explicit unsupported-recipient error before loading a file
or invoking the connector, for both local and remote. No connection implies no
auto-start, SSH or mutation send. Initial Rework retains its existing serialization,
create-new0600/nofollow/sync and send behavior, with a warning before sending.
Help must disclose unsupported durable retry and ambiguous-outcome reconciliation
without suggesting new identity/preparation. Durable CLI retry remains OPEN ov-941;
Feature B/full-app acceptance is not complete.

The first red source retains original Retry behavior, with only private connector
extraction and focused tests added. No real transport is used: the test connector
counts attempts and returns an error without a Link; send is unreachable. Local
and remote assertions must fail on that count, not a compilation/environment
failure. Preserve red bytes/raw output. Restore the exact red source, then apply
only the explicit refusal/help/warning correction and run workflow_ tests once.
Includes malformed, unbound schema, missing file, unchanged bytes, original file
and producer tests. Compare the red/restored seam and unrelated code hashes.

Use separate reports here; do not modify original accepted reports. Run new
check.py red via cargo test --offline --locked -p farcooler-cli --bin farcooler
workflow_retry_ -- --nocapture; green via workflow_; lint via cargo clippy
--offline --locked -p farcooler-cli --all-targets -- -D warnings. Every invocation
uses native Python PATH, normal scripts/heavy FIFO/load-limit20, original923 warm
target and actual post-lock holder/profile/source proof, jobs6/dev-test-debug0/
incremental0/offline, >=10GiB sampled reserve and owned1800s supervisor cancel/reap.
No TMPDIR override, cold target, real runner, accepted shared/native/Web reruns.
