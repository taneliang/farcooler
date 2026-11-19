# ov-1372 validation

The no-lock check now starts two heavy jobs which each write a start marker and
wait for the other marker before ending. The wait has a bounded failure path;
the test checks both job results and that both starts precede either end. It no
longer depends on a 0.2-second thread-start delay or a 0.5-second job duration.

`self-test-streak.log` records 20 consecutive full lane-gate self-tests on the
merged base and the ticket's code, including exit statuses, full output, and
start, periodic, and end load readings. This includes the no-lock check, the
24-test landing-policy suite, process guard and landing benchmark tests. No
synthetic load was generated. The first attempt, `missing-cargo.log`, failed
because PATH omitted Cargo; the PATH was corrected before the streak. An earlier
run was interrupted while correcting the fixture and is in `interrupted.log`.

The original JDK rejection's actual cause remains unknown. The earlier
`../ov-1361/probe-reproduction.log` shows that a fake Java process delayed beyond
0.5 seconds is rejected. In this ticket the fixture Java executables deliberately
sleep for one second if launched; the fixed fixture supplies their results without
launching them, and the full self-tests pass the JDK selection and preflight
assertions. This reproduces the timeout condition on the repaired fixture rather
than attributing the historical failure to it. Production probe deadlines remain
unchanged.

The original 24-test failure's test name and traceback were not retained, so its
historical cause remains unknown. `../ov-1361/fold-reproduction.log` records 20
passing runs of that suite on the affected fold; the full streak here also runs the
24-test suite against the fixed code. Under the ticket's ruling these are
reproductions, not proof of a historical attribution.
