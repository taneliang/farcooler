# Flaky-test quarantine (ov-1229)

`.config/quarantine.txt` lists tests that flake and are being fixed. Each line names a test, the ticket
that tracks its fix and an expiry date (the format is documented at the top of that file). It starts empty.

**Adding a test** is a one-line edit, made when the flake's ticket is filed. Nothing opens a PR, branch or ticket for you.

**What a listed test gets.** CI's full Rust run and the lane gate both run
`python3 scripts/quarantine.py run -- cargo nextest run --profile ci ...`. A listed test that fails is rerun,
up to twice. If a rerun passes, the run prints `flaky (quarantined): <binary> <test> ... (<ticket>)` and stays green. If it fails
every rerun, the run fails as before. A test that is not listed gets no rerun. With an empty list the command runs unchanged.

**Name resolution.** Names are checked where a build exists. `quarantine.py run` resolves every listed
`<binary-id> <test-name>` against `cargo nextest list --workspace --message-format json` before testing, in CI's Rust job and
in the lane gate, and refuses to run on a name that is not there. That is what nextest will run, so every way of declaring a
test is covered: inline modules, file modules (`mod x;`), `#[path = "..."]` modules (also inside an inline module),
integration tests and custom `[[test]]`/`[[bin]]` targets, and macro-generated tests. Doctests are not in
nextest's inventory and cannot be quarantined.

**The lint.** `scripts/quarantine.py --lint` (the wire job, no build) fails on a malformed line, a duplicate or an entry on or
after its expiry date. With no inventory it does not judge names, so it never rejects a valid inventory-only entry; give it
`--inventory FILE` (nextest's JSON) or `--nextest` to match names too. `--source` is the fallback source scan: it follows
`mod x;` and `#[path]` from each Cargo target's root and needs the exact binary id and full name to be a test function. It cannot see
macro-generated tests, `[[test]]` paths or `#[path]` inside an inline module.

**Removing an entry** when the fix lands is a one-line deletion. To keep one past its expiry, edit the date in the ticket's say-so.

`python3 scripts/quarantine-test.py` is the self-test: the lint's cases, and real nextest runs of a scratch crate
(a flake that passes on rerun, one that fails every rerun, an unlisted failure, a flake beside a real failure).
