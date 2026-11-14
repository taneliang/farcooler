# Spotlight CPU sample (2026-11-14, Singapore time)

On this engine worktree, `cargo check --offline -p farcooler-store` was running with a newly populated `target/`. One `ps -A -ww -o %cpu=,command=` snapshot at 23:47:11, before adding `target/.metadata_never_index`, showed mds 0.0%, mds_stores 0.4%, and six mdworker_shared processes at 0.0% each.

The marker was then created in this worktree's target while the same build was still running. A snapshot at 23:47:43 showed mds 0.7%, mds_stores 0.0%, and three mdworker_shared processes at 0.0% each. A daemon test build was also waiting for the same Cargo target lock. These are single system-wide process snapshots, not attribution to this worktree or a controlled performance comparison; the baseline was already quiet, so they do not establish a CPU reduction.
