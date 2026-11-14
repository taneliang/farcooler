# ov-1186 — sharing local build work with CI

14 November 2026. Source baseline: `54aa2ec89382e729e592e8fe12320d52e35fb853`.

## Decision

Choose **native caches plus nextest archive transport**, with the existing
ov-1117 provenance system as the only eventual test-result reuse layer. Finish
that approach across Rust, Apple, Android and web; do not introduce Nx or Turbo
as a second dependency graph. The branch experiment transfers a locally built
agent-core test archive through an immutable GitHub blob and executes all eight
tests on a separate hosted Mac without compiling them there.

Bazel is the strongest alternative for a complete hermetic cutover. It deserves
a real budget, not an opaque `cargo test` genrule called a migration. Its cost
here is native inputs and integration-test isolation, not editor support.
Buck2's publicly documented Apple support is a material obstacle for this repo.
Neither system makes tmux, SSH, simulator, network or UI tests hermetic merely
by wrapping the current commands. No measured whole-repository sub-five-minute
claim is supported by this spike.

## Evidence and measurement boundaries

- Baseline [CI run 43557233452](https://github.com/taneliang/farcooler/actions/runs/43557233452),
  downloaded using `gh run view 43557233452 --json jobs`; raw [step data](baseline-ci.json).
- [ov-1110 measurements](../ov-1110/investigation.md) and
  [tables](../ov-1110/tables.md): historical gate step work 2,806 s; measured cold
  workspace compilation 152.61 s versus warm 0.40 s; a different historical Mac
  Rust job spent 504 s compiling and 1,216 s executing the workspace test step.
  These are different samples. They are ceilings and bottleneck evidence, not
  an after-measurement of this branch or additive savings.
- This experiment: [seed manifest and measured commands](seed.json),
  [cold build](cold-archive.log), [warm build](warm-archive.log),
  [local tests](local-tests.log). Cold means a new target directory, with the
  registry and filesystem already warm. Queue time is outside the inner timer.
  Both archives use the same toolchain, target, deployment version and profile.
- Producer: arm64 Mac, macOS 27.0.1, rustc 1.98.1, nextest 0.9.148, three compile
  jobs, incremental and debug info off, deployment target macOS 15.0. Uses the
  repository machine queue, target files stay in this worktree. Ending load
  averages: 8.78 / 7.79 / 7.04. One tiny crate is not a representative gate.

| Baseline job | Wall minutes | Relevant work, minutes |
| --- | ---: | --- |
| Rust, macOS | 39.02 | full tests + shadow 26.07; this step mixes compilation/execution |
| Rust, Linux | 24.17 | full tests + shadow 15.58 |
| Swift, tests | 22.62 | AgentKit 2.75; Mac tests 16.07; Rust library 1.57 |
| Swift, build | 22.37 | app build 20.37 |
| Android | 19.53 | Robolectric 9.35; release APK 3.00; native core 2.42 |
| Wire | 3.98 | includes test helpers and retry/lease tests |

## Comparison

Savings are **per landing**, separating local gate and CI. `p` is the fraction
of matching reusable tasks, which has not been measured fleet-wide. Zero is
the honest default for unmeasured fleet savings. Bounds below describe avoided
executed work, not necessarily elapsed critical-path time: CI jobs overlap and
queueing remains. Setup/upkeep figures are engineering estimates for one owner
with agent assistance, not vendor benchmarks or measured delivery times.

| Approach | Gate minutes saved | CI minutes saved | Risks, missed inputs, costs |
| --- | --- | --- | --- |
| **Native compile caches**: sccache remote S3/R2; Xcode compilation cache; Gradle remote build cache; npm downloads. Add native nextest archives for completed test builds. | Fleet unmeasured. Existing cold/warm workspace observation bounds unchanged local compile reuse at **2.54 min**, before transfer; zero test execution saved by sccache. This spike measures an 8.775 s archive build and 0.308 s warm archive. | **Measured agent-core: 0.336 min** saved versus a fresh hosted target, or **0.297 min** after local upload cost. Fleet unmeasured. Historical Mac Rust compile component bounds reuse at **8.40 min**, less transfer/linking/misses. Does not remove its **20.27 min** execution. | Compiler/SDK/target/features must match; sccache cannot cache linker-invoking units or incremental Rust. Build scripts and external macro reads can escape keys. Audit depfiles and clean rebuild samples. Estimate **1–2 weeks** remote infrastructure + Rust/Gradle, **1–2 further weeks** Apple validation, **0.5–1 day/week** upkeep at first. |
| **Task results**: Nx / Turbo opaque cargo, xcodebuild, Gradle tasks; or extend ov-1117 + ov-1116 + ci-changes. | Exact duplicate successful full gate ceiling **46.77 × p min** of historical step work, minus validation. On changed input it is **0** for whole-tree receipts. Per-task inputs improve hits but require consumer coverage. | Matching Mac job ceiling **39.02 × p min** in this baseline, minus validation; **0 Linux result reuse from a Mac pass**. CI-produced Linux results can reuse only within their own validated environment/trust domain. | A missing declared input can replay false green in all three implementations. Tool versions, fixtures, generated inputs, feature/target/env/native state and producer authority belong in the key. Independent uncached runs + planted omitted-consumer failures catch errors. Prefer extending existing provenance (**1–2 weeks**) over Nx/Turbo graph + native-input ownership (**2–4 weeks**); **1–2 days/week** initial upkeep. |
| **Hermetic build graph**: Bazel / Buck2, remote action/test cache and optional execution | For unchanged hermetic actions, the same **46.77 × p min** work ceiling applies; no repo-specific Bazel/Buck2 speedup measured. Dynamic integration tests must still run, so this is not a forecast. | Matching hermetic actions may skip compilation **and tests**. Platform-independent actions can share; Darwin/Linux execution actions remain separate. Existing **20.27 min** Rust execution cannot be promised away until its hermetic subset is identified. | More robust action-input model and sandboxing, but undeclared host tools/SDKs, native libraries, clock/network and non-isolated tests still break hermeticity. Missing file reads should fail in a sandbox; run cache-disabled audits for leaks. **Bazel 10–16 engineer-weeks**, **Buck2 14–24+**, breakdown below; **1–2 days/week** rule/toolchain upkeep initially. |

### Native systems and opaque task caches

[sccache's Rust caveats](https://raw.githubusercontent.com/mozilla/sccache/main/docs/Rust.md)
exclude incremental compilation and linker-invoking crate types. Files read by
procedural macros need particular care. Therefore “restore all Rust compilation”
is not a defensible sccache estimate. A nextest archive complements compiler
caching by preserving completed, linked test binaries; it preserves no verdict.
[Nextest documents](https://nexte.st/docs/ci-features/archiving/) source remapping,
separate build/run hosts and the requirement to supply matching source/fixtures.

[Gradle's build cache](https://docs.gradle.org/current/userguide/build_cache.html)
is a task-output cache, including eligible test tasks; declare native library
inputs, JDK, Android plugin, SDK and Robolectric dependencies before trusting
cross-machine hits. Configuration caching saves configuration, not test
execution. [Xcode's compilation caching](https://developer.apple.com/documentation/xcode-release-notes/xcode-26-release-notes)
addresses compilation, not arbitrary UI test verdicts. Remote Apple caching
needs validation against this project's exact Xcode 27, signing and simulator
setup; this spike did not provision an Apple remote service. DerivedData restore
is less portable than content-based compiler caching. npm's cache primarily
avoids fetching packages; it is not a cache of successful web tests.

[Nx task caching](https://nx.dev/docs/features/cache-task-results) and
[Turbo caching](https://turborepo.dev/docs/crafting-your-repository/caching)
can wrap non-JS commands, but cannot infer every Cargo build-script, Xcode SDK,
Gradle JNI, include-file or test-fixture input of opaque commands. Both need
explicit outputs and native/environment inputs. Their cache transport does not
resolve the provenance problem ov-1117 already tackles. Introducing either
would duplicate this repo's task selection and evidence ownership.

### Bazel and Buck2: real replacement scope

| Area | Bazel | Buck2 | Work specific to this repository |
| --- | --- | --- | --- |
| Rust | [rules_rust](https://bazelbuild.github.io/rules_rust/) with crate_universe for third-party resolution and build scripts | Rust in public prelude; [official support table](https://buck2.build/docs/about/language_support/) calls setup easy | 19 workspace crates; serde/prost generators, bundled SQLite, aws-lc, proc macros, Go tunnel, C ABI/staticlibs, wasm and mobile cross targets, helper/default feature variants, doctests and clippy equivalence |
| Apple | [rules_apple](https://github.com/bazelbuild/rules_apple) plus [rules_swift](https://github.com/bazelbuild/rules_swift) build libraries/apps/tests | Official support table marks Swift/Objective-C unavailable despite prelude entries | App Groups, extensions/watch, entitlements, Sparkle, package resources, universal bundles, signing/notarization, simulator tests; Apple SDK still requires a Mac |
| Android | [rules_android](https://github.com/bazelbuild/rules_android), [rules_kotlin](https://github.com/bazel-contrib/rules_kotlin) | Public Kotlin/mobile prelude requires complex setup | Compose compiler/toolchain, JNI artifacts/ABIs, Robolectric resources, release shrinking/signing and instrumented bundle parity; wrapping gradlew does not replace Gradle |
| JS/web | [rules_js](https://github.com/aspect-build/rules_js) | Budget custom/community integration; public support table gives no first-class JS path | npm lockfiles, wasm bindings, Cloudflare dry-run packaging, browser fixtures, Go wasm tests and size checks |
| Remote cache/execution | [Bazel action cache and CAS](https://bazel.build/remote/caching) | [REAPI support](https://buck2.build/docs/users/remote_execution/) includes tested BuildBuddy/BuildBarn/EngFlow configurations | Separate execution-platform keys, isolated write permissions, pinned toolchains; test runner integration remains required |

These are primary-source capability checks, not proof that any rule set builds
Far Cooler unchanged. Bazel's Apple/Swift and Android rules are independently
maintained projects with version compatibility constraints. Buck2's Rust engine
and graph model are credible; its documented external Apple gap is why this
all-platform repo should prefer Bazel if it replaces native tools. Editor
integration, rust-analyzer, Xcode editing and Android Studio editing contribute
**zero** to these estimates, per the owner's ruling.

[Bazel's test contract](https://bazel.build/reference/test-encyclopedia) requires
declared dependencies and a reproducible runtime; its test cache is useful only
when the tests honor that contract. [Buck2's test runner interface](https://buck2.build/docs/rule_authors/test_execution/)
is a separate concern from its build action cache: the open-source default is
a simple command executor. Budget test discovery, scheduling, results and
hermetic result-cache integration explicitly; Meta's Tpx capabilities are not
automatically supplied to this repository by installing Buck2.

The [tracked-file inventory](repository-size.json) gives the estimate's scale:
1,368 crate files, 691 Mac files, 256 iOS files, 564 Android files and 473 shared
Apple files, in addition to services and build scripts. These are file counts,
not source-line counts or build targets; the integration boundaries above drive
the estimate more than the number of leaf modules.

Bazel migration estimate, one engineer with agent assistance (elapsed planning
ranges, not parallel staffing arithmetic):

| Replacement boundary | Estimate | Required deletion at that boundary |
| --- | ---: | --- |
| Pin toolchains, cache ACLs, baseline inventories, rule proof | 1–2 weeks | experimental wrappers once canonical targets exist |
| Entire Rust/Go build graph including native, wasm/mobile outputs and test runner | 3–4 weeks | workspace/per-crate Cargo manifests and lockfiles, Cargo/nextest build invocations, Cross.toml and build-profile configuration; dependencies become canonical pinned Bazel declarations, not a second Cargo graph |
| Apple app targets, resources, tests, bundles and release signing | 3–5 weeks | SwiftPM manifests, generated build project/build generator and replaced shell build recipes; shipping checks become Bazel actions |
| Android + web/relay/site and packaging equivalence | 3–5 weeks | Gradle settings/build files/wrapper, npm task scripts replaced by Bazel targets, old CI/gate commands; preserve package metadata required at runtime, not duplicate recipes |
| **Complete cutover** | **10–16 weeks** | one authoritative build graph and one entry point; no permanent Cargo/Bazel or Gradle/Bazel parity maintenance |

Buck2 adds roughly **4–8+ weeks** for externally usable Apple/toolchain rules
and less established JS integration; it is not a faster migration just because
the engine is written in Rust. This estimate is explicitly uncertain and needs
a native-link + signed Apple bundle proof before commitment. Under either
replacement plan a moved family is built only by its new system; consumers use
the new family's artifacts. No project is kept building both ways indefinitely.
If the single-engine rule requires an all-repository switch on one date, prepare
the work on a migration branch and make that switch atomically.

### Hosted and self-hosted backends

- [BuildBuddy](https://www.buildbuddy.io/docs/quickstart/) offers hosted Bazel
  cache/execution and [read-only keys](https://www.buildbuddy.io/docs/guide-auth/).
  Smallest operations burden; obtain a current storage/egress/concurrency quote
  before committing spend. No price assumption is used in the recommendation.
- [NativeLink](https://docs.nativelink.com/) implements REAPI cache/execution for
  Bazel and Buck2. Consider its hosted offering or deploy the open-source
  service; cache-only rollout does not require a worker fleet. Mac remote
  execution still needs suitably provisioned/licensed Mac hosts.
- [bazel-remote](https://github.com/buchgr/bazel-remote) is a smaller self-hosted
  cache (HTTP/gRPC, disk limit/GC and object-store backends). It does not itself
  provide an execution fleet. Budget TLS, authentication, availability, quota,
  backups/eviction and monitoring. Native sccache uses an S3-compatible backend;
  Gradle uses its HTTP cache protocol, not a Bazel endpoint automatically.

## Trust and missed inputs

The owner Mac is a trusted **producer**, not an arbitrary PR writer. A cache
hash detects corruption; it does not prove a compiler actually compiled the
claimed source or a test actually passed. Cache poisoning of a test executable
can execute attacker code, and poisoning a task verdict can silently skip a
failure. Production needs authenticated producers and server-enforced namespace
ACLs. Let the owner's machine and protected CI write a trusted namespace;
untrusted PRs get a separate namespace with no write/read escalation to secrets
or authoritative verdicts. Credentials must be scoped and rotated. Signed
receipts require a trusted key rooted outside PR-controlled content.

Match source and generated input bytes, graph and command, compiler/SDK/native
tool versions, feature set, target triple, profile/flags, relevant public env,
runner ABI and external fixtures. Linux never accepts a Darwin binary or a
local Mac pass as its Linux proof. Matching Mac toolchain/environment may
reuse a local verdict only through ov-1117's complete successful prospective
evidence; the experiment **does not** establish that equivalence. Compatible
Mach-O test binaries may still run on a different Mac OS version, as tested by
this archive experiment, because CI executes the tests again there.

No scheme detects every missed input from a key alone. Keep cache-disabled
reference runs (nightly and a sampled main subset), compare test inventories and
outputs, and plant changes in fixtures, generated sources, features, SDK/native
libraries and out-of-graph consumers. A disagreement revokes the cache namespace
and returns that family to uncached execution. Bazel's sandbox moves many missed
file reads from false hits to immediate failures, but host tools and external
services still need explicit platform inputs or non-cacheable tests. Nx/Turbo
and a thin receipt layer depend more heavily on audits and differential runs.

## Branch prototype

Only `.github/workflows/ov1186-cache.yml` is added; **ci.yml and main checks are
unchanged**. It triggers only on this named branch when the seed or experiment
code changes. Its GitHub token is contents-read-only. The cache transport is a
1,275,279-byte nextest archive stored as an immutable GitHub Git blob
`a86c41eb70188f4fedd260539a4cede1f2f711e8`; its SHA-256 is pinned in `seed.json`.
No binary or credentials are tracked in the Git tree. This uses existing GitHub
authentication; no remote-cache secret/account was created.

Git blobs are a disposable experimental transport. Unreferenced objects can
be garbage-collected, there is no cache discovery/TTL service, and production
must not use this endpoint as its cache backend. If unavailable/corrupt, this
experiment fails; a production cache miss must rebuild normally. The same API
permits authenticated local or CI writers; this branch gives CI only read access
because the test needs only one-way reuse.

The helper hashes tracked working bytes/modes conservatively, refuses unknown
untracked inputs outside the experiment, checks SHA-256, target, architecture,
package and nextest version, then runs the archived tests. Its scope explicitly
excludes this report and experiment workflow so recording observations does not
invalidate the build. That exclusion and the branch-controlled seed are why
this is **not a reusable production receipt or required check**. It does not
capture ignored/ambient native inputs, attest the producer, skip test execution,
or cover doctests. Agent-core has eight nextest tests; doctests are checked
separately. Implement real provenance by extending ov-1117, not this helper.

Reproduce, with an empty `target/ov-1186/measured-build`:

```sh
python3 scripts/heavy -- python3 reports/ov-1186/prototype.py seed
python3 reports/ov-1186/prototype_test.py
actionlint .github/workflows/ov1186-cache.yml
python3 reports/ov-1186/prototype.py upload
python3 reports/ov-1186/prototype.py consume
```

The upload uses `gh api repos/taneliang/farcooler/git/blobs --input <payload>`;
the API accepts base64 and returns an immutable object ID
([GitHub documentation](https://docs.github.com/en/rest/git/blobs)). Credentials
are handled by gh; none are dumped into reports. Seed reruns deliberately refuse
an existing measured-build directory rather than mislabel a warm build cold.

### Observed results

| Local phase | Outer seconds | Tests |
| --- | ---: | --- |
| Cold archive build | 8.775 | build only |
| Warm archive build | 0.308 | build only |
| Local archive execution | 0.063 | 8 passed, 0 skipped |
| Upload immutable blob | 2.383 | transport only |
| Download, verify, execute locally again | 2.402 | 8 passed, 0 skipped; download/verify 2.246 s |

Hosted [run 43579040900](https://github.com/taneliang/farcooler/actions/runs/43579040900)
**passed** at commit `134a889660efd7e03265f612576f7ff562bb4a8c`.
The consumer was a separate hosted arm64 Mac running **macOS 15.8.2**, not the
producer's macOS 27.0.1. See [summary](ci-43579040900/summary.json),
[job/step data](ci-43579040900/run.json), [raw run log](ci-43579040900/run.log),
[reuse timer](ci-43579040900/consume.json), [cold timer](ci-43579040900/cold.json)
and the two [archived-test](ci-43579040900/ci-archive-tests.log) /
[cold-test](ci-43579040900/ci-cold-tests.log) logs. Both executed **8 passed,
0 skipped**. The reuse log contains no compilation; the independent comparison
compiled into a separate target. The [inventory comparison](inventory-comparison.json)
confirms identical test names in local, reused CI and independently compiled CI runs.

| Hosted phase | Outer seconds | Meaning |
| --- | ---: | --- |
| Fetch + verify + archived test run | **1.266** | includes 0.990 s download/verification and 0.208 s nextest invocation |
| Independent cold build + tests | **21.447** | same runner; Cargo reports 16.26 s for build phase including its resolution/download work |
| Avoided CI step work | **20.181** | 21.447 − 1.266; **0.336 min** |
| Avoided work after producer upload cost | **17.798** | derived 20.181 − 2.383; **0.297 min**, if the local gate already produced the archive |
| Queue to first job step | **762** | **12m42s**; real observation, not excluded from end-to-end discussion |
| Whole experimental job | **36** | includes both reused and deliberately cold comparison steps |
| Push to experiment verdict | **798** | **13m18s** including queue |

The gate saved **0** compilation seconds in this one-way experiment: it did the
original local build. The observed CI saving is not also counted as a gate
saving. A later gate reading a warm artifact is a separate opportunity, whose
fleet rate has not been measured. No main CI test result was skipped or changed.

The independent CI cold comparison uses a fresh target without Swatinem restore;
it includes registry work if needed. It is not the saving versus today's warm
production CI cache. Producer and consumer toolchain/image differences must be
read alongside the timings; the result proves artifact reuse, not compiler or
host-speed equivalence.

Download the evidence again with:

```sh
python3 reports/ov-1186/capture_ci.py 43579040900
```

Use a new output directory or remove only your previous downloaded capture
before repeating: gh intentionally refuses to overwrite existing artifacts.
The 14-day Actions artifact retention is not a long-term dependency; the JSON
timings and raw logs are committed with this report.

## Under five minutes: what can and cannot fit

“Under five minutes total” means local gate + remote queue/setup + CI verdict,
not summing overlapping job times and not counting an artifact hit as a green
test. This ticket's tiny crate measures the mechanism, not this target.
**At the observed runner contention, none of the change shapes can guarantee
five minutes: queue time alone was 12m42s.** The feasibility below assumes
capacity has been made available and addresses executed work separately.

| Change shape | Feasibility after rollout | Required cost/work |
| --- | --- | --- |
| Docs/site | Plausible with existing selection: baseline wire 239 s + changes 15 s leaves only 46 s for local work/startup/queue; no guarantee under contention | Keep cache hits small and reduce wire helper overhead; do not add an app build |
| Rust leaf crate | Plausible with warm native inputs, selected dependency/consumer tests and fast CI startup; archive transport alone cannot make today's full main suite fit | ov-1116 coverage proof + ov-1182 sharding, native cache integration and authenticated evidence; fleet after-measurement required |
| Apple UI/app-only | Usually cannot fit today: Mac test step alone is 964 s | ov-1183 test isolation/selection plus validated compiler caches; hardware/UI tests that vary across hosts still execute |
| Core/protocol/global toolchain change | Cannot responsibly promise <5 min: broad rebuild + platform tests and native integrations | Eliminate external-test bottlenecks or execute in a larger isolated fleet; Bazel improves action granularity but does not erase uncached critical paths |

## Rollout tickets and coordination

Proposed tickets below are **specifications**, not newly filed board items.
No duplicate cache-key, sharding, test-fixing or gate-parallelism work is done
here. Read ov-1182..1185, ov-1165, ov-1116 and ov-1117 before implementation.

| Proposed follow-up | Ownership / estimate | Acceptance and what it replaces |
| --- | --- | --- |
| Shared cache trust/bootstrap | new, 2–3 days | owner provisions bucket/ACLs; protected-CI identity + owner login; fork denial and poisoned-object drills; separate trust/platform namespaces; lifecycle/quota metrics; replaces experimental blob transport |
| Native Rust cache + local archive producer | new, 3–5 days, after ov-1165 and with ov-1182 | measure cold/warm on Mac and Linux, archive download versus rebuild thresholds, compiler hit/miss reasons, exact feature/toolchain keys; extend ov-1182's canonical archive creation so gate and CI use one producer; remove conflicting Rust cache paths after conversion |
| Publish trusted prospective evidence | extend ov-1117, 3–5 days | consume only complete authenticated matching evidence; modified/partial/stale/fork/environment mismatch rebuilds; keep failures immutable and logs; no second receipt format; selection graph remains ov-1116/ci-changes |
| Complete native caches across Apple/Android/web | coordinate ov-1183/1184, 1–2 weeks | Xcode 27 compiler cache and Gradle remote task cache validated against native/SDK inputs; npm downloads; each family fully replaces old caching stanzas, no second build graph; retain required tests |
| Fleet savings and five-minute budget | new, 2–3 days after rollout | at least 10 representative landings, p50/p95 including queue/setup/upload and both stages, hit rate/bytes, sampled uncached parity and planted consumer failures; rollback on incorrect hit or negative wall-time saving |

**Do not schedule a parallel Bazel migration while choosing the native path.**
If fleet evidence shows that native cache/selection ownership is the limiting
cost, explicitly replace this decision with a funded Bazel complete-cutover
project using the deletion gates above. Require its first Rust/native/mobile
proof to enumerate and run the same tests before deleting Cargo, rather than
keeping two systems forever. Buck2 would first need an externally usable signed
Apple app proof before it can compete for that decision.

### Owner-only provisioning example (not executed)

Use the owner's chosen account/region and an existing short-lived AWS profile;
no access keys belong in the repository. Create the bucket, block public access,
enable default encryption and add a lifecycle policy, then provision two
identities: owner SSO and protected-CI OIDC. IAM must grant only GetObject to
readers and GetObject/PutObject to trusted writers under the selected prefix;
untrusted workflows cannot assume the writer role. Object-store prefixes alone
are **not** an ACL.

```sh
# Owner fills these with their own resources, then runs from their account.
export AWS_PROFILE='<owner-sso-profile>'
export CACHE_REGION='<region>' CACHE_BUCKET='<globally-unique-cache-bucket>'
aws sso login --profile "$AWS_PROFILE"
if [ "$CACHE_REGION" = us-east-1 ]; then
  aws s3api create-bucket --bucket "$CACHE_BUCKET" --region "$CACHE_REGION"
else
  aws s3api create-bucket --bucket "$CACHE_BUCKET" --region "$CACHE_REGION" \
    --create-bucket-configuration LocationConstraint="$CACHE_REGION"
fi
aws s3api put-public-access-block --bucket "$CACHE_BUCKET" \
  --public-access-block-configuration BlockPublicAcls=true,IgnorePublicAcls=true,BlockPublicPolicy=true,RestrictPublicBuckets=true
aws s3api put-bucket-encryption --bucket "$CACHE_BUCKET" \
  --server-side-encryption-configuration '{"Rules":[{"ApplyServerSideEncryptionByDefault":{"SSEAlgorithm":"AES256"}}]}'
aws s3api put-bucket-lifecycle-configuration --bucket "$CACHE_BUCKET" \
  --lifecycle-configuration '{"Rules":[{"ID":"expire-cache","Status":"Enabled","Filter":{"Prefix":"farcooler/"},"Expiration":{"Days":30},"AbortIncompleteMultipartUpload":{"DaysAfterInitiation":1}}]}'
# Only after owner has installed scoped IAM/OIDC and lifecycle policies:
export SCCACHE_BUCKET="$CACHE_BUCKET" SCCACHE_REGION="$CACHE_REGION"
export SCCACHE_S3_KEY_PREFIX='farcooler/trusted/darwin-arm64/'
export CARGO_INCREMENTAL=0 RUSTC_WRAPPER=sccache
export SCCACHE_S3_RW_MODE=READ_ONLY
# Trusted producers use READ_WRITE only with a writer-scoped identity.
```

[sccache's S3 backend](https://raw.githubusercontent.com/mozilla/sccache/main/docs/S3.md)
supports scoped prefixes, read-only mode, AWS profiles and web-identity roles.
The provisioning follow-up must deliver reviewed IAM/OIDC/lifecycle commands
for the owner's selected resources before any production writer is enabled.

## Validation

Archive counterexamples reject source/target/package/schema/nextest mismatch,
Linux and wrong-architecture reuse, altered archive bytes and new untracked
source. A dirty tracked source changes the fingerprint. The consumer's positive
case would fail if verification rejected every archive. The
[mutation run](mutation-check.log) deletes each of six validation guards and
confirms every mutant fails the tests. Workflow lint and ci-changes self-test pass;
production checks and build inputs are untouched. Full CI and gate tests are
outside this research-only change's affected surface.

Acceptance: comparison and cost/risk table above; successful local-to-hosted
reuse with measured timings and matching test inventories; recommendation and
scoped follow-up list above. All three ticket acceptance items are covered.
