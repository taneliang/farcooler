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
