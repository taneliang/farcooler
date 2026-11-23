# Follow-up tickets from ov-1429

## ov-1429 A — Avoid recursive open-file walks

Measure open-file checks and the deletion accounting walk on a disposable deep build tree. Replace repeated recursive scans with a bounded implementation that still refuses deletion while any descendant is held open, including an alias of the same inode. Preserve tracked files, nested repositories, mount boundaries, retained reports and unknown-byte reconciliation.

## ov-1429 B — Profile the full CLI and idle runner

Build a portable replay with relocated repository, checkout, receipt and GitHub bindings. Record per-executable launch counts for a full idle minute. Measure the five commands end to end, including RPC payload sizing, CLI pagination, and rendering. Split remaining Plan time between SQL, state reads and wire projection.

## ov-1429 C — Keep terminal CI observations across subjects

Persist completed CI observations by repository and run identity so removing and re-adding a page reference or restarting the daemon cannot trigger a second job query. Precompute unpublished workflows’ starter settings so cold full Get replies can also reuse an observation. Measure remote-head latency against a delayed fixture and keep transport waits isolated from request serving.
