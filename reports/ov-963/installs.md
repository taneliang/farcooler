# ov-963 B1: which installs sit below registry 60

Read-only. Sources: `git log`, `gh run list` and `gh release list` for this repository, the workflow files, and `plutil -p` on the installed Canary app's `Info.plist`. No database, daemon or app was opened.

## What shipped, by channel

| Channel | What the record shows |
| --- | --- |
| Stable | Nothing. `gh release list` is empty, the repository has no `v*` tag (one tag, `proto/ov-699`), and `promote.yml` has no dispatch run. Its only runs are four 0-second failures on push, from Aug 12 to 15, before the workflow loaded. |
| Preview | Nothing, for the same reasons. A Preview build is `v<version>-preview.N`, made only by Promote. |
| Canary | 555 successful `canary.yml` runs, Aug 12 to Oct 31. The ones before Oct 5 are push-triggered and titled by commit message, so I could not map them to a commit; the 291 from Oct 5 on are mapped below. The last two runs, Nov 2 and Nov 4, concluded `failure`, yet the app installed here is `canary f60fee4e6` (build 4557), so a failed run can still publish. |
| Local | Built on the owner's machines and lanes; each carries whatever its branch had. |

## Registry per Canary build

Counted from `MIGRATIONS` at each successful run's commit. Counts are builds, not people.

| Registry | Builds | Dates |
| --- | --- | --- |
| 28 to 40 | 85 | Oct 5 to Oct 14 |
| 41 to 50 | 151 | Oct 14 to Oct 26 |
| 51 to 54 | 42 | Oct 26 to Oct 28 |
| 56, 57 | 13 | Oct 28 to Oct 31 |
| 58 (c165eb123, failed) | 0 successful | Nov 2 |
| 60 (f60fee4e6, installed here) | not in the success list | Nov 4 |

Registries 39, 55 and 59 have no successful run; I did not look for why. Before Oct 5 the registry was below 28. `origin/main` is at 60. Registries 61 to 67 exist only on this branch.

## Which real installs sit below 60

**Preview and Stable: none can exist**, because no build ever shipped on either. Nothing in the repository says otherwise. I can't see App Store Connect or TestFlight, so a TestFlight external build from a hand-run workflow would be invisible to me. The owner should confirm there is none.

**Canary: any machine that last updated before Nov 4.** Sparkle updates are on a schedule and `SUAutomaticallyUpdate` is false in the plist, so an install updates when its owner accepts. Candidates, none of which I could read:

- The owner's other Macs and phones' paired runners.
- Remote runners installed from a Canary disk image's `Resources/dist/` Linux binaries. Those do not update themselves.
- `com.farcooler.FarCooler` and `com.farcooler.FarCooler.canary` data directories on this Mac, which are not `FarCoolerCanary`. I listed their names only.

For a runner that is below 60, the cost today is concrete. The previous build migrated it forward on start. This build refuses it (`startup.rs:89`, `engine_cutover.rs:212` admits 60 and 65 to 67 only), `prepare` and `recover` print `error: operation failed` (now a sentence, see B2) and the only way out is reinstalling an older build.

**Registries 61 to 64 are branch-only.** Only a Local or lane home can hold one, so they are not a release exposure.

## What a stepping-stone build would take

The registry-60 prefix of `MIGRATIONS` is unchanged between f60fee4e6 and this branch. The diff of `migrate.rs` only adds entries after 60 and splits out `apply_remaining`. That gives two ways to close the gap, smallest first.

1. **Ship nothing new; order the rollout.** Registry 60 is already Canary's head (`origin/main`, f60fee4e6). A runner that updates to it migrates to 60 on its first start. Publish the cutover build only after every runner the owner cares about has run a 60 build once, and say so in the release note. Cost: no code, but it depends on a person and on remote runners being updated by hand. Nothing in the product enforces it.
2. **Let the cutover build bring a runner from below 60 up to 60 itself.** Ordinary startup would run `migrate` for versions below 60 only, stopping at 60 (the prefix is identical, and `apply_remaining` already runs a chosen range), then hand off to the existing Prepare with the pinned `SOURCE60_SCHEMA_HASH` check as the gate. A catalog that drifted would still be refused by that hash, so nothing is weakened. Cost: a new bounded path in `startup.rs`, one test per old registry the fixtures can build (28 and 57 are enough to bracket), and a decision on whether it runs at ordinary start or only inside Prepare. Prepare is the safer place, because it has the baseline and the proof already.

I would take option 2 inside Prepare. It needs the orchestrator's ruling, because it widens what Prepare accepts. I did not build it.

## Not verified

- Anything about TestFlight or the App Store.
- Whether Canary's real catalog hash matches the pinned registry-60 hash. Only the version number was ever read, and not by me.
- Why registries 39, 55 and 59, and f60fee4e6's own run, are missing from the success list.
- The registry of the 264 runs before Oct 5. It is below 28.
