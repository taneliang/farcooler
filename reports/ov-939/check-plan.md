# Finite offline checks and resources — publish before execution

Source-only preparation has run no compiler, test, app or cache probe. All
commands below are proposed finite checks, not passes or an allocation of
Store/protocol/daemon edits. Resolve the exact manager seam handoff first.

## Resource envelope

* Native `/usr/bin/python3`; PATH begins `/usr/bin:/bin:/usr/sbin:/sbin` with
  existing Homebrew/Cargo tools as needed. Never change TMPDIR before heavy.
* Normal canonical `scripts/heavy --load-limit 20 -- ...` FIFO; no light/no-lock,
  CI bypass or injected lock flags. One foreground owned process group at a
  time. At most 1800s including queue per allocated invocation; communicate
  progress while queued/running. TERM, bounded wait, then KILL/reap only owned
  descendants on timeout, interruption, reserve breach or failure.
* Jobs <=6, dev/test debug0, incremental0, `--offline --locked`; no download,
  cold duplicate target, cleanup or automatic retry. Maintain >=10GiB reserve.
* Only original `../ov-923-workflow-runtime/target` for Rust. After obtaining the
  actual normal FIFO lock, verify live holder absence on its Cargo lock and
  source/profile provenance from saved original923/927 receipts, plus actual
  current toolchain/profile inputs. Save actual post-lock observations. A warm
  directory alone is insufficient proof. Do not copy stale top-level binaries
  or infer current-source proof from previous receipts. If gate fails, stop.
* Source hashes before/after, head, exact argv/env/profile/tool identities,
  raw stdout/stderr, exit/counts/duration/minimum free/output growth and owned
  child cancellation/reap go into separately named report files. Preserve
  every original failure and negative variant; never overwrite a red as green.

## Planned finite verification sequence

1. Light static checks: changed-file diff/whitespace, existing file-size and
   doc-comment rules; compare historical prepared JSON and feature A/goldens
   before/after without regenerating unrelated fixtures.
2. Offline focused pure Rust tests: reason byte/whitespace/control bounds,
   Unicode/exact retention; authority/revision/epoch/sequence, exact evidence,
   run/pin/candidate/policy, valid target and exhausted budget; unchanged state
   on refusal; two iterations and default-empty historical decode/replay.
3. Focused client/presentation/protocol/CLI tests: one shared semantic fixture
   for the finite form/errors/action/remaining count; exact reason forwarding;
   lossless IDs; strict rejection of malformed/unknown fields, unsupported
   prepared versions; historical schema1 prepared bytes unchanged; stale
   prepare never sends; no silent reprepare on retained retry. Cargo packages:
   farcooler-workflow, farcooler-client, farcooler-protocol, farcooler-cli.
   Use exact name filters selected after implementing tests, not whole workspace.
4. One manager-allocated offline daemon integration file: private temporary
   fixture only, no normal daemon/app startup. Accepted reason survives Store
   reopen and event replay, duplicate after lost reply gives same durable
   receipt/no new effect, altered reason under same ID is identity conflict,
   stale/unauthorized/exhausted/malformed attempts have durable refusal where
   admission reaches receipts. Preserve independent pure-parser errors as such.
5. Existing focused TS/Swift/Kotlin harnesses with proven cached matching
   compilers/dependencies only, under the same FIFO/envelope. Check each native
   finite input path, draft/error/focus preservation, disconnect/replaced-run
   tickets, separate observation/input identity and retained bytes after adapter
   restoration. Inspect cache/toolchain compatibility first; stop missing cache
   rather than cold build. No full SwiftPM/Gradle/app/WASM rebuild by implication.
6. Meaningful red/restored controls: save original and mutant bytes/hashes;
   deliberately bypass reason validation, exact evidence, budget and immutable
   retry in the relevant focused implementations, one control at a time. Require
   intended assertion failure, restore exact bytes, then run the corresponding
   selected tests once. Do not stage/commit mutants or call an unrelated compile
   failure meaningful red evidence. Choose a finite minimum set that witnesses
   all specified failure classes; preserve expected vs actual counts/exits.
7. One scoped all-target compile/Clippy pass for changed Rust packages only if
   required by the final delta/gates. Stop unexpected failures, diagnose source,
   then run only the corrected affected check. No repeated unrelated passes.

Publish exact selected argv/resources once test names and required caches are
known and before any heavy execution. All checks are pending. Unit/compiler/
fixture proof does not establish real root/device navigation or full-app runtime.
Signed frozen source/API and sealed evidence need independently allocated scoped
security/ports review before composition; no acceptance item is checked here.
