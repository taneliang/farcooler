# ov-1366 validation

The adopted branch's `../ov-1361/final-streak.log` records 20 consecutive
successful lane-gate self-tests at natural machine load, including full output,
exit statuses and 30-second load samples. No synthetic load was generated.
The ticket's ruling accepts natural load; it does not require the previously
requested above-15 threshold.

The last review findings and their limits are recorded in
`../ov-1361/findings.md`: the original 24-test failure's traceback was not
preserved and its cause remains unknown; a controlled probe reproduction shows
only a plausible mechanism, and 20 runs of the affected fold passed. The
prior streak contains readings below 15, explicitly disclosed there. These
records do not establish the original failure's cause.

After merging the requested base, `natural-load.log` records three further
successful complete self-tests, with start and end load readings. The fourth
was deliberately interrupted because the existing 20-run streak already
satisfied the ruling; its traceback is retained and is not counted as a pass.
The full self-test includes process guard, landing policy and landing benchmark
tests. The native system Python was placed first on PATH as in the earlier runs.

Fix round 1: `merged-streak.log` records 20 consecutive successful full
self-tests on merged code at SHA `62648c979cf83fd4a4fd314f97dd73d8aa025ad2`.
It includes every run's output, exit status, start and end load, and 30-second
periodic load samples. The natural one-minute load varied; no synthetic load
was generated. This evidence supersedes the three-run post-merge check above.
The subsequent commit only adds this report and log.

The historical JDK rejection and 24-test error remain unverified for the reasons
above. The reviewer requires an explicit owner revision if their causes cannot
be established; a revision has been requested and must not be assumed.

Fix round 2: the process-guard bystander reads a parent-owned pipe until EOF.
An outer finally closes the pipe and waits for the bystander even if owner
startup or its cleanup raises; parent process exit also closes the pipe. The
fixture remains alive through the assertion without a fixed time limit.

`fix-round-2-streak.log` records four passes followed by a failure in the
no-lock overlap assertion on run five. There was a roughly 30-minute gap
between periodic samples during that run; the log does not establish why.
`fix-round-2-retry.log` records a fresh 20 consecutive full self-test passes on
code SHA c8dec951afeae22debdcf5a4b084998dc3d5d125, with start, periodic and
end load samples and exit statuses. No synthetic load was generated.

The original JDK rejection and 24-test error remain unverified. The prior
request for an explicit acceptance revision has not been granted in the ticket
record; inherited acceptance line 1 remains open pending owner revision or
original diagnostics.
