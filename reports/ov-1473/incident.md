# Nov 26 Uncertain steps (ov-1473)

I read the Canary runner’s `farcoolerd.log` for 00:45–03:15 Singapore time on Nov 26, 2026 (16:45–19:15 UTC on Nov 25). The investigation used the log. The recovery tests used a scratch home and local repositories.

- **Landing group 7045 (ov-1428 and ov-1468).** At 02:45:01 the runner recorded “push outcome unknown” for effect `wf1:837d1508…`. Its recovery at 02:45:23 found that the send had been admitted and could not establish whether it landed. The log does not identify the transport failure or say whether the target accepted the commit.
- **Builds ov-1412, ov-1426, ov-1434, ov-1435 and ov-1465.** The cause for each build could not be determined from the runner log. Two effects, `wf1:22a35179…` at 03:01:22 and `wf1:16eaa92a…` at 03:10:57, reported that a native agent left a process running outside its group. Recovery for those effects and others reported missing files. These entries name effect IDs rather than ticket keys, so they do not establish which builds they affected or explain all five. The log also records Git timeouts during the window, but does not establish a causal link between machine load and the six Uncertain steps.

## Landing-gate rerun

The failed landing candidate included ov-1441's review-limit test, which called `crate::plan_run::run_of` from the Engine and raised the recorded Engine-to-Board dependency count from eight to nine. Substituting that test file into a scratch copy of the ov-1473 tree reproduced the exact failure. Substituting the file from ov-1441's fix (`eaf6007fe`) passed the layer check at the existing count of eight. The ov-1473 branch also passes at eight.
