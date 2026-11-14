# ov-1242: CI train rebuilt on current main

Rebuilds ov-1240 (`ov-1240-ci-train-2`, based on the old main `73974f898`) on `origin/main`
at `f9afc6472`, so it lands as one linear member. The branch has no merge commits, and
`origin/main` is an ancestor of it.

## Tickets the branch carries

Queued tickets not yet on main:
ov-1211, ov-1216, ov-1209, ov-1184, ov-1219, ov-1183, ov-1215, ov-1225, ov-1230, ov-1223.

The train's own tickets:
ov-1218, ov-1224, ov-1189, ov-1182, ov-1226, ov-1156, ov-1237.

Also carried, because they were in ov-1240 and are not on main yet:

- ov-1186, ov-1200, ov-1188, ov-1205, ov-1207 (landing now; the final rebase drops any that land first)
- ov-1185, ov-1190, ov-1194 (ov-1239's train work)
- ov-1229, ov-1239, ov-1240 (train bookkeeping and its quarantine list)

Plus this ticket's own commit: a self-test for an Android shard matrix in `ci.yml`.

## Skipped as already on main (33 commits)

The first 33 commits of the 125 in `73974f898..ov-1240-ci-train-2` were not picked.
Each belongs to a ticket whose commits `git log origin/main --grep` shows on main
(landed with different SHAs). The first of them, `76fa64bfd`, conflicted in `ci.yml`,
`nightly.yml` and `scripts/macos-rust-tests.py` against main's later form of ov-1160.

| Ticket | Commits skipped | Commits on main |
| --- | --- | --- |
| ov-1160 | 7 (`76fa64bfd` .. `68e2a014a`) | 7 |
| ov-1181 | 1 (`5bf2d76b0`) | 1 |
| ov-1180 | 1 (`bfe99f524`) | 1 |
| ov-1157 | 2 (`1ce01c030`, `cda646aed`) | 2 |
| ov-1098 | 6 (`03f13a95e` .. `c2b98b988`) | 6 |
| ov-1151 | 1 (`89c05701a`) | 1 |
| ov-1008 | 1 (`c6c7350da`) | 1 |
| ov-1020 | 3 (`1e3a6b3e0`, `a90d6d698`, `78f4c2d7b`) | 4 |
| ov-1149 | 2 (`3edf9d5db`, `3053e7280`) | 2 |
| ov-1137 | 3 (`a9b891d04`, `8f3c0a89a`, `080c9fef1`) | 3 |
| ov-1162 | 2 (`57be83bdb`, `9f88addd6`) | 2 |
| ov-1165 | 1 (`c76bffa14`) | 1 |
| ov-1172 | 3 (`12c0b5a2c`, `ae0807c00`, `74092c9e4`) | 3 |

Empty picks: none. Every one of the other 92 commits (`76354851c` .. `7d294b8ba`) applied
cleanly, in order, with `cherry-pick -x`.

## Conflicts resolved

None among the 92 picked commits. The conflict markers ov-1240 had left in `ci.yml`
(`bc9bdf4a8`) are already fixed in the picked history; no marker remains in `.github/` or `scripts/`.

## The ov-1184 gate failure

`scripts/ios-ui-shards.py --check` objected to a `shard:` matrix in `ci.yml`. The picked
`134b61a6a` (ov-1240) already restricts that check to the iOS UI job (`ios-ui`, or a job that
uses the ios-ui-shard action or `scripts/ios-ui-tests.sh`), so the check passes with the
Android `android-ui` matrix present.

What was missing was a test. `--self-test` now plants a second Android shard matrix next to
`android-ui` and requires `--check` to pass. With the iOS-job filter removed, that case, and
the real-tree case, fail with the exact gate message. With the filter, both pass.

## Verification

All commands ran with stdin from `/dev/null`: bash sources `~/.bashrc` when stdin is a socket,
which broke `gate-evidence-test.py`'s output assertion in this tool session. The test is not at fault.

- `CI=true ./scripts/lane-gate.py --base origin/main --skip rust,swift-tests,swift,android,web`: pass
- `HEAVY_LOCK_HELD=1 ./scripts/lane-gate.py --self-test`: pass
- `./scripts/lane-gate.py --dry-run --base origin/main`: pass
- `gate-evidence-test.py`, `engine-gate-test.py`, `nightly-dispatch-test.py`, `ci-concurrency-test.py`: pass
- `./scripts/ci-job-time-check.py --self-test`: pass
- `./scripts/actionlint.sh`: ok. The repo pins and patches actionlint for `concurrency.queue`;
  Homebrew's stock `actionlint` rejects that key on main too.
