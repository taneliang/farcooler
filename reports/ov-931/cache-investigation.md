# Shared target provenance correction

The first affected-suite attempt compiled the protocol test target with the new
module, but reused an older protocol dependency library lacking workflow_config.
The preceding check had succeeded. Source hashes remained unchanged in both runs.
`cache-investigation.json` records protocol dep-info files: check/test artifacts
`e0daa2b9df0567f3` and `8a3ab00a0937e0f8` include workflow_config.rs;
normal dependency artifact `caa847620be6bb13` does not. All use relative source
paths in one shared warm target across worktrees; older artifact timestamps are
newer than this worktree's source root, so mtime freshness can accept another
worktree's library. This is a stale dependency-library selection, not evidence
that the new module is conditionally absent.

Correction: after normal FIFO admission and an empty actual cargo-holder probe,
verify the original warm-target provenance, then refresh only the local affected
crate entrypoint mtimes without changing source bytes. Cargo invalidates those
crate fingerprints normally. Record touched paths, source hashes and holder probe;
retain the same target/profile/jobs/offline parameters. No cache deletion, cargo
clean, target duplication, or concurrent writer. Preserve the failed attempt.
