# ov-984: run the cutover from the Mac's recovery screen

## What's there today

- A daemon whose database is below the engine schema serves recovery only
  (`main.rs:218`, `rpc/recovery.rs`): every ordinary method gets
  `version-incompatible` / `cutover_held`, so the window's fleet read fails and
  `FleetPlaceholder` draws "Couldn't Read the Fleet" with the CLI's stderr. There
  is no recovery screen; the way through is `farcooler workflow-cutover` five
  times from a terminal outside every pane.
- `recovery_seat` already takes the Mac app as a person's seat: the bundle the
  daemon ships in, on the caller's chain (`caller_seat::vouches_for_a_person`).
  The CLI the app runs as its child passes; the same CLI from an agent pane
  doesn't. Nothing in the seat check changes here.

## Design

1. **Detect.** The local runner's fleet error carries `cutover_held`
   (`RunnerRefusal.isCutoverHeld`). `FleetPlaceholder` gets an `.upgrade`
   phase for it, ahead of `.failed`, and draws `RunnerUpgradeView`.
2. **One button, Touch ID first.** "Finish Upgrade" asks `LAContext`
   (`.deviceOwnerAuthentication`: Touch ID, or the login password) with a fresh
   context and no reuse window. A cancel runs nothing. This is the person's
   intent at the app; the runner's own gate stays the seat walk, unchanged.
3. **The steps, as the person, from the app's own process.** The app runs its
   bundled CLI as its child (`farcooler --json workflow-cutover <op>`), as
   every other app action does; nothing goes through a pane. Order:
   Prepare (attempt in hand, or a new `mac-<time>`), Commit, Verify, Release,
   Finish. It starts where the runner stands (`inspect`: no baseline → Prepare;
   `quiesced` → Commit; `committed` → Verify; `released` → Finish), so a
   second press after a failure goes on rather than starting over.
   `inspect` gains `attempt` (`transfer::attempt_in_hand`, read-only), so a
   runner held by a live Prepare is prepared under its own attempt.
4. **Progress in plain words**, one row per step: Getting your data ready,
   Making a backup and switching over, Checking everything came across,
   Handing your data to this version, Restarting Far Cooler.
5. **Failure.** A sentence for the step ("Far Cooler couldn't check that
   everything came across."), the runner's own words in a box, and **Recover**.
   - Before Commit, Recover is `workflow-cutover recover`: the original database
     back, as it was; the runner stays on the recovery screen, ready to try again.
   - From Commit on, the runner's `recover` refuses (committed state recovers
     forward only), so Recover runs the remaining steps forward, which is the only
     recovery there is. The copy says the upgrade is kept.
   - Nothing to undo (Prepare failed before it wrote anything): Recover just
     goes back to ready.
6. **After Finish** the recovery daemon exits 75. The app runs `daemon ensure`
   (it starts the daemon again where nothing supervises it) and reads the fleet;
   the window goes on as normal.

## Tests (each seen red first)

- Swift, `RunnerUpgradeTests`: a failure at each of the five steps gives that
  step's sentence and offers Recover; Recover before Commit runs `recover`, after
  it runs forward; Touch ID refused runs nothing; it starts from where `inspect`
  says; the held runner's attempt is used.
- `FleetPlaceholder.phase`: a `cutover_held` fleet error is `.upgrade`.
- Rust: `inspect` reports the attempt in hand.
- End to end: a scratch runner on schema 60 (`write_source60`) and the real
  recovery daemon; the app's model drives the real CLI. Touch ID is the one
  stand-in there.
- Captures at the narrowest window width: before, during, after.
