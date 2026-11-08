# ov-984 result

Design: `design.md`. Scratch runners only; this Mac's runner and app were never touched.

## What now exists

- **The recovery screen.** When this Mac's runner refuses with `cutover_held`, the window's empty state is `RunnerUpgradeView` (`FleetPlaceholder.Phase.upgrade`), not "Couldn't Read the Fleet". One button, **Finish Upgrade**.
- **Touch ID first.** A fresh `LAContext` (`.deviceOwnerAuthentication`, no reuse window). Closing the prompt runs nothing.
- **The steps, as the person, from the app.** The bundled CLI, run as the app's own child: `inspect`, then Prepare, Commit, Verify, Release and Finish from wherever the runner stands. Nothing goes through a pane. The recovery seat check is unchanged (`caller_seat.rs` untouched).
- **Progress in plain words**, a row per step. **On failure:** that step's sentence, what Recover will do, the runner's own words in a box, and **Recover**. Before Commit, Recover puts the original data back (`workflow-cutover recover`). From Commit on the runner refuses a rollback, so Recover runs the steps left. Recover asks for Touch ID too.
- **After Finish** the app runs `daemon ensure` and reads the fleet again, so the window opens the work.
- **Daemon:** `inspect` names the attempt in hand (held or prepared), so a runner held by a live Prepare is prepared under its own attempt.
- **Store:** `examples/write_source60` writes a schema-60 database for a scratch runner.

## Evidence

- `RunnerUpgradeTests`, 13 tests. A failure at each of the five steps is its own test case (parameterized). Each gives the step's sentence and the runner's words, offers Recover alone, and stops there.
- Mutants (`mutants.py`, `mutants-results.txt`), 6 of 6 red: the upgrade phase dropped, no Recover on failure, a committed upgrade rolled back, no Touch ID, the held attempt ignored, a failure that runs on.
- `RecoveryTests::inspect_names_the_attempt_in_hand`, seen red first (`left: Null`), then green with all 15 recovery tests.
- **End to end** (`end-to-end.py`, `end-to-end.json`). A scratch runner on schema 60 starts in recovery. The screen's model drives the real CLI against the real daemon, through Prepare, Commit, Verify, Release and Finish. The daemon exits 75 and restarts into ordinary service, with the data `released`. Only Touch ID is a stand-in. The test runs as an orphan on its own pty, which the seat check reads as a person, as it reads the app.
- **The converse** (`end-to-end.py --as-agent`, `end-to-end-as-agent.json`). The same test from an agent's shell is refused at the first call (`scope denied: person required`), and the data is untouched.
- **Captures** at the narrowest window width, 640 pt, light and dark (`captures/`): before, during (a real run held at Verify), failed at Verify, and after.
- Lints: copy-lint, doc-comment-check and file-size-budget pass. The Mac floor is raised from 1929 to 1942, for 13 new tests. It isn't measured by a full run; the train runs the full suite.

## Not proved

- **A real Touch ID press on the real app.** No agent can press the sensor. The real `.app`, launched by `open` against a scratch schema-60 home, with the button pressed by a person, is untried. That's acceptance 1's last step, and it's the owner's.
- **The seat as the app.** The end to end proves a person's seat through an orphan pty, not through the app bundle on the chain. `vouches_for_a_person` already has its own tests.
- **The words in the box.** The runner's generic refusal still says "Run `farcooler workflow-cutover inspect`", a terminal pointer, and it shows in the box. The sentence above it is plain. Changing the daemon's `Refused::said` would change the CLI's words too, so it's left as a follow-up.
