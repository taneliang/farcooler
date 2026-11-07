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

## Concrete first invocation, after manager8fc7ffd6 handoff
+
+`/usr/bin/python3 reports/ov-939/check.py compile /Users/e-liang/.cargo/bin/cargo check --offline --locked -p farcooler-workflow -p farcooler-protocol -p farcooler-client -p farcooler-daemon -p farcooler-cli --all-targets`
+
+The owned supervisor enforces the published1800s/reserve/profile envelope and
+post-lock entry binds actual original923 Cargo holders and local roots. This is
+one compile pass to find all changed public-struct consumers before selected
+tests. No other source changes while it runs. Directly necessary fixture
+constructors outside Review admission are mechanical `reason: None` additions
+only; no bootstrap/Store/notice behavior changes.
+EOF
/usr/bin/python3 reports/ov-939/check.py compile /Users/e-liang/.cargo/bin/cargo check --offline --locked -p farcooler-workflow -p farcooler-protocol -p farcooler-client -p farcooler-daemon -p farcooler-cli --all-targets
The first compile exited101 solely at a baseline socket-test Publish constructor
missing `notifications` (verified in approved82d6). Mechanical `notifications:None`
restores that fixture, without publication/notice behavior changes. Original
compile.log/json retained. Next checks select affected client/pure/protocol/CLI
workflow tests and the exact daemon reasoned test; they also compile the correction.

Concrete focused test group: `/usr/bin/python3 reports/ov-939/check.py rust-selected /usr/bin/python3 reports/ov-939/rust-selected.py`.
It takes one normal FIFO lease with the same post-lock provenance gate, then
runs three sequential Cargo commands (offline/locked): workflow integration
`workflow_reasoned`, client/protocol/daemon lib `workflow_reasoned`, CLI bin
`workflow_reasoned`. Stops first unexpected failure; no unrelated tests/retries.
Native preflight found the recorded TS compiler, Kotlin2.4.10 compiler JAR/cache
and SDK27 Swift compiler present. Matching cached classpath inputs and exact
native argv are published in `ports.py` before those checks.

Initial focused run: pure2/client1 pass. The new daemon test failed before its
assertions because `coding_definition(false)` removes Review altogether; change
its fixture argument to true. Remaining-only rerun selects daemon/protocol and
CLI `workflow_reasoned`; it does not rerun the already-passed pure/client cases.

Focused reasoned engine/client/protocol/CLI/daemon tests now pass (six distinct
cases). A new authenticated socket test adds actual Session prepare_validation,
accepted-before-answer-loss and replacement-Session exact receipt/bytes proof;
this is a private fixture, not real app/root navigation. Native commands and161
cached input hashes are frozen in ports-plan.json. Next invocation:
`check.py ports /usr/bin/python3 reports/ov-939/ports.py` (TS, Node lifecycle and
journal, Swift6/SDK27 shared views/model and harness, Kotlin model/JUnit/Compose;
sequential, stop first failure, same1800s/FIFO/reserve).

Concrete subsequent checks (each normal FIFO, separate labels, same bounds):

* `check.py rust-regressions /usr/bin/python3 reports/ov-939/regressions.py`:
  all18 pure workflow tests, client workflow/presentation/socket tests including
  added B, protocol workflow tests, only daemon wire_commands tests, CLI workflow
  producer tests. Existing golden files are compared unchanged, not regenerated.
* `check.py rust-negatives /usr/bin/python3 reports/ov-939/negatives.py`:
  six one-at-a-time reducer controls remove reason, authority, exact evidence,
  budget, revision and duplicate fences. Raw source originals/mutants retained;
  expected assertion101 required, exact source restoration including interruption,
  then the two corresponding restored reasoned tests once. No mutated commit.
* After these pass, one scoped all-target Clippy of changed Rust packages and
  whitespace/doc-comment/file-size gates. Native compile failures, if any, stop
  the group and are preserved; continue only corrected/unreached stages.

Ports first run: TS compile0, Web session (legacy + B)0, all16 controller/DOM
cases0. Swift stopped on a harness-only `Array.isEmpty()` typo, before Kotlin.
Original harness saved as swift-check.initial.txt and raw compile log retained;
corrected `isEmpty` property. Resume at Swift only via `ports.py swift corrected`,
with new labels, followed by the unexecuted Swift/JUnit/Compose stages. Supervisor
now also hashes its own helper/harness sources, in addition to product sources.

Corrected Swift compile/harness pass, including B restored retry. Kotlin compile
caught a newly introduced local `selection` payload shadowing its lifecycle
selection counter. Rename the local payload `submitted`; save original source
in kotlin-shadow.original.txt and retain raw failure log. Resume only Kotlin,
JUnit and Compose via `ports.py kotlin corrected2`; no TS/Swift rerun needed.

Kotlin compile/JUnit (10 cases)/Compose all pass. Final source inspection found
new Swift local-wrapper object casts should refuse malformed arrays instead of
force-casting; use guards and add the malformed-array no-prepare check. Only
Swift compile/harness needs repeat for that delta, `ports.py swift final
swift-test`; Kotlin/TS unchanged and not repeated. Then the planned Rust
regression/red/restored and final applicable lint gates remain.

CLI completion delta uses shared Session envelope validation and `--prepared-file`
create-new0600 + file/parent sync before send. `workflow retry --prepared-file`
reads only the retained envelope and forwards `json` verbatim. Focused tests
exercise preserved whitespace/reason/ID bytes, no overwrite, mode, unchanged
file after request construction, unsupported schema and symlink refusal. Next:
`check.py rust-cli-session /usr/bin/python3 reports/ov-939/cli-session.py`, only
changed client workflow/socket/envelope and CLI workflow tests. Pure/protocol/
daemon and native compilers are unchanged and are not repeated.

One additional immutable-retry red/restored control:
`check.py web-negative /usr/bin/python3 reports/ov-939/web-negative.py` alters
only owned compiled workflow-session.js, and only when prepared bytes contain a
reason. The B journal test must raise AssertionError for changed retained bytes;
source/mutant/hash/raw logs saved, exact compiled bytes restored in finally,
then the corresponding Session test once. No app/browser or source mutation.

Web retry red/restored assertions passed, but its inner result filename collided
with the outer supervisor web-negative.json. Raw red/restored logs and outer
source/reserve receipt are intact. Correct only the report filename and seal
that one small Node control with distinct log/result/supervisor labels; no
product/compiler rerun. Preserve the original helper in web-negative.initial.txt.
Final client17+1ignored and CLI4 delta tests pass; combined final distinct Rust
coverage is46 passes/1ignore. All app/root/device obligations stay unverified.

Final scoped lint invocation:
`check.py rust-lint /Users/e-liang/.cargo/bin/cargo clippy --offline --locked -p farcooler-workflow -p farcooler-protocol -p farcooler-client -p farcooler-daemon -p farcooler-cli --all-targets -- -D warnings`.
This also supplies a final all-target compilation after mechanical baseline
fixture correction and final CLI/shared-envelope delta. Then light existing
file-size/doc-comment/diff rules, manifests and normal signed freeze only.
