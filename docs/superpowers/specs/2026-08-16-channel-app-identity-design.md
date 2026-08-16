# A channel's app is its own app

Four channels ship four apps. On iOS they already have four bundle identifiers,
four keychain groups and four URL schemes. On macOS they have one of everything,
and on both platforms they have the same name and the same icon — so the only
way to tell a canary from the build you depend on is to open it and read
Settings.

This makes a channel's app distinct where a person can see it, and distinct
where the operating system can collide it.

Sparkle auto-update is deliberately **not** in this document. It is designed on
top of this one, because an updater's feed URL, its signing key and the meaning
of "replace the app in place" all reference the identity established here.

## What is wrong today

`apps/macos/Resources/Info.plist` names the app `com.farcooler.FarCooler` for
every channel, and `com.farcooler.daemon.plist` labels the login agent
`com.farcooler.daemon` for every channel. So on macOS:

- A canary build installs **over** the stable one. Same identifier, same
  `Far Cooler.app`, same place in `/Applications`.
- Two channels, if they could coexist, would register **one launchd label**.
  `SMAppService` would let the second registration quietly replace the first,
  and which daemon starts at login becomes a question of install order.
- Preferences are one domain, so a canary experiment rewrites the settings of
  the app someone works in.

And on both platforms every channel draws the same bear. The iOS apps are
already separate installs; they are simply indistinguishable on the home screen.

## The shape

One new fact per channel, asked of the file that already owns this kind of
answer. `scripts/version.sh` gains three:

| Answer | stable | canary |
| --- | --- | --- |
| `app-suffix` | *(empty)* | `.canary` |
| `app-name` | `Far Cooler` | `Far Cooler Canary` |
| `app-name-short` | `Far Cooler` | `FC Canary` |

Three answers rather than one because the alternative is the same three mappings
written into `build-app.sh` and `generate-project.py` separately, which is
exactly the drift `version.sh` exists to prevent — it is already the single
source for the channel itself, the CLI binary name and the URL scheme.

`app-name-short` exists for one reason: iOS truncates a home screen label at
roughly twelve characters, so `Far Cooler Canary` renders as `Far Cooler C…`.
macOS has no such limit and shows the full name in the menu bar.

Everything else derives:

| | stable | canary |
| --- | --- | --- |
| macOS bundle id | `com.farcooler.FarCooler` | `com.farcooler.FarCooler.canary` |
| macOS bundle | `Far Cooler.app` | `Far Cooler Canary.app` |
| macOS display name | Far Cooler | Far Cooler Canary |
| iOS bundle id | `com.farcooler.ios` | `com.farcooler.ios.canary` *(already true)* |
| iOS display name | Far Cooler | FC Canary |
| LaunchAgent plist | `com.farcooler.daemon.plist` | `com.farcooler.daemon.canary.plist` |
| LaunchAgent label | `com.farcooler.daemon` | `com.farcooler.daemon.canary` |

Stable keeps every bare name, for the reason it keeps the bare bundle
identifier, the bare CLI name and the bare URL scheme: those are what existing
installs already answer to, and renaming them orphans people rather than
updating them.

**Preferences partition for free.** `UserDefaults` keys off the bundle
identifier, so changing it changes the domain with no code at all. The
consequence is a real one and is not a bug: a canary install starts with default
settings rather than inheriting the stable app's.

## The icon

One source of truth stays one source of truth:
`apps/shared/Assets.xcassets/AppIcon.appiconset/AppIcon.png`, 1024×1024, the
bear.

A new `scripts/icon-label.swift` reads it, draws a diagonal banner across the
bottom-right corner — rotated 45°, filled in the channel's color, the channel
name in the same near-black as the sunglasses — and writes a new PNG. Stable is
passed through **byte-identical**: no banner, no re-encode.

Swift and CoreGraphics rather than ImageMagick, because every consumer of this
script is already a macOS runner where Swift exists, and adding a `brew install`
to a path that currently needs none is a cost with no return.

Colors are per channel and are doing more work than the text: at 32 points —
Cmd-Tab, the menu bar, a Finder list — the word is unreadable and the color is
the entire signal. So they are named exactly, and chosen to stay apart from each
other and from the cream backdrop:

| Channel | Banner | Text |
| --- | --- | --- |
| canary | `#E8A21C` amber | `#16130B` |
| preview | `#3B6FD4` blue | `#FFFFFF` |
| local | `#6E6E73` grey | `#FFFFFF` |

### Where the output may be written

**Never into the tracked asset catalog.** This is the constraint the whole
arrangement is shaped around.

`version.sh channel` answers `local` the moment the working tree is dirty. So
writing a generated icon into a tracked path would mean that generating the
canary icon makes the tree dirty, and every build step *after* that point
believes it is building `local` — a canary-labeled app installed at local's
identifier, with local's LaunchAgent, silently. The failure has no error
message and looks like a build that worked.

So output goes only where git already ignores:

- iOS: the shared catalog is **copied** to
  `apps/ios/build/Assets.xcassets/` and the labeled PNG replaces `AppIcon.png`
  inside the copy, so `Contents.json` and every other asset travel unchanged and
  the shared catalog is never written to. `generate-project.py` points the
  project at the copy.
- macOS: `apps/macos/build/`, feeding the `.iconset` that `build-app.sh`
  already turns into an `.icns`.

### Not Android

Android has its own mipmap icons and no release pipeline. The same generator
will serve it the day that changes; building for a channel that does not ship is
work with no reader.

## What changes, file by file

- **`scripts/version.sh`** — three new answers, each a `case` on the channel it
  already computes.
- **`apps/macos/build-app.sh`** — stamp `CFBundleIdentifier`, `CFBundleName`,
  `CFBundleDisplayName` and the LaunchAgent `Label`; name the output bundle
  `$(version.sh app-name).app`; run `icon-label.swift` before building the
  iconset.
- **`apps/macos/Resources/com.farcooler.daemon.plist`** — copied into the bundle
  under the per-channel filename, its `Label` stamped to match. `SMAppService`
  addresses the daemon with `BundleProgram`, so the job keeps pointing inside
  whichever bundle registered it.
- **`apps/macos/Sources/FarCooler/ServiceRegistration.swift`** — derive
  `plistName` from the channel already stamped in the app's own Info.plist
  instead of hardcoding `com.farcooler.daemon.plist`.
- **`apps/ios/generate-project.py`** — set the short display name; point
  `ASSETCATALOG` at the generated catalog. The bundle identifier is already per
  channel.
- **`.github/workflows/canary.yml`, `.github/workflows/release.yml`** — both
  reference the literal path `apps/macos/build/Far Cooler.app` when signing and
  packaging. That path now depends on the channel and becomes
  `$(scripts/version.sh app-name).app`. Missing this breaks both workflows at
  the signing step.

### To verify before writing code

`crates/daemon/src/paths.rs:11` documents the daemon's home as
`~/Library/Application Support/com.farcooler.FarCooler`. The channel work
already partitioned the daemon's runtime directory, so this is very likely
already per channel — but a path two channels could share is not something to
assume, and it is the first thing the plan checks.

## How it is verified

**`scripts/version-test.sh`** gains cases for the three new answers, in the
scratch repositories it already builds: stable is bare on all three, each other
channel has its own, and no two channels agree.

**A new icon test**, in the shape of `workos-test.sh`: stable's output is
byte-identical to the source; canary, preview and local each differ from the
source and from each other; an unknown channel is refused rather than silently
unlabeled.

**A real build**, because plists are where assumptions go to die. Build the Mac
app as canary locally and read back from the assembled bundle: its identifier,
its display name, its LaunchAgent label and filename, and that the `.icns`
differs from stable's. The same technique confirmed the keychain group and the
URL scheme earlier today, and in both cases the built artifact was the only
place the truth was visible.

## What this does to an existing install

Stable is untouched: same identifier, same name, same paths, no migration.

The canary dmg published before this change installs as `Far Cooler.app` under
the **stable** identifier. After this lands, the next canary installs beside it
as `Far Cooler Canary.app`, and the old one remains — looking exactly like the
stable app while being a canary build. So the release note for the first
labeled canary has to say: delete the canary you already installed, and if it
ever registered the daemon, switch that off before deleting, or a launchd job
under the old shared label outlives the app that created it.

## Deliberately not here

- **Sparkle.** Its own spec, on top of this one.
- **Android channel icons.** No release pipeline to carry them.
- **A channel switcher in the app.** Four installable apps make switching a
  download, which is enough until someone asks otherwise.
- **Migrating preferences between channels.** A canary starting from defaults is
  correct; importing the settings of the app someone depends on is how a canary
  reaches into stable's state, which is the thing every other partition here
  exists to prevent.
