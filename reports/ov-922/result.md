# ov-922 implementation handoff

Baseline and cached origin/main: d625aaeb9c4c41d5849170eb21b909e55bd56d11.
Branch: ov-922-workflow-core. Implementation frozen at source-sha256.json.

The original fixed-six-stage engine framing was wrong: the approved finite coding
flow is an example, not mandatory review policy. Definitions now validate any
bounded forward sequence, optional review, repeated checks, generic Action/Evaluate
bindings, and selected/every-stage approval. Rework targets a pinned stable earlier
producer ID. Producerless evaluation paths reject before enqueue. Coding remains
a ready-made fixture; tracker is an exact immutable no-Git checklist fixture.

Only active admitted items reserve resources. Blocked dependent items do not
reserve ahead of their prerequisite; activation rechecks one waiter at a time.
Independent review exposed an additional cross-workspace completion deadlock.
Store-stamped relevant busy_resources and controller Advance resolve it while
persisting complete deterministic replay inputs. Original request receipts remain
unchanged on duplicate ID; effective event payload/hash includes actual occupancy.
See contract.md for public core/store APIs and runtime obligations.

## Validation

- final-focused-tests.log: 13 store workflow tests + 16 pure workflow tests,
  29 passed, no failures/ignored; wrapper exit 0. The subsequent store regression
  and restored cross-workspace test validate the final SQL occupancy filter.
- store-regressions.log: 486 passed, 0 failed, 1 ignored, exit 0. Covers all store
  library tests, including all 13 workflow tests and migration from each of 61
  registered source schema positions. Historical Welcome migration identities
  remain asserted; obsolete registry-tip assertions were removed. Compatibility
  floor follows the actual appended Refused migration, not a copied number.
- Six mutation tests (stale revision, mandatory review, blocked reservation,
  fixed rework index, candidate producer, cross-workspace occupancy): each fails
  its intended assertion with exit 101; each restored filtered test passes exit 0.
  See mutation-*.log, restored-*.log and checks.py; restoration verifies hashes.
- Generated replay property trace: 128 seeds, 8192 inputs, 572 accepted and 7620
  refused; deterministic transition/effect equality, revision and refusal purity,
  and replay identity checked. This bounded generator is not a formal proof.
- clippy.log: both focused crates, all targets, -D warnings, exit 0.
- Source whitespace check: exit 0. The inherited raw focused-tests.log retains
  its extra blank line at EOF, which the staged whitespace check flags; evidence
  content is preserved. No formatter or full integration gate was run.

All heavy checks used one normal scripts/heavy FIFO foreground route, jobs6,
load5 limit20, native Xcode Python via /usr/bin/python3, offline Cargo,
dev/test debug0 and incremental0. The assigned tree's inherited target cache was
used; pre-run host process inspection found no cargo/test wrapper competitor.
Normal Cargo source/profile fingerprints rebuilt changed crates; no alternate or
shared target was selected. Scripts monitor a 10GiB reserve and own/reap their
process group; final minimum free space 22,821,310,464 bytes, no cancellation.
Exact tools/profile and final source hashes are in validation-provenance.json and
source-sha256.json. Earlier focused-tests.log is historical, not final certification.

## Review and release boundary

Independent code/data-safety review: independent-review.md, including the two
findings, fixes and final no-blocking-findings source verdict. Execution evidence
is the builder's saved raw logs; the reviewer did not rerun heavy commands.

No external effect dispatcher, new runtime caller, legacy mutation replacement,
owner DB migration, installed app activation, push or product UI change is enabled.
The registry appends additive isolated tables with Older::Refused semantics.
Future runtime must authenticate grants, enforce adapter fencing/receipt provenance,
schedule Advance after cross-workspace release, and reconcile admitted uncertainty.
Required integration/main CI and owner-data conservation/cutover remain parent
release conditions. This commit is a focused core/store handoff for review.
