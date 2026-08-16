# Sparkle Auto-Update Implementation Plan

> **Corrections (added after execution — the tasks below are left as executed,
> not edited):**
>
> - **Task 3, Step 3** claims the tree will be dirty during that test, so
>   `FARCOOLER_CHANNEL` is what makes it meaningful. That is wrong:
>   `channel()` in `scripts/version.sh` checks whether the tree is dirty
>   *first*, before it looks at `FARCOOLER_CHANNEL` or a tag, and answers
>   `local` regardless of what `FARCOOLER_CHANNEL` says. The env var does not
>   rescue a dirty-tree run.
> - **Task 5** ("A daemon older than the app that ships it") was reverted.
>   `crates/cli/src/daemon_link.rs`'s `ensure_local()` already compares the
>   running daemon's build stamp against `farcooler_protocol::BUILD` and
>   respawns it on a mismatch, so the Swift-side restart Task 5 proposed was
>   redundant. `apps/macos/Sources/FarCooler/DaemonFreshness.swift`, listed in
>   the File Structure table below, was never created and does not exist.

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Each shipping channel's Mac app checks its own feed daily, tells you when a newer build exists, and installs it when you say so.

**Architecture:** Sparkle 2 arrives as a SwiftPM dependency and is embedded by hand into the bundle `build-app.sh` assembles. Every per-channel fact — feed URL, public key — is stamped from `scripts/version.sh` and `apps/macos/sparkle-public-keys.txt`. CI signs each dmg with that channel's private key and publishes an appcast to R2.

**Tech Stack:** Swift 6 / SwiftPM / Sparkle 2, bash, Python 3, GitHub Actions, Cloudflare R2 via wrangler.

## Global Constraints

- **US English** in all code, comments and copy.
- **`local` never updates.** No feed URL is stamped for it, and the app must not start the updater when no feed is present.
- **Nothing installs unasked, on any channel:** `SUAutomaticallyUpdate` is `false` everywhere, `SUEnableAutomaticChecks` is `true`, `SUScheduledCheckInterval` is `86400`.
- **Per-channel values come from one place** — `version.sh` for the feed URL, `apps/macos/sparkle-public-keys.txt` for the public key. No second copy of either rule.
- **Nothing generated is written into a tracked path.** A dirty tree makes `version.sh channel` answer `local`, which silently turns a build into a local build.
- Public keys, already committed: stable `i+AqhWs/SKqHU1/zchkxXh8SSfMC/3IMzAMGFq9NLTE=`, preview `8zzBKbs6ZYzp2Zy8lJgATvFmxzP7ylyHvbXzaqfOOjI=`, canary `smRRRWWUuaCI8HjhrzBifwUWypSNdkl7uOBDyjlj4j4=`.
- `cargo` is not on PATH: build the app with `cd apps/macos && PATH="$HOME/.cargo/bin:$PATH" ./build-app.sh`.
- Never run `cargo fmt`.
- A live Far Cooler app runs on this machine. Never kill processes or match them by pattern.

---

## File Structure

| File | Responsibility |
| --- | --- |
| `apps/macos/Package.swift` | *(modify)* Sparkle dependency and the bundle rpath. |
| `apps/macos/build-app.sh` | *(modify)* Embed and sign Sparkle; stamp the five update keys. |
| `apps/macos/Resources/Info.plist` | *(modify)* The five keys, so `stamp()` has something to set. |
| `apps/macos/Sources/FarCooler/Updates.swift` | *(create)* Owns the updater and the menu item. |
| `apps/macos/Sources/FarCooler/DaemonFreshness.swift` | *(create)* Restart the daemon when its stamp differs from the app's. |
| `scripts/version.sh` | *(modify)* `feed-url`, empty for local. |
| `scripts/version-test.sh` | *(modify)* Cases for it. |
| `scripts/appcast.py` | *(create)* Emit one appcast from a signed dmg. |
| `scripts/appcast-test.sh` | *(create)* Its test. |
| `.github/workflows/canary.yml`, `release.yml` | *(modify)* Sign, generate, upload. |
| `docs/releasing.md` | *(modify)* How updates reach people. |

---

### Task 1: Sparkle in the bundle

Built first because everything else is inert until the app can load Sparkle, and because embedding a framework into a hand-assembled bundle is the only part of this feature with no precedent in the repository.

**Files:**
- Modify: `apps/macos/Package.swift`
- Modify: `apps/macos/build-app.sh`

**Interfaces:**
- Produces: a bundle containing `Contents/Frameworks/Sparkle.framework`, with the main binary carrying an `@executable_path/../Frameworks` rpath. Task 4 imports `Sparkle`.

- [ ] **Step 1: Add the dependency**

In `apps/macos/Package.swift`, add to `dependencies:`:

```swift
        // Sparkle, for the reason docs/superpowers/specs/2026-08-16-sparkle-auto-update-design.md
        // gives: a Developer ID app has no App Store to update it. The FIRST remote
        // dependency this package has ever had — everything else here is a path
        // dependency or a system library.
        .package(url: "https://github.com/sparkle-project/Sparkle", from: "2.6.0"),
```

and to the executable target's `dependencies:`: `.product(name: "Sparkle", package: "Sparkle"),`

Then extend that target's `linkerSettings:` with:

```swift
                // Where the framework will live once build-app.sh assembles the
                // bundle. SwiftPM links against its own artifact directory and has
                // no reason to know about a bundle it did not create, so without
                // this the app builds and then dies at launch with "Library not
                // loaded: @rpath/Sparkle.framework/Versions/B/Sparkle".
                .unsafeFlags(["-Xlinker", "-rpath", "-Xlinker", "@executable_path/../Frameworks"]),
```

- [ ] **Step 2: Confirm it resolves and the artifact appears**

```bash
cd apps/macos && swift build -c release 2>&1 | tail -3
find .build -name 'Sparkle.framework' -maxdepth 6 | head
```
Expected: a build that succeeds, and at least one path ending `Sparkle.xcframework/macos-*/Sparkle.framework`.

- [ ] **Step 3: Embed it**

In `apps/macos/build-app.sh`, after the `cp "$BIN" "$APP/Contents/MacOS/FarCooler"` line, add:

```bash
# Sparkle, copied in by hand because this script assembles the bundle by hand.
#
# Xcode would embed a framework for you; `swift build` produces a bare
# executable and an artifact directory, so the framework has to be found, copied
# and signed here. The macOS slice is selected explicitly: an XCFramework holds
# several, and copying the wrong one produces a bundle that launches on nothing.
#
# `ditto` rather than `cp -R`: a framework is a bundle of symlinks and extended
# attributes, and `cp` flattens both, which breaks the signature.
echo "==> Embedding Sparkle"
SPARKLE_SRC="$(find .build -path '*Sparkle.xcframework/macos-*/Sparkle.framework' -maxdepth 7 | head -1)"
[ -n "$SPARKLE_SRC" ] || { echo "no Sparkle.framework in .build — did swift build run?"; exit 1; }
mkdir -p "$APP/Contents/Frameworks"
ditto "$SPARKLE_SRC" "$APP/Contents/Frameworks/Sparkle.framework"
echo "    embedded from $SPARKLE_SRC"
```

- [ ] **Step 4: Build and verify the bundle can find it**

```bash
cd apps/macos && PATH="$HOME/.cargo/bin:$PATH" ./build-app.sh && cd ../..
APP="apps/macos/build/$(scripts/version.sh app-name).app"
ls -d "$APP/Contents/Frameworks/Sparkle.framework"
otool -l "$APP/Contents/MacOS/FarCooler" | grep -A2 LC_RPATH | grep -c 'Frameworks'
otool -L "$APP/Contents/MacOS/FarCooler" | grep Sparkle
```
Expected: the framework exists; the rpath count is at least 1; the load command names `@rpath/Sparkle.framework/Versions/B/Sparkle`.

- [ ] **Step 5: Prove it actually loads**

A framework that is present but unloadable is the failure this task exists to avoid, and neither `ls` nor `otool` can tell you.

```bash
APP="apps/macos/build/$(scripts/version.sh app-name).app"
DYLD_PRINT_LIBRARIES=1 "$APP/Contents/MacOS/FarCooler" --help 2>&1 | grep -i sparkle | head -3
```
Expected: at least one line naming the embedded `Sparkle.framework` inside the bundle. If the binary has no `--help`, run it with a two-second timeout and read the same output; a dyld failure prints `Library not loaded` and exits non-zero.

- [ ] **Step 6: Sign the nested code**

Sparkle's framework contains its own executables — `Autoupdate.app`, `Updater.app` and XPC services — and each needs its own signature before the framework and the app are signed, or notarization rejects the bundle. The signing step is conditional, exactly as the existing one is.

In the `Sign and notarise` step of `.github/workflows/canary.yml` and `.github/workflows/release.yml`, before the loop that signs `farcooler` and `farcoolerd`, add:

```bash
          # Inside-out, which is what codesign requires and what --deep gets
          # wrong: every nested executable is signed before the thing containing
          # it. Sparkle ships an updater app and XPC services inside its
          # framework, and an unsigned one fails notarization with a message
          # naming a path most people have never looked inside.
          sparkle="$app/Contents/Frameworks/Sparkle.framework"
          if [ -d "$sparkle" ]; then
            find "$sparkle" \( -name '*.xpc' -o -name '*.app' \) -print0 |
              while IFS= read -r -d '' nested; do
                codesign --force --options runtime --timestamp \
                  --sign "$MACOS_SIGN_IDENTITY" "$nested"
              done
            codesign --force --options runtime --timestamp \
              --sign "$MACOS_SIGN_IDENTITY" "$sparkle/Versions/B"
          fi
```

- [ ] **Step 7: Commit**

```bash
git add apps/macos/Package.swift apps/macos/build-app.sh .github/workflows/canary.yml .github/workflows/release.yml
git commit -m "feat(macos): carry Sparkle inside the bundle"
```

---

### Task 2: The feed URL joins the other per-channel answers

**Files:**
- Modify: `scripts/version.sh` (after `app_name_short`, before the dispatch)
- Test: `scripts/version-test.sh`

**Interfaces:**
- Produces: `./scripts/version.sh feed-url [tag]` → `https://updates.farcooler.com/<channel>/appcast.xml`, and **empty** for local. Task 3 stamps it.

- [ ] **Step 1: Write the failing tests**

Append to `scripts/version-test.sh` before the final `echo`:

```bash
# --- the update feed is per channel, and local has none --------------------
#
# A local build is the working tree of whoever built it. An updater offering to
# replace it with somebody else's build is a bug, so local gets no feed at all
# and the app declines to start the updater without one.
dir="$(scratch)"
check "stable has its own feed" \
  "https://updates.farcooler.com/stable/appcast.xml" \
  "$(cd "$dir" && FARCOOLER_CHANNEL=stable ./scripts/version.sh feed-url)"
check "canary has its own feed" \
  "https://updates.farcooler.com/canary/appcast.xml" \
  "$(cd "$dir" && FARCOOLER_CHANNEL=canary ./scripts/version.sh feed-url)"
check "preview has its own feed" \
  "https://updates.farcooler.com/preview/appcast.xml" \
  "$(cd "$dir" && FARCOOLER_CHANNEL=preview ./scripts/version.sh feed-url)"
check "local has no feed at all" "" "$(at "$dir" feed-url)"
feeds="$(
  for c in stable canary preview; do
    (cd "$dir" && FARCOOLER_CHANNEL=$c ./scripts/version.sh feed-url)
  done | sort -u | wc -l | tr -d ' '
)"
check "no two channels share a feed" "3" "$feeds"
rm -rf "$dir"
```

- [ ] **Step 2: Run and watch it fail**

Run: `./scripts/version-test.sh` — expect failures on the unknown `feed-url` subcommand.

- [ ] **Step 3: Implement**

After `app_name_short()` in `scripts/version.sh`:

```bash
# Where this channel's app looks for a newer version of itself.
#
# Empty for local, and that emptiness is load-bearing: `build-app.sh` stamps no
# feed, and `Updates.swift` starts no updater without one. A local build is
# somebody's working tree, and replacing it with a build from CI is not an
# update, it is losing work.
feed_url() {
  case "$(channel "$1")" in
    local) echo "" ;;
    *)     echo "https://updates.farcooler.com/$(channel "$1")/appcast.xml" ;;
  esac
}
```

Add `feed-url) feed_url "${2:-}" ;;` to the dispatch and `feed-url [tag]` to the usage line.

- [ ] **Step 4: Run and watch it pass** — `./scripts/version-test.sh`, expect 39 passed.

- [ ] **Step 5: Commit**

```bash
git add scripts/version.sh scripts/version-test.sh
git commit -m "feat(version): where a channel looks for its own updates"
```

---

### Task 3: Stamp the update keys into the bundle

**Files:**
- Modify: `apps/macos/Resources/Info.plist`
- Modify: `apps/macos/build-app.sh`

**Interfaces:**
- Consumes: `version.sh feed-url` (Task 2); `apps/macos/sparkle-public-keys.txt` (already committed).
- Produces: a bundle whose Info.plist carries the five keys. Task 4 reads `SUFeedURL` to decide whether to start.

- [ ] **Step 1: Give the plist the keys to stamp**

`stamp()` fails by design when a key is absent, so all five must exist in the source plist. Add to `apps/macos/Resources/Info.plist`:

```xml
    <!-- Sparkle. Stamped per channel by build-app.sh — see
         docs/superpowers/specs/2026-08-16-sparkle-auto-update-design.md.

         `SUFeedURL` is empty for local, and Updates.swift starts no updater
         without one: a local build is the working tree of whoever built it.

         `SUAutomaticallyUpdate` is false on every channel including canary.
         This is a tool people work inside, and an app that replaces itself
         unasked is a worse failure than an update noticed a day late. -->
    <key>SUFeedURL</key>
    <string></string>
    <key>SUPublicEDKey</key>
    <string></string>
    <key>SUEnableAutomaticChecks</key>
    <true/>
    <key>SUAutomaticallyUpdate</key>
    <false/>
    <key>SUScheduledCheckInterval</key>
    <integer>86400</integer>
```

- [ ] **Step 2: Stamp them**

In `build-app.sh`, beside the other resolved values at the top, add `FEED_URL="$(../../scripts/version.sh feed-url)"`. Then after the existing identity stamps:

```bash
# Which feed this build watches, and whose signature it will accept.
#
# The key is read from the committed table rather than written here: it is the
# trust anchor for the whole update path, and a second copy is a second thing to
# disagree. Local gets neither — an empty feed is how Updates.swift knows not to
# start.
stamp SUFeedURL "$FEED_URL"
if [ -n "$FEED_URL" ]; then
  SPARKLE_KEY="$(awk -v c="$CHANNEL" '$1 == c { print $2 }' sparkle-public-keys.txt)"
  [ -n "$SPARKLE_KEY" ] || {
    echo "no public key for $CHANNEL in sparkle-public-keys.txt"; exit 1; }
  stamp SUPublicEDKey "$SPARKLE_KEY"
fi
```

- [ ] **Step 3: Build each channel and read the values back**

```bash
cd apps/macos
for c in local canary stable; do
  FARCOOLER_CHANNEL=$c PATH="$HOME/.cargo/bin:$PATH" ./build-app.sh >/dev/null
  app="build/$(FARCOOLER_CHANNEL=$c ../../scripts/version.sh app-name).app"
  printf '%-7s feed=%s key=%s\n' "$c" \
    "$(/usr/libexec/PlistBuddy -c 'Print :SUFeedURL' "$app/Contents/Info.plist")" \
    "$(/usr/libexec/PlistBuddy -c 'Print :SUPublicEDKey' "$app/Contents/Info.plist")"
done
cd ..
```
Expected: `local` has an empty feed and an empty key; `canary` and `stable` each carry their own feed URL and the matching key from the committed table. Note the tree will be dirty while you work, so `FARCOOLER_CHANNEL` is what makes this test meaningful.

- [ ] **Step 4: Commit**

```bash
git add apps/macos/Resources/Info.plist apps/macos/build-app.sh
git commit -m "feat(macos): stamp the feed and the key this channel trusts"
```

---

### Task 4: The updater, and the menu item

**Files:**
- Create: `apps/macos/Sources/FarCooler/Updates.swift`
- Modify: the app's menu construction (find it with `grep -rn 'CommandGroup\|MenuBarExtra\|commands' apps/macos/Sources/FarCooler/`)

**Interfaces:**
- Consumes: `SUFeedURL` from Task 3.
- Produces: `Updates.shared.checkForUpdates()` and `Updates.shared.isEnabled`.

- [ ] **Step 1: Write it**

```swift
import Foundation
import Sparkle

/// Checking whether a newer build of THIS channel exists.
///
/// Every channel asks before installing, canary included. Far Cooler is a tool
/// people work inside, and an app that replaces itself unasked is a worse
/// failure than an update noticed a day late — so `SUAutomaticallyUpdate` is
/// false everywhere and this exists to surface the question rather than to
/// answer it.
///
/// A build with no feed does not start an updater at all. That is how `local`
/// declines: it is the working tree of whoever built it, and replacing it with
/// a build from CI is not an update.
@MainActor
final class Updates {
    static let shared = Updates()

    /// Nil when this build has no feed, which is the local channel and any
    /// bundle somebody assembled by hand.
    private let controller: SPUStandardUpdaterController?

    var isEnabled: Bool { controller != nil }

    private init() {
        let feed = Bundle.main.object(forInfoDictionaryKey: "SUFeedURL") as? String ?? ""
        guard !feed.isEmpty else {
            controller = nil
            return
        }
        controller = SPUStandardUpdaterController(
            startingUpdater: true,
            updaterDelegate: nil,
            userDriverDelegate: nil
        )
    }

    func checkForUpdates() {
        controller?.checkForUpdates(nil)
    }
}
```

- [ ] **Step 2: Add the menu item**

In the app's `commands` block, add a `CommandGroup(after: .appInfo)` containing a button titled `Check for Updates…` that calls `Updates.shared.checkForUpdates()` and is `.disabled(!Updates.shared.isEnabled)`. Match the surrounding menu code's style — read it first.

- [ ] **Step 3: Build and verify both states**

```bash
cd apps/macos && PATH="$HOME/.cargo/bin:$PATH" ./build-app.sh && cd ..
```
Expected: builds clean. Then confirm the local build's updater stays off — the bundle's `SUFeedURL` is empty, so `isEnabled` is false and the menu item is greyed. Say in your report how you confirmed it rather than assuming.

- [ ] **Step 4: Commit**

```bash
git add apps/macos/Sources/FarCooler/Updates.swift apps/macos/Sources/FarCooler
git commit -m "feat(macos): ask whether this channel has something newer"
```

---

### Task 5: A daemon older than the app that ships it

**Files:**
- Create: `apps/macos/Sources/FarCooler/DaemonFreshness.swift`
- Modify: wherever the app performs launch-time setup

**Interfaces:**
- Consumes: the daemon's reported build stamp and the app's own.

- [ ] **Step 1: Find the two stamps**

The app knows its own build from `AppVersion`; the daemon reports one over its protocol — `grep -rn 'build\|stamp' apps/macos/Sources/FarCooler/ | grep -i version` and read how `HostProbe` compares stamps for remote machines. Use the same comparison locally.

- [ ] **Step 2: Write it**

A type with one job: at launch, ask the local daemon what build it is; if it differs from the app's own, restart it through the same `SMAppService` registration `ServiceRegistration` owns. Comment it with the reasoning — the obvious Sparkle hook fires while the old bundle is still in place, so it would restart the daemon into the binary being replaced; comparing stamps at launch gets the same result after an update AND covers someone dragging a new app over the old one.

Restart only when the stamps differ, never unconditionally: a restart on every launch would tear down and rebuild the daemon's view of every machine for no reason.

- [ ] **Step 3: Verify**

Build the app, confirm the daemon is not restarted when the stamps already match (the ordinary case — say how you checked), and confirm the comparison reads both values rather than assuming either.

- [ ] **Step 4: Commit**

```bash
git commit -m "fix(macos): a daemon older than the app that ships it gets restarted"
```

---

### Task 6: The appcast

**Files:**
- Create: `scripts/appcast.py`
- Create: `scripts/appcast-test.sh`

**Interfaces:**
- Produces: `./scripts/appcast.py --channel <c> --version <marketing> --build <n> --url <enclosure> --length <bytes> --signature <edSignature> --notes <url>` printing the XML to stdout.

- [ ] **Step 1: Write the failing test**

`scripts/appcast-test.sh`, in the shape of `scripts/icon-test.sh`: run the generator with fixed inputs and assert the output contains the enclosure URL, the `sparkle:edSignature`, `sparkle:version` equal to the build number, `sparkle:shortVersionString` equal to the marketing version, and `sparkle:minimumSystemVersion`. Assert it is well-formed XML by parsing it with `python3 -c 'import sys,xml.dom.minidom; xml.dom.minidom.parse(sys.argv[1])'`. Assert an unknown channel exits non-zero.

- [ ] **Step 2: Run it, watch it fail** — the script does not exist yet.

- [ ] **Step 3: Write the generator**

One `<item>`, because Sparkle offers only the newest and the GitHub releases page is already the history. Read `sparkle:minimumSystemVersion` out of `apps/macos/Package.swift`'s `platforms:` floor rather than hardcoding it, so raising the floor cannot leave the appcast offering updates to Macs that cannot run them.

- [ ] **Step 4: Run it, watch it pass.**

- [ ] **Step 5: Commit**

```bash
git add scripts/appcast.py scripts/appcast-test.sh
git commit -m "feat(release): the feed that says a newer build exists"
```

- [ ] **Step 6: Wire the test into CI**

Add `- run: ./scripts/appcast-test.sh` to `ci.yml`'s `wire` job — it is pure Python and needs no Mac, unlike `icon-test.sh`, which is why that one lives in the `swift` job. Update the CI table in `docs/releasing.md`. Commit.

---

### Task 7: Publish it

**Files:**
- Modify: `.github/workflows/canary.yml`, `.github/workflows/release.yml`

**Interfaces:**
- Consumes: Task 6's generator; the `*_SPARKLE_KEY` secrets; the R2-enabled Cloudflare token.

- [ ] **Step 1: Fetch Sparkle's signing tool**

Both workflows need `sign_update`. Add a step that downloads the Sparkle release tarball and extracts `bin/sign_update`, pinned to the same major version `Package.swift` resolves.

- [ ] **Step 2: canary.yml — sign, generate, upload**

After the disk image is notarized, add a step gated on `vars.RELAY_DEPLOY`-style availability of the R2 credentials — use the same `vars` gate pattern the relay job uses, since `secrets` is unavailable in an `if`. It must:

1. `sign_update -s "$CANARY_SPARKLE_KEY" "<dmg>"` and capture the `sparkle:edSignature` and `length` it prints.
2. Upload the dmg to `canary/Far Cooler-$(scripts/version.sh build).dmg`.
3. Generate the appcast with that exact enclosure URL and upload it to `canary/appcast.xml` with `--cache-control 'max-age=300'`.

The dmg key includes the build number deliberately: overwriting one fixed path makes a stale appcast resolve to the wrong bytes and fail signature verification, which is indistinguishable from tampering. Comment that where the key is built.

- [ ] **Step 3: release.yml — the same, after the release exists**

Same three actions in the `publish` job, after `action-gh-release`, using the GitHub release asset URL as the enclosure rather than uploading the dmg. The channel decides which key and which path: `stable/appcast.xml` or `preview/appcast.xml`.

- [ ] **Step 4: Verify statically**

You cannot run these. Confirm: both files parse (`ruby -ryaml`), every path containing spaces is quoted, the enclosure URL in the generated appcast matches the object actually uploaded, and the signing step runs after the artifact exists in both files. Paste the step ordering for each.

- [ ] **Step 5: Commit**

```bash
git add .github/workflows/canary.yml .github/workflows/release.yml
git commit -m "feat(release): publish the appcast each channel watches"
```

---

### Task 8: Say how it works

**Files:** Modify `docs/releasing.md`

- [ ] **Step 1: Document it** — a section covering: what a person sees (a daily check, a prompt, nothing installed unasked, on every channel including canary); where the feeds and dmgs live; that `local` never checks; that the public keys are committed and why; and the setup a fork or a new machine needs (bucket, custom domain, token permission, lifecycle rule, three key pairs).

- [ ] **Step 2: Commit** — `git commit -m "docs: how a new build reaches a Mac"`

---

## Self-Review

**Spec coverage.** Policy and the five keys → Tasks 2, 3. Local never checks → Tasks 2, 3, 4. R2 layout, per-build canary key, cache-control → Task 7. Lifecycle rule → user setup, no task. Keys committed vs secret → already landed in `dc262ea`; consumed in Task 3 and Task 7. Sparkle dependency, embedding, rpath, nested signing → Task 1. `Updates.swift` and the menu item → Task 4. Daemon restart at launch → Task 5. `appcast.py` and its test → Task 6. Verification by reading built bundles → Tasks 1, 3, 4. Docs → Task 8.

**Placeholders.** Tasks 4, 5 and 8 describe rather than transcribe in places — deliberately: the menu construction, the daemon's stamp accessor and the docs' surrounding prose all depend on code the plan cannot quote without reading it first, and each step says which file to read. Every other step carries its code.

**Type consistency.** `feed-url` is spelled identically in Tasks 2, 3 and 7. `SUFeedURL` / `SUPublicEDKey` match between the plist, the stamps and `Updates.swift`. The appcast generator's flags in Task 6 match its invocation in Task 7. `sparkle-public-keys.txt`'s two-column format matches the `awk` that reads it.
