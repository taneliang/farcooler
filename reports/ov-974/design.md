# ov-974: the owner's orchestrator can run the cutover

## Where the brief is right, and where it isn't

- `recovery_seat` (`crates/daemon/src/caller_seat.rs:148`) and the recovery
  handler (`rpc/recovery.rs:34`, `caller_is_a_person`) admit only `Seat::Person`. Right.
- The brief misses the first gate: the **live** daemon's Prepare
  (`rpc.rs:799-805`) captures its admission with `Admission::HOST_WIDE` and calls
  `require_person()`. No orchestrator gets past it, so the owner's orchestrator
  could never reach recovery at all. It changes too.
- "Check the pane table is readable then": it isn't, usefully. A recovery
  process has no Store, no inventory and no registry (`rpc/recovery.rs:1-8`),
  and it must not hold the database open: every recovery op takes
  `JoinedOwners::acquire`, the kernel proof that nobody else has the file open.
  Opening the Store per request to read `terminals`, logins and leads would race
  that proof, and between Prepare and Commit the file is frozen mid-transfer.

## Design

**Decide once, where the rules live; pin the process; check the pin in recovery.**

1. **Live Prepare** (has the Store). The caller's seat is walked as now. A
   person takes the existing path. An agent in a pane is admitted with
   `Admission::capture` for *that pane's workspace*, which is exactly the rule
   `workflow.move` uses (ov-951): the pane is that workspace's `Orchestrator`,
   `refuse_another_leads_login` passes, and `caller_person` is
   `Person::Owner` (so a member's orchestrator is refused). Not reused by copy:
   the same function is called.
2. Before the hold mark is written, the live daemon writes
   `<root>/cutover-delegate.json`: the attempt, the terminal and workspace ids,
   and the **pane's root process: pid and start time**
   (`process_incarnation`). A person's Prepare removes any such file, so an
   earlier attempt's delegate never outlives a Prepare.
3. **Recovery** admits a caller when it is a person (as today), **or** when the
   delegate file names the attempt now in hand (the hold mark's, or the
   baseline's once Prepare has run) and the walk from the caller, up its
   parents, reaches the named pane root with the same start time before it
   meets any tmux server. The connect-time pid + start check stays.
4. **Commit** records the caller as "the owner's orchestrator (terminal …,
   workspace …), acting for the owner: pid …".
5. Finish and Recover, once they succeed, remove the delegate.

### Why this is not a new way in

- Only the live daemon's full admission ever names a pane; recovery only asks
  "is this caller inside that one process tree, still the same process". A
  builder pane, a member's orchestrator and another lead's pane are refused at
  Prepare and so never named; in recovery any other pane fails the walk.
- An orphan the orchestrator left behind (`(cmd &)`, reparented to pid 1)
  doesn't reach the pane root: refused, as today.
- A pane restarted between Prepare and recovery has a new root process: refused.
  The owner can still run the rest by hand.
- The delegate is a file beside the database, as trusted as the hold mark and
  the database themselves: a process that can write it can already write the
  database it guards. The seat walk was always a guardrail against an agent
  *asking*, not a boundary against one writing files (`caller_seat.rs` docs).

## Tests (red first)

- e2e, real socket, scratch runner (`tests/the_owners_orchestrator_runs_a_cutover.rs`):
  a real tmux pane, the workspace's orchestrator on the runner's login, runs
  Prepare on the live daemon (exit 75), then Prepare, Commit, Verify, Release,
  Finish in recovery from the same pane shell; the journal names the
  orchestrator at Commit.
- Live Prepare refuses a builder pane, a member's orchestrator, and an
  orchestrator on another lead's login; each writes no hold and no delegate.
- Recovery refuses a pane not named by the delegate, a delegate for another
  attempt, and a pane root with a different start time.

## Skill

`crates/daemon/assets/manager/SKILL.md` "Cutover": the owner's orchestrator may
run it for the owner, from its own pane; no other agent ever does.

## Fix round: the owner's grant (review: pane descent admits everything under the pane)

Pane descent alone admits the orchestrator's shell commands, subagents and
scripts. So the live Prepare also needs the owner's word, once.

- **The grant** (`crates/daemon/src/cutover_grant.rs`). An answer
  `Allow cutover once` (exact, trimmed, any case) to a question on the same
  card that offered it, on a card of the orchestrator's own workspace, actor
  `user`, `said_from` `app` or `device`, at most 24 hours old, not superseded
  and not spent. The live Prepare spends it (a `runner` note on the card naming
  the answer and the attempt) under one lock, after every other check, before
  the hold. Recovery steps ride the attempt the Prepare pinned.
- **Where the seat is recorded.** The note's `actor`: an answer claiming `user`
  is walked (`task_ops::answer_actor`, ov-630), so one from a pane is
  `manager`/`agent:…`/`unknown` whatever `--actor` says. The walk is a
  guardrail (an orphan with a terminal of its own reads as a person), so the
  grant doesn't rest on it: `task.note` also stamps `extra.said_from`, the
  daemon's word, each from an authenticated seat (orchestrator, from the
  ov-971 review):
  - `app`: the caller process satisfies the designated code requirement of the
    bundle this daemon ships in (`code_identity`, Security.framework by pid,
    start time rechecked). The live Canary's DR pins
    `identifier "com.farcooler.FarCooler.canary"` and team `H6A2TRW47J`; its
    running app process passes it and its bundled CLI fails it (checked with
    `codesign --verify -R`, read-only). The app talks to `farcoolerd.sock`
    from its own process (lsof), so the daemon sees the app's pid.
  - `device`: the answer carries a step-up (ov-856) verified against the
    owner's phone's trusted key, over this very request. `task.note` takes an
    optional step-up (`step_up::OPTIONAL`) and never needs one. A browser's
    passkey and a member's key don't count. A device line alone is a preamble
    any local process can write, and an ssh session one any process of the
    owner's can open, so neither counts.
  - `cli`: anything else, the CLI in any terminal included.
  A caller sending `said_from` is refused, as with `message`.
- **Recheck in recovery.** Nothing but recovery can write the database while it
  holds it, so a lead, role or login can't change there; the recheck guards an
  edited or restored file. Each recovery step, `cutover_delegate::admits` reads
  the database read-only (`engine_cutover::startup::owners_orchestrator`): the
  terminal is still the workspace's orchestrator, with no member login, no rep,
  and no member leading the workspace (the owner's case of
  `refuse_another_leads_login`).
- **Journal.** Commit's note names the grant's answer id. `record_caller`
  already fsyncs the entry and its directory (`closing.rs:58-59`).
- **Not built here.** iOS and Android don't sign an answer yet, so until a
  follow-up ships that, the owner grants from the Mac app. The watch can't
  sign either.

Tests, each red with its check removed: `cutover_grant_tests.rs` (no grant,
agent-written, `--actor`, a forged stamp, a terminal's, old, spent, unasked,
another board; the app's grants once, the phone's signed answer grants and
its unsigned one doesn't), `code_identity` (a process is the program its
signature says),
`answer_seat_tests::no_answer_from_the_cli_grants_a_cutover` (real processes),
`recovery_tests::live_prepare_refuses_the_owners_orchestrator_without_the_owners_grant`,
`cutover_delegate_tests::every_recovery_step_rechecks_role_login_and_lead`, and
the e2e, now with the owner's grant.

## Second fix round: every grant needs the owner present

The review found that any process running the app's binary passes its code
signature, that age and spend were checked only at Prepare, and that the
routes writing an answer weren't listed. ov-977 (phones sign the grant) folds
in here.

### Where the brief is wrong

- `MacRemoteStepUp.swift` isn't the pattern for the Mac's own grant: it asks the
  iPhone to approve, for a Mac reaching another runner as a paired device. The
  owner's Mac talks to its own runner's socket, which had no step-up at all
  (`step_up::admit` refused one there: `step_up_unneeded`).
- The Mac app never sent its answers itself: `DaemonClient.answerTask` runs the
  bundled CLI, which fails the app's code signature. So the round-one `app`
  stamp could never come from the real app's answer button. The grant now goes
  out in-process (`RunnerCore`), signed.
- `step_up.rs:431-439` doesn't exist at `210fb000d` (the file is 421 lines).
  The replay guard is the challenge's single spend, `Challenges::take`
  (`step_up.rs:135`), called before any signature is read.

### The Mac grant

- **The key.** A CryptoKit Secure Enclave P-256 key with `.privateKeyUsage` and
  `.userPresence` (Touch ID, or the login password), made by the app
  (`MacStepUpSigner`). A keychain enclave key needs an `application-identifier`
  the app lacks; CryptoKit's doesn't (checked: an unentitled binary on this Mac
  makes one and reloads it from its wrapped blob). The blob lives in the app's
  Application Support and is useless off this Mac's enclave or without the
  person. Each signature gets a fresh `LAContext` with no reuse window.
- **On the runner.** The key is kept as `step_up_keys` row `local:app`
  (`step_up::LOCAL_APP`). A device's client id never holds a `:`
  (`farcooler_fence`), so no line can be it. `client.set_step_up_key` for
  `local:app` is taken only from a caller that passes `caller_seat::is_own_app`
  (code signature, never the walk), only `p256-sha256`, and the app may replace
  its own key: its code makes nothing but enclave keys that need the person, so
  a replacement is as good as the first.
- **The signature.** `step_up::admit` takes a step-up on the socket for an
  `OPTIONAL` method (`task.note`) only, verified against `local:app` with host
  pin `local`: the challenge is spent first, the request run is the one signed
  (`signed_request`), and the statement names `local:app`.
- **The stamp.** `said_from` is `app` only when the request carries that
  verified step-up for `task.note` *and* the caller passes the code signature.
  Neither alone: the binary can be run, and its window driven, by anything; only
  the person can put a finger on the sensor.
- **No other way to make the app answer.** `AppControl`'s socket serves
  `about`, `version` and `update` only; its one URL scheme is AuthKit's sign-in
  callback (`Info.plist`), and it has no AppleScript dictionary. And whatever reaches `answerTask` (a button, a
  notification action, accessibility) ends at the Touch ID prompt. The app
  reads no launch arguments; its environment overrides (`FARCOOLER_HOME`,
  `FARCOOLER_BIN`, the capture probes) can point it at another daemon, but a
  signature it makes there and an agent relays here fails the other half: the
  answer must come from the app's own process.

### Phones (ov-977)

`task.note` joins the client core's step-up builder (`session/step_up.rs`
`build`): an answer only, built as the plain one is, with its target. iOS
(`Connection.answerCutoverGrant`) and Android (`Connection.answerCutoverGrant`)
send "Allow cutover once" only signed, after Face ID, the passcode or the screen
lock; never plain, and a phone with no key is told how to get one. Every other
answer goes out as before. The watch relays to the phone, which prompts there.

### Rechecked at every recovery step

`cutover_delegate::admits` now also asks `grant_holds`
(`engine_cutover::startup::grant_holds`, read-only): the pinned answer is the
owner's, said from `app` or `device`, under `MAX_AGE_MS` old *now*, and spent by
the runner on this attempt and no other.

### Every route that writes an Answer note as `user`

| Route | `said_from` | `at` |
|---|---|---|
| `task.note` over RPC (`rpc_board.rs:101`, CLI, apps, web, phones) → `task_ops::note_from` | the daemon's `cutover_grant::said_from`; a caller's own is refused `extra_json` (`task_ops.rs:644`) | the store's clock |
| `pr_actions` thread reply → `task_ops::note` | always `cli` (`task_ops.rs:585`) | the store's clock |
| tests and migrations (`insert_note_with_at_for_test`, `migrate.rs` backfills) | none | not reachable from a request |

`Store::append` stamps `at` with `now_millis()` (`tasks.rs:312`); no request
field sets it. `TaskNoteAppend` has no `at`.

### Replay

A step-up's nonce is one of at most four live per session, 60 s, spent by
`take` before the signature is checked and spent whatever follows
(`step_up.rs:135`, `admit_local`). The same signed request sent again is
refused `challenge_expired`; a client-side token is spent by `step_up.finish`.
A spent grant admits no second Prepare, and a grant spent twice holds for
neither attempt in recovery.

### Tests (each red with its check removed)

- `cutover_grant_tests::only_the_apps_signature_and_the_owners_phone_key_say_a_grant`:
  the app unsigned, the app's key from another process, the app's key for
  another method, a phone's key on the socket.
- `cutover_grant_tests::the_macs_answer_is_signed_by_the_apps_key_once`
  (through `Rpc`): only the app writes `local:app`; no key, replay, another key,
  a device's pin, another method are each refused.
- `cutover_delegate_tests::every_recovery_step_rechecks_the_grants_age_and_spend`
  and the delegate test's unknown grant.
- `step_up_phone_tests::the_owners_signed_cutover_answer_goes_through_the_bridge`
  (client core to daemon): unsigned grants nothing, signed is `device`, a spent
  token is refused, a signed note that isn't an answer is refused.
- AgentKit `StepUpTests.cutoverGrant`; Android `CutoverGrantTest`.

## Third fix round: only a key the owner vouched for grants (review 2: B1, B2, M1 to M5)

Review: `.claude/agent/reports/ov-974/independent-review-2.md`. Ruling: decision `0b66353c`. B3 (the person path skips the grant; a direct SQLite insert forges one) is with the owner and not built here.

### B1: a key trusted on the walk never grants

- **Vouched.** `said_from: device` now needs the device bound to the owner (`owner_devices`, `confirmed`, never "nobody's, so the owner's") *and* its key vouched for with the owner present (`cutover_grant::owners_vouched_device`).
- **Who vouches** (`step_up_rpc::vouch_of`): the Mac app approving the key with its Touch ID key (`local:app` step-up on `client.approve_step_up`, plus the code signature), or a vouched owner device approving it with its own step-up, or enrolling it stepped up (`enrollment.rs`). The socket's `set_step_up_key`, the CLI's `people approve` and the socket's enroll still trust a key, and vouch for none.
- **Kept** in `meta` as `step_up_vouch:<id>` = the key's algorithm and bytes, then who. A new key under the id is vouched by nobody; a revoke deletes it (`clear_device_rows`). No migration this close to the cutover.
- **Refused loudly.** A signed grant that stamps `cli` is refused `key_unvouched`, so a phone says: approve it once in People on your Mac. The Mac lists the owner's unvouched devices with their codes ("Allow the Cutover From"); a phone shows its own code until then.
- **Today's phones** were all trusted on the walk: each needs one Touch ID approve on the Mac before it can grant.

### B2: a fresh key per signature, never from disk

`MacStepUpSigner` makes a new `.userPresence` enclave key in the app for each signature and holds its wrapped form in memory only. `MacOwnerStepUp.perform` proposes it as `local:app` and signs. Nothing is read from or written to disk.

### Minors

- **M1.** The hold is a file mark, not a transaction, so the spend can't share one. Admission checks the grant (`required`); `pin_and_hold` spends it after the drain and recheck, just before the mark, and a hold that fails gives it back (`cutover_grant_returned`, discounted by both readers). Past 24 hours mid-recovery, the pinned orchestrator is refused with `cutover_delegate::LAPSED`: only the owner finishes, by Recover or Finish.
- **M2.** A grant must be the person's only answer to the card's latest question, and that question must offer the option.
- **M3.** The Mac prompt: "allow your orchestrator on this Mac to run the cutover once, as <card> asks".
- **M4.** `admits_at`, with a case a minute past 24 hours.
- **M5.** `find` reads `Store::owner_answers_since`, a plain query, not full-text search.

### Tests, each red with its check removed

| Check removed | Test |
|---|---|
| `owners_vouched_device` back to `person_of == Owner` | `a_key_trusted_on_the_walk_alone_never_grants`, `the_owners_phone_grants_by_signing_its_answer`, `a_vouched_device_vouches_for_the_owners_next` |
| the vouch check alone | the same three |
| the `owner_devices` check alone | `a_key_trusted_on_the_walk_alone_never_grants` |
| the approver needn't be vouched | `a_vouched_device_vouches_for_the_owners_next` |
| the socket vouches without the app | `only_the_app_vouches_on_the_socket` |
| enroll vouches from an unvouched phone | `an_enroll_by_a_vouched_phone_vouches_for_the_new_key` |
| a revoke keeps the vouch | store `a_vouch_holds_for_its_key_alone` |
| any earlier question, any number of answers | `a_grant_answers_the_latest_question_alone` |
| a give-back ignored (daemon or recovery read) | `a_failed_hold_gives_the_grant_back` |
| `admits_at` without the grant check | `a_delegate_is_read_only_for_the_attempt_in_hand_and_only_for_its_pane` |
| no lapsed sentence | `a_lapsed_grant_says_the_owner_finishes_it` |
| `makeKey` reuses a key | Mac `MacStepUpSignerTests.freshKeys` |

Not run: a real Touch ID, Face ID or fingerprint prompt; the give-back after a real failed hold (only the ledger is tested).
