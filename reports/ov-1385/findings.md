# ov-1385 validation

Rebased with `git rebase --no-autostash "$FARCOOLER_BASE"` onto
`15d08bec00f81724410b19603d53684b4ff8de19`, which includes the local
`origin/main` at `729b7450e1e6f52152cfac2c6fab2242c53f6b79`. There are no
merge commits above the supplied base.

`red-green.log` records a controlled scheduling reproduction using the full
self-test. Temporarily restore the old no-lock fixture from the parent of
9bbb8c4e3 and delay the second job by one second (0.2 seconds already in the
old fixture plus 0.8 seconds extra): the overlap assertion fails because the
first job has already ended. Restore the synchronized fixture and insert the
same one-second delay before its second job: the full self-test passes. The
source was restored afterwards. This simulates delayed scheduling; it does not
claim the historical failure had this exact cause.

`self-test-streak.log` records ten consecutive foreground runs of
`CI=true ./scripts/lane-gate.py --self-test` with no synthetic load, including
full output, exit statuses, timestamps, and load readings at each boundary.
All ten passed. PATH placed Cargo and the native system Python before
`/usr/local/bin`; the sandboxed attempt could not execute `ps`, so the recorded
runs used system access for the process-guard checks. Earlier evidence and the
ruling on historical attribution are in `../ov-1372/findings.md`.
