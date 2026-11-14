# Flaky-test quarantine (ov-1229)

`.config/quarantine.txt` lists tests that flake and are being fixed. Each line names a test, the ticket
that tracks its fix and an expiry date (the format is documented at the top of that file). It starts empty.

**Adding a test** is a one-line edit, made when the flake's ticket is filed. Nothing opens a PR, branch or ticket for you.

**What a listed test gets.** CI's full Rust run and the lane gate both run
`python3 scripts/quarantine.py run -- cargo nextest run --profile ci ...`. A listed test that fails is rerun,
up to twice. If a rerun passes, the run prints `flaky (quarantined): <binary> <test> ... (<ticket>)` and stays green. If it fails
every rerun, the run fails as before. A test that is not listed gets no rerun. With an empty list the command runs unchanged.

**The lint.** `scripts/quarantine.py --lint` (the wire job) fails on a malformed line, a duplicate, an entry on or after its expiry
date, or a name that matches no test. The name check is static, so it needs no build: it reads each crate's source the way nextest names tests and requires the exact
binary id and full test name to exist as a `#[test]` function (`farcooler-tmux` for a unit test, `farcooler-daemon::a_foo`
for an integration test file). A typo, a rename, a wrong module path or a plain non-test function fails the lint.

**Removing an entry** when the fix lands is a one-line deletion. To keep one past its expiry, edit the date in the ticket's say-so.

`python3 scripts/quarantine-test.py` is the self-test: the lint's cases, and real nextest runs of a scratch crate
(a flake that passes on rerun, one that fails every rerun, an unlisted failure, a flake beside a real failure).
