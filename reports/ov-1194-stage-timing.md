# ov-1194: actual workflow stage timing

Read-only snapshot on November 15, 2026, from the Canary runner's workflow
SQLite store, restricted to workspace `01a0e3c7-3e2c-7ed0-b31c-0cc0d7401571`.

```sh
python3 scripts/workflow-review-timing.py \
  "$HOME/Library/Application Support/com.farcooler.FarCoolerCanary/farcooler.db" \
  --workspace 01a0e3c7-3e2c-7ed0-b31c-0cc0d7401571
```

The script pairs each successful build observation with the subsequent
`WaitingJoin` notice for the same item and candidate. A later build replaces
the earlier start. Effect `run` identifiers change between stages, so they are
reported for audit, rather than used to pair stages. It reads persisted event
`now_ms` timestamps, in revision order, in one database snapshot.

| Cohort | Candidate samples | Median recorded interval |
| --- | ---: | ---: |
| Existing production definition | 135 | 0 ms |
| Light review with parallel ticket check | 0 | unavailable |

129 of the 135 recorded baseline intervals are zero: build completion and
`WaitingJoin` have identical stored timestamps. The six nonzero intervals are
59,210,528 ms (three samples) and 103,838,830 ms (three samples). These stored
intervals do **not** establish real review latency. In particular, the zero
median must not be read as instantaneous review or used to claim an improvement.

All four schema-2 definitions in this store have an agent gate followed by the
legacy path route and optional owner approval. None contains `ReviewRoute` or
an agent review with a concurrent `check`. Therefore no actual after median or
before/after improvement can be reported from this evidence. The five simulated
500 ms samples in `docs/light-review.md` are only a scheduling experiment.

The measurement acceptance item remains open. It requires publication of a
light-review definition with a trusted full-review policy, completed real
candidates in both cohorts, and stage timestamps that capture their elapsed
execution time. This patch does not publish a workflow or replace the running
daemon. Those are operational changes beyond committing this ticket's fix.
