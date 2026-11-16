# ov-1293: a report file no longer forces full Rust

## Why every run went full

`select()` in `scripts/rust-select.py` returned "everything" at the first changed file that was not
a crate `.rs` source. `reports/<key>/README.md`, any `docs/` or `*.md` file, and `.github/workflows/ci.yml`
all fell into that. It also stopped at the first one, so the gate printed one reason out of many.

## What changed

- Docs paths (`reports/`, `docs/`, any `*.md`) select nothing, unless a crate `include_str!`s them.
- `ci.yml` is judged by the jobs an edit touches. Only the Swift, iOS, Android, relay, site and web
  jobs are Rust-free. A Rust job, the triggers, `env`, a new or removed job select everything.
- `Cargo.lock`, `Cargo.toml`, build scripts, the toolchain file, `.config/nextest.toml`, fixtures,
  the protocol crate and unmapped paths still select everything.
- Every reason is listed, one per offending change.
- A selection of no packages skips the nextest step (it was an error). CI on main is untouched and runs the full suite.

## Replay of the 10 newest landings on main (fixture `scripts/rust-select-landings.json`)

"Before" is the old rule (any non-crate-source path is full). The Nov 16 gate receipts keep only the
first reason, not the 4,300-file lists, so the fixture uses the commits on `origin/main` instead.

```
31646caf5 narrow 4 crates | reverse closure includes dev/build/optional/all target edges and CLI -> daemon
c65f00916 narrow 13 crates | reverse closure includes dev/build/optional/all target edges and CLI -> daemon
42522c5ee narrow 13 crates | reverse closure includes dev/build/optional/all target edges and CLI -> daemon
780db62db full 0 crates | docs and non-Rust files select no tests: docs/ci-reruns.md, docs/engine-pipeline.md
d421b493e full full | full: M .github/workflows/ci.yml (changes the workflow header, changes, tunnel-helper, wire); full: M scripts/
7b49c542c full full | full: M crates/client/src/testdata/workflow-labels.json
7ef431e6b narrow 13 crates | reverse closure includes dev/build/optional/all target edges and CLI -> daemon
41d057c43 full 0 crates | docs and non-Rust files select no tests: reports/ov-1290/README.md
e5bf49fc6 full 0 crates | docs and non-Rust files select no tests: reports/ov-1290/README.md, reports/ov-1290/per-file.txt
d51f1f221 full full | full: M scripts/daemon-layers-counts.txt; full: M scripts/daemon-layers.py
```
