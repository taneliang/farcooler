# ov-1117 operational validation (2026-11-13)

This record supplements the historical fixture and failed preflight records in
`engine-gate-reuse.md`. The operational target is the canonical local lane
planner's **wire and doc-comments** jobs, including the real repository lints,
script suites and generated wire-type comparison. It does not measure an
application compilation, remote CI, or the historical 2,806-second opportunity.
No existing workflow's reuse setting is changed.

## Reviewed candidate and public prerequisites

The isolated checkout tests candidate
`4b1f7c069df506cab144c7d7eacb387e6d327214` against resolved base
`bb38bf01b8f1583db1e182d177111dc5febce35d`. The planner selects both jobs
without an `--only`/`--skip` override. Adapter SHA256:
`11b6871f77cc9ab0472dbb7c7314a188cd26b1a90e9e6afad0bea794353d51e4`;
prospective writer/checker SHA256:
`ea3cd7a1d469426b95a47d708a4c227ed03a053b01e34a2b7692d9180453960e`.
The final documentation commit does not change either implementation.

An independent reviewer inspected the adapter, capture/checker, tests, canonical
commands and the actual public input declaration before execution/reuse. The
declaration SHA256 is
`c36c5561b976298bac19a97b2ccbab96668d8970ae44f3f10eef26ade21a9425`.
There were no blocking findings within the documented local-receipt boundary.
The review is limited to these two jobs on this isolated runner/configuration;
it does not approve prerequisites for heavy jobs or general production reuse.

Both jobs declare the following executable names (the capture fingerprints their
resolved paths, bytes and executable modes, plus mandatory bash and python3):

```
python3 typeshare cargo rustc git sh env sed awk grep find sort head tail wc tr
mktemp mkdir rm cp cat chmod sleep date touch dirname basename diff xargs cmp
cut uname tee expr mv printf kill ps readlink stat perl which od true seq
plutil xcrun
```

The private curated bin supplies ARM Python 3.14.10, typeshare 1.13.4 and the
resolved installed Cargo/rustc executables. Git runs from Xcode's actual bin
directory, preserving its helper/template lookup. Child `python3` resolves to
the ARM interpreter; `/usr/local/bin` is absent. The reviewed declaration also
contains 2,963 explicit public native files per job: the installed Python
stdlib/runtime including bytecode, extension modules, site startup files and
libpython; discovered Homebrew dynamic dependencies; Xcode Git helpers and
templates; the frozen origin's packed refs; and the task-owned checkout's public
Git remote configuration. No arbitrary HOME/configuration trees are inventoried.
SDK and immutable-cache lists are empty: these light jobs consume source and
the declared installed runtimes, rather than prior build products or caches.
The installed typeshare version satisfies the conditional installer; there is
no Cargo installation or application build in the measured run.

The independent review identified wire's `git fetch origin main` as an additional
external input. Origin is a task-owned frozen bare clone, with main fixed at
`3f78754f936855710dfec920fe3d492f879b610d`, no loose ref overrides and no object
alternates. Its packed refs and checkout remote URL are fingerprinted. The gate
does not fetch changing upstream state. Source/history and generated outputs
remain subject to the canonical checks.

HOME is an initially empty private directory. The process receives only the
explicit public environment in the saved invocation. OS/shared-cache bytes and
unchanged external/system configuration remain controlled assumptions of trust
scope `cosmo-darwin27-arm64-private-home-wire-doc-20261113-v1`; no global
configuration or credentials are collected or changed. The review identified
these assumptions explicitly; the receipts do not claim hermetic execution.

## Independent counterexample validation

The independent reviewer reran all nine adapter fixtures and all sixteen original
capture/checker fixtures successfully. All four original guard mutations produced
the required assertion failures (coverage, numeric exit, prerequisite bytes and
tracked bytes). The existing fixtures include whole-job retries, immutable
failures/raw logs, dirty/untracked/candidate mismatch, workflow/policy/workspace/
environment misses, v1 rejection, moved HEAD, writer scope drift and malformed
receipt recovery. These fixture timings are excluded from operational metrics.

## Measurement procedure

First run the canonical gate with reuse disabled. After that succeeds, the
measurement-only enabled command uses definition pin
`ov1117-wire-doc-reviewed-enabled-v2` and includes `--enable-reuse` on **both**
its cold call and repeat. Its first call must execute both jobs because the new
definition/context cannot use the disabled command's evidence. Its repeat keeps
the same argv, definition, policy, workspace, base, source, input declaration,
environment and trust scope; only the operation binding changes.
"Cold" means no usable gate receipt in this definition's scope, not cleared
compiler or operating-system caches.

This distinction matters: configured publication pins command argv, so toggling
`--enable-reuse` while retaining an old definition pin would not model an engine
publication accurately. The disabled validation is separate from the enabled
cold/repeat comparison. Only the isolated measurement command is enabled; no
existing workflow is republished.

The driver samples one-minute host load every 0.5 seconds, refuses to start at
load >=20 and excludes a complete timing if any sample reaches 20. Adapter
metrics and whole-process elapsed times are recorded separately. No other task
is stopped and no application target/cache directory is cleaned. Gate receipts
and raw logs remain private and immutable; only the observations are committed.

## Disabled operational validation and excluded attempts

The clean default-disabled canonical run passed all 56 wire steps and both
doc-comments steps: required 2, selected 2, reused 0, numeric exit 0. Adapter
elapsed time was 287.592 s; whole-process elapsed time was 288.722 s. All sampled
one-minute loads were below 20 (4.238–12.007). Its private manifest is
`evidence/attempt-d62d380363864c7fb8bc3a0bdae7ac84/manifest.json` under
`/private/tmp/ov1117-validation-current`.

Two earlier attempts in that directory are retained separately. The first is a
numeric failure: a relocated Git symlink inferred nonexistent template paths,
so the canonical canary-baseline fixture failed. Removing that symlink and using
Git's actual installed directory fixed the environment; no application source
was altered. The next corrected run passed, but its load reached 25.065, so its
280.144-second adapter timing is excluded from the comparison. Neither attempt
supplies coverage to the enabled definition. The separate earlier Python/Cargo
preflight failures remain described in `engine-gate-reuse.md`.

Private `inputs.json`, `run.json`, `enabled-run.json`, per-call invocation metrics,
load samples, manifests and raw logs retain the audit trail. The corrected
disabled invocation descriptor SHA256 is
`5ed40116d3d48b3fddaf9bbf2e90e568d4c24a3a7133fb8e817a580fedb041a9`;
the independently reviewed enabled descriptor SHA256 is
`52b5413a3d6236cac3e8c11efadc3150130062df2670ecb2f92e54c6b44ce980`.
Per-call descriptors change only the operation binding within each definition.

## Qualified enabled cold/repeat results

Both calls passed with the same scope digest
`ad5f62c4842bf254bffdab0ea5f5cc366d814e8b3b4b336f0e25fe3c0cb426c8`.

| Invocation | Required | Reused | Executed | Adapter wall | Process wall | Load range |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Enabled cold | 2 | 0 | 2 | 259.974 s | 260.543 s | 3.773–8.827 |
| Same-definition repeat | 2 | 2 | 0 | 5.497 s | 6.052 s | 3.965–4.137 |

Repeat job hit rate is **2/2 = 100%**. Aggregate job hit rate across the qualified
pair is **sum(reused)/sum(required) = 2/4 = 50%**. Across all five adapter calls
in this follow-up session, including the Git failure, high-load success and
separate disabled validation, the rate is **2/10 = 20%**. Historical attempts
and tiny fixture invocations are outside those explicitly stated cohorts.

The observed wall saving is **254.478 s** in adapter metrics, or **254.491 s**
including process startup. All qualified load samples stayed below 20. This is
one observed local wire/doc pair; it establishes operational reuse but does not
estimate future hit frequency, application compilation savings, queue time
avoided or the historical 2,806-second opportunity.

The repeat created no job attempt. Its pointer still names cold manifest
`evidence/attempt-de9557a246494de69654a569f7f5d714/manifest.json`. The driver
verified that all 258 prior private evidence files, including failed manifests
and raw logs, retained their exact bytes. The standalone summary checker then
returned exit 0: `gate-summary-check: ok, the gate covers HEAD`.
The cold manifest's SHA256 is
`fa5e4d3c442cc5cf965f429de06065d2646e87d3f70433b728a187ad34c85c32`.

The independent reviewer verified the corrected disabled descriptor and enabled
descriptor before the enabled pair. This completes the isolated operational
validation required for ov-1117; production publication remains a separate
reviewed configuration decision.
