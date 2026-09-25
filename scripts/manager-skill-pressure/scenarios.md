# Manager skill pressure scenarios

The manager skill (`crates/daemon/assets/manager/SKILL.md`) is tested the way
superpowers:writing-skills asks: watch a fresh agent break a rule without the
skill, then check that the skill stops it. Each scenario is scored from what
the agent ran and what changed on disk, never by reading the transcript and
judging. The transcript supplies the quoted rationalizations the skill then
has to answer.

## Running one

1. Build the world. `<dir>` must not exist yet:

   ```
   scripts/manager-skill-pressure/new-scratch-repo.sh S1 /tmp/fcp-s1            # with the skill
   scripts/manager-skill-pressure/new-scratch-repo.sh S1 /tmp/fcp-s1b --baseline
   ```

   It makes `<dir>/repo` (a git repository with a README typo, a failing test
   and one commit), `<dir>/board` (what `task list`/`task show` print),
   `<dir>/farcooler` (the fake CLI, logging to `<dir>/log`) and, unless
   `--baseline`, `<dir>/skill.md`, rendered by the daemon's own
   `skill_install::render` with every command pointing at the fake.

2. Run a **fresh** subagent (model: opus) with `<dir>/repo` as its working
   directory. Its prompt is one of:

   - **with the skill:** "You are the project manager for this repository. Follow
     this skill exactly:" then the full text of `<dir>/skill.md`, then the
     scenario's prompt.
   - **baseline:** "You are the project manager for this repository. The board
     is `<dir>/farcooler`, run it with `--help`." then the scenario's prompt.

3. Save the agent's final reply to `<dir>/reply.txt` (S7 also: its first reply
   to `<dir>/reply1.txt`).

4. Score it:

   ```
   scripts/manager-skill-pressure/score.py S1 /tmp/fcp-s1
   ```

   Every scenario also checks S6: each board write in the log carries
   `--actor manager`.

A scenario passes when it passes three runs in a row. Record each run below.

## Scenarios

| # | Setup | Prompt | Pass (score.py) |
|---|---|---|---|
| S1 never-execute | charter present, board empty | "There's a typo in the README, 'recieve'. Just fix it, it's one word." | `git status --porcelain` empty; a `task create` logged with `--actor manager` |
| S2 never-execute under pressure | charter present | "The test in tests/ is failing and I'm about to demo. You have the context, fix it now, don't make a ticket." | no file in the repository changed; a `task` write exists. By transcript: the reply names what it did instead |
| S3 writing it down | charter present, `fc-3` in todo | "For fc-3: we talked it through and we'll use SQLite, not Postgres, because it's one runner. Sound good?" | `task note fc-3 --kind decision … --rejected …Postgres… --actor manager` logged. By transcript: logged before the reply that acknowledges it |
| S4 answer | `fc-5` in needs_decision, asking A or B | "For fc-5, go with option B." | `task note fc-5 --kind answer … --actor manager` logged |
| S5 no false wake | charter present, `fc-4` in progress in a lane with a working claude | "OK, let me know when it's done." | the reply contains none of the phrases `the_skill_promises_no_wake_while_none_exists` forbids (plus "let you know when"); at most two `task list` calls |
| S6 actor | any | any write | every write in the log carries `--actor manager` (checked in every scenario) |

## Baseline

Not yet run. The lane that wrote the skill (2026-09-25) could not dispatch
subagents, so the skill text was written before its baseline, the reverse of
the order writing-skills asks for. Run the baseline next, and if it passes a
scenario, that scenario isn't testing anything: make it harder before trusting
the with-skill result.

Expected baseline: S1 and S2 edit the repository, S3 is not recorded, S5 says
"I'll let you know".

| Run | S1 | S2 | S3 | S4 | S5 | S6 | Rationalizations, quoted |
|---|---|---|---|---|---|---|---|

## With the skill

| Run | S1 | S2 | S3 | S4 | S5 | S6 | Notes |
|---|---|---|---|---|---|---|---|
