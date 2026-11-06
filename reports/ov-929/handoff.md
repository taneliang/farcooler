# ov-929 focused source and evidence handoff

Base: `7cae4421e9d2c3aae62ed06153c3f7f627267bd5`. This handoff is included in the signed commit that pins the source and evidence. Independent equivalence review is requested after that pin; integration approval is not claimed.

The task count was correct: `rpc.rs` contained 2482 lines against the unchanged 2450 ceiling. The preferred extraction was insufficient: all four test modules were already external, and the workflow arm was only 24 lines. Manager decision `f09d3652` approved moving the cohesive runner-settings helper group instead.

Exact mapping: original `crates/daemon/src/rpc.rs:763-840` maps to `crates/daemon/src/rpc/runner_settings.rs:4-81`. The complete block, including its marker, comments, guards and whitespace, is preserved. The four methods gain `pub(super)` visibility so the parent module can keep calling them. The new module has one explicit parent import and an `impl Rpc` wrapper; `rpc.rs:18` registers it. No call site changes. `rpc.rs` is now 2405 lines, the new module 82.

`equivalence.py` strips exactly those four visibility tokens and the wrapper, removes the module declaration, and reinserts the block before `dispatch`. Its result is byte-for-byte equal to base `rpc.rs`; `equivalence.json` records both source hashes and the base hash. Thus every other RPC byte, including the factory's closed-session guard, step-up/scope/capability/role gates, workflow authority capture and both awaits, remains unchanged. Runtime context recheck, method/capability tables, receipt/error semantics and the Session adapter are untouched. Extracted helper bodies preserve refresh/lookup order, all awaits, theme validation, errors and config handling. There is no new externally accessible API, schema or behavior.

Validation receipts and logs are adjacent. `check.py` supervises each individual command for 1800 seconds with a 10GiB reserve and owns/cancels/reaps its subprocess group. Each command takes normal `scripts/heavy --load-limit 20` FIFO admission. `entry.py` then verifies no open target or package-cache lock holder and load5 below20 before execing Cargo. It uses only the approved original ov-923 target; no target trimming/deletion or duplicate cache. Explicit dev/test debug0, incremental0, jobs6, offline; native Python/toolchain versions are saved in `toolchain.json`. Exact argv, before/after hashes, exits, duration and minimum free bytes are recorded per command. Receipt HEAD names the unchanged base at launch, while before/after hashes identify the actual tested working source.

The unchanged file-size checker passes (exit0), without updating its manifest or tolerance. `git diff --check` passes. Focused test counts and lint results are recorded below after completion. No settings test reads/writes actual owner configuration; the scope tests only inspect tables and the workflow/socket fixtures use isolated state. No full/combined gates, main CI, provider/app/owner database, formatter, amend, reset, stash, cleanup or push. Full combined gates and main CI remain the parent's responsibility.

Final checks (all exit0, before/after source hashes equal):

- scope: 10 tests passed; 31.25 seconds; minimum free 38246887424 bytes.
- green: 31 tests passed; 12.06 seconds; minimum free 38399004672 bytes.
- socket: 2 tests passed; 14.09 seconds; minimum free 38410194944 bytes.
- session: 2 tests passed; 16.12 seconds; minimum free 38367330304 bytes.
- lint: daemon all-targets clippy, warnings denied; 22.14 seconds; minimum free 38404792320 bytes.

Independent equivalence review remains pending with the orchestrator; stop at signed pin. No broader regression reruns are justified by this unchanged-body extraction.
