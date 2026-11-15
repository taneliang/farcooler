# ov-1210: Rust tests from one shared archive

## What changed

`scripts/rust-archive.sh` is the only place the runtime binaries and the nextest
archive are built. Both CI archive jobs (`rust-build`, `rust-macos-build`) call it,
and the lane gate derives its archive step from the same CI commands.

ov-1182 had already moved CI to one archive per platform with 4 (Linux) and 2 (Mac)
partitions. This ticket's change is the single producer, so CI and the lane gate
cannot drift into a second build graph.

## Wall minutes (GitHub-hosted runners, real runs)

- Before: main run 43732000446 (commit e0b125237, the base of this branch)
- After: branch run 43733882456 (commit 01670be6a, the same Rust jobs copied from ci.yml)

| Job | Before | After |
| --- | --- | --- |
| Linux archive | 4.0 | 4.0 |
| Linux shards 1-4 | 9.9 / 11.5 / 9.7 / 12.8 | 10.4 / 11.6 / 9.4 / 10.2 |
| Mac archive | 14.5 | 7.8 |
| Mac shards 1-2 | 8.1 / 4.0 | 9.0 / 8.5 |
| Linux Rust check job | 8.2 | 7.9 |
| First start to last Linux Rust job done | 22.8 | 15.8 |
| First start to last Mac Rust job done | 22.7 | 16.9 |

Read this carefully: the script refactor does not compile or run anything
differently, so the gain is not from this commit. The Mac archive's 6.7 min drop is
a Rust cache hit versus a cold build, and Mac shard times swing with runner
speed (shard 2 ran 180 s before, 399 s after). The runs show the one-producer
layout costs nothing; they do not isolate a speedup from it. The 8.4 + 20.3 min
figure in the ov-1186 spike was the layout before ov-1182.

## Test inventory (identical by count)

The nextest log of every partition prints a coverage line from `scripts/rust-shards.py`
proving the union of partitions equals the unpartitioned inventory.

| Platform | Runnable tests | Per-partition run counts, before | after |
| --- | --- | --- | --- |
| Linux | 7250 in 4 | 1785 + 1804 + 1812 + 1849 = 7250 | 1785 + 1804 + 1812 + 1849 = 7250 |
| Mac | 2785 in 2 | 1372 + 1413 = 2785 | 1372 + 1413 = 2785 |

The Linux Rust check job's targeted nextest runs (21, 2, 3, 3, 14, 2 tests) are also
the same in both runs.

## Rust caching stanzas

None is deleted, because none is replaced:

- `rust-build` (`key: rust-test-archive`) and `rust-macos-build`
  (`shared-key: rust-macos-tests`) are the archive producers. They are the only jobs
  that compile the test profile, so these caches are what keeps the archive build short.
- The shard jobs (`rust-tests`, `rust-macos-tests`) have no Rust cache: they download
  the archive and never compile.
- The `rust` matrix job's cache (`<os>-dev-test-clippy-features-default-tailcat`)
  serves a different build: `cargo clippy` and `cargo check` across feature sets,
  which an archive cannot provide.
- The Swift, Android and web jobs' `rust-cache` stanzas build other targets (FFI
  libraries, JNI, the web daemon) and consume no nextest archive.
