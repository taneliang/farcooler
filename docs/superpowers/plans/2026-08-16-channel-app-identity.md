# Channel App Identity Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make each channel's app a distinct app — its own bundle identifier, name, login agent and icon — so a canary can be installed beside the build someone depends on and told apart at a glance.

**Architecture:** Three new answers in `scripts/version.sh` (`app-suffix`, `app-name`, `app-name-short`) are the single source; `build-app.sh`, `generate-project.py`, `ServiceRegistration.swift` and both workflows derive from them. A new `scripts/icon-label.swift` draws a per-channel banner onto the one source icon at build time, writing only into already-ignored `build/` directories.

**Tech Stack:** bash, Swift 6 / AppKit (macOS only), Python 3, GitHub Actions YAML, PlistBuddy.

## Global Constraints

- **US English** in all code, comments and copy.
- **Stable keeps every bare name** — no suffix, no banner, no rename. Existing installs must not migrate.
- **Never write generated output into a tracked path.** `version.sh channel` answers `local` for a dirty tree, so a generated file inside the repository makes every later build step believe it is building `local`. Output goes only to `apps/ios/build/` and `apps/macos/build/`, both already in `.gitignore`.
- **Banner colours, exact:** canary `#E8A21C` on ink `#16130B`; preview `#3B6FD4` on `#FFFFFF`; local `#6E6E73` on `#FFFFFF`.
- **iOS short names, exact:** `Far Cooler`, `FC Canary`, `FC Preview`, `FC Local`. No home screen label may exceed 12 characters.
- **macOS names, exact:** `Far Cooler`, `Far Cooler Canary`, `Far Cooler Preview`, `Far Cooler Local`.
- Run scripts from the repository root unless a step says otherwise. `cargo` is not on `PATH`; prefix with `PATH="$HOME/.cargo/bin:$PATH"` if a step needs it (none here do).
- Never run `cargo fmt`. The Rust tree is hand-formatted and CI skips the check deliberately.

---

## File Structure

| File | Responsibility |
| --- | --- |
| `scripts/version.sh` | *(modify)* Add `app-suffix`, `app-name`, `app-name-short`. Sole owner of the channel→name mapping. |
| `scripts/version-test.sh` | *(modify)* Cases for the three new answers. |
| `scripts/icon-label.swift` | *(create)* Draw the channel banner onto a PNG. Pass stable through untouched. |
| `scripts/icon-test.sh` | *(create)* Assert stable is byte-identical and the other three differ from it and each other. |
| `apps/macos/build-app.sh` | *(modify)* Derive the bundle path, stamp identity keys, name the LaunchAgent per channel, label the icon. |
| `apps/macos/Sources/FarCooler/ServiceRegistration.swift` | *(modify)* Derive `plistName` from the stamped channel. |
| `apps/ios/generate-project.py` | *(modify)* Short display name; point the project at the generated asset catalog. |
| `apps/ios/FarCooler/Info.plist` | *(modify)* `CFBundleDisplayName` becomes a build setting. |
| `.github/workflows/canary.yml`, `.github/workflows/release.yml` | *(modify)* Stop hardcoding `Far Cooler.app`. |
| `docs/releasing.md` | *(modify)* Record the per-channel identity and the one-time migration. |

---

### Task 1: The three new answers in version.sh

**Files:**
- Modify: `scripts/version.sh` (add functions before the `case` dispatch at the end; extend the dispatch and usage line)
- Test: `scripts/version-test.sh` (append before the final `echo "$PASS passed, $FAIL failed"`)

**Interfaces:**
- Consumes: the existing `channel()` function, which takes an optional tag argument and honours `FARCOOLER_CHANNEL` and `FARCOOLER_TAG`.
- Produces: `./scripts/version.sh app-suffix [tag]` → `""` | `.canary` | `.preview` | `.local`; `app-name [tag]` → `Far Cooler` | `Far Cooler Canary` | …; `app-name-short [tag]` → `Far Cooler` | `FC Canary` | …. Every later task calls these.

- [ ] **Step 1: Write the failing tests**

Append to `scripts/version-test.sh`, immediately before the final `echo "$PASS passed, $FAIL failed"` line:

```bash
# --- a channel's app is its own app ---------------------------------------
#
# One identifier suffix, one name for the Dock and menu bar, one short name for
# a home screen that truncates at about twelve characters. Stable is bare on all
# three, because that is what an existing install already answers to.
dir="$(scratch)"
check "stable takes no suffix" \
  "" "$(cd "$dir" && FARCOOLER_CHANNEL=stable ./scripts/version.sh app-suffix)"
check "canary's suffix is its own name" \
  ".canary" "$(cd "$dir" && FARCOOLER_CHANNEL=canary ./scripts/version.sh app-suffix)"
check "stable keeps the bare app name" \
  "Far Cooler" "$(cd "$dir" && FARCOOLER_CHANNEL=stable ./scripts/version.sh app-name)"
check "canary says so where a name is shown" \
  "Far Cooler Canary" "$(cd "$dir" && FARCOOLER_CHANNEL=canary ./scripts/version.sh app-name)"
check "preview says so too" \
  "Far Cooler Preview" "$(cd "$dir" && FARCOOLER_CHANNEL=preview ./scripts/version.sh app-name)"
check "the short name is short" \
  "FC Canary" "$(cd "$dir" && FARCOOLER_CHANNEL=canary ./scripts/version.sh app-name-short)"
check "stable's short name is still the real one" \
  "Far Cooler" "$(cd "$dir" && FARCOOLER_CHANNEL=stable ./scripts/version.sh app-name-short)"

# The reason app-name-short exists at all: iOS truncates a home screen label at
# roughly twelve characters, and a name that truncates tells you nothing.
for c in stable canary preview local; do
  short="$(cd "$dir" && FARCOOLER_CHANNEL=$c ./scripts/version.sh app-name-short)"
  check "$c's home screen label fits" "ok" \
    "$([ "${#short}" -le 12 ] && echo ok || echo "too long: $short (${#short})")"
done

# And no two channels may answer the same thing, or two apps collide.
suffixes="$(
  for c in stable canary preview local; do
    (cd "$dir" && FARCOOLER_CHANNEL=$c ./scripts/version.sh app-suffix)
  done | sort -u | wc -l | tr -d ' '
)"
check "no two channels share a suffix" "4" "$suffixes"
rm -rf "$dir"
```

- [ ] **Step 2: Run it and watch it fail**

Run: `./scripts/version-test.sh`
Expected: FAIL. `version.sh` prints its usage line to stderr and exits 1 for the unknown subcommands, so the checks compare against empty strings — around 12 failures.

- [ ] **Step 3: Implement the three answers**

In `scripts/version.sh`, immediately after the `scheme()` function and before the final `case` dispatch, add:

```bash
# What a channel's app is CALLED, and what its identifier is suffixed with.
#
# Three answers rather than one, because three different things need naming: an
# identifier (`com.farcooler.FarCooler.canary`), a name a person reads in the
# Dock and the menu bar, and a shorter one for an iOS home screen, which
# truncates a label at roughly twelve characters — `Far Cooler Canary` arrives
# there as `Far Cooler C…`, which tells nobody anything.
#
# Here rather than in the build scripts for the same reason as `scheme`: three
# mappings copied into build-app.sh and generate-project.py are three mappings
# that drift.
#
# Stable is bare on all three. It is what every existing install already answers
# to, and renaming it orphans people instead of updating them.
app_suffix() {
  case "$(channel "$1")" in
    stable) echo "" ;;
    *)      echo ".$(channel "$1")" ;;
  esac
}

app_name() {
  case "$(channel "$1")" in
    stable) echo "Far Cooler" ;;
    *)      echo "Far Cooler $(channel "$1" | awk '{print toupper(substr($0,1,1)) substr($0,2)}')" ;;
  esac
}

app_name_short() {
  case "$(channel "$1")" in
    stable) echo "Far Cooler" ;;
    *)      echo "FC $(channel "$1" | awk '{print toupper(substr($0,1,1)) substr($0,2)}')" ;;
  esac
}
```

Then extend the dispatch `case` and its usage line:

```bash
  scheme) scheme "${2:-}" ;;
  app-suffix) app_suffix "${2:-}" ;;
  app-name) app_name "${2:-}" ;;
  app-name-short) app_name_short "${2:-}" ;;
  *)
    echo "usage: version.sh [marketing|build|channel [tag]|display [tag]|scheme [tag]|app-suffix [tag]|app-name [tag]|app-name-short [tag]]" >&2
    exit 1
    ;;
```

- [ ] **Step 4: Run the tests and watch them pass**

Run: `./scripts/version-test.sh`
Expected: PASS, `34 passed, 0 failed` (22 existing plus 12 new).

- [ ] **Step 5: Commit**

```bash
git add scripts/version.sh scripts/version-test.sh
git commit -m "feat(version): what a channel's app is called"
```

---

### Task 2: The icon labeller

**Files:**
- Create: `scripts/icon-label.swift`
- Create: `scripts/icon-test.sh`
- Read only: `apps/shared/Assets.xcassets/AppIcon.appiconset/AppIcon.png` (1024×1024, the source)

**Interfaces:**
- Consumes: nothing from Task 1 — it takes the channel as an argument.
- Produces: `swift scripts/icon-label.swift <channel> <input.png> <output.png>`, exit 0 on success, exit 1 for an unknown channel, exit 2 for wrong arguments. Tasks 3 and 5 call it.

- [ ] **Step 1: Write the failing test**

Create `scripts/icon-test.sh`:

```bash
#!/bin/bash
# That four channels produce four icons, and that stable produces none at all.
#
# The banner is what tells a canary from the app someone depends on at a glance,
# so "did it draw anything" is not a question to answer by looking once and
# trusting it afterwards. Stable must come through BYTE-IDENTICAL: it is not
# labelled, and re-encoding it would churn the one asset every channel shares.
#
#   ./scripts/icon-test.sh
set -euo pipefail

cd "$(dirname "$0")/.."
SRC="apps/shared/Assets.xcassets/AppIcon.appiconset/AppIcon.png"
PASS=0
FAIL=0

check() {
  local what="$1" want="$2" got="$3"
  if [ "$want" = "$got" ]; then
    PASS=$((PASS + 1))
  else
    FAIL=$((FAIL + 1))
    echo "FAIL: $what" >&2
    echo "  want: $want" >&2
    echo "  got:  $got" >&2
  fi
}

sha() { shasum -a 256 < "$1" | cut -d' ' -f1; }

out="$(mktemp -d)"
trap 'rm -rf "$out"' EXIT

for c in stable canary preview local; do
  swift scripts/icon-label.swift "$c" "$SRC" "$out/$c.png"
done

check "stable is passed through untouched" "$(sha "$SRC")" "$(sha "$out/stable.png")"

for c in canary preview local; do
  if [ "$(sha "$SRC")" = "$(sha "$out/$c.png")" ]; then
    check "$c is labelled" "different from the source" "identical to the source"
  else
    check "$c is labelled" "different from the source" "different from the source"
  fi
done

# The point of the exercise: four channels, four distinguishable icons.
distinct="$(shasum -a 256 "$out"/*.png | awk '{print $1}' | sort -u | wc -l | tr -d ' ')"
check "no two channels share an icon" "4" "$distinct"

# An unknown channel is refused rather than silently unlabelled, for the reason
# version.sh refuses one: a name we cannot read must not pass itself off as
# stable.
set +e
swift scripts/icon-label.swift production "$SRC" "$out/x.png" >/dev/null 2>&1
code=$?
set -e
check "an unknown channel is refused" "1" "$code"

echo "$PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ]
```

Make it executable: `chmod +x scripts/icon-test.sh`

- [ ] **Step 2: Run it and watch it fail**

Run: `./scripts/icon-test.sh`
Expected: FAIL — `swift: cannot open file scripts/icon-label.swift`, non-zero exit before any check runs.

- [ ] **Step 3: Write the labeller**

Create `scripts/icon-label.swift`:

```swift
#!/usr/bin/env swift
// Draw a channel's banner across the app icon.
//
// One source icon, four apps. Without this they are the same bear, and the only
// way to tell a canary from the build you depend on is to open it and read
// Settings.
//
// AppKit rather than ImageMagick because every caller is already a macOS runner
// where Swift exists, and adding a `brew install` to a path that needs none
// buys nothing.
//
//   swift scripts/icon-label.swift canary in.png out.png
import AppKit
import Foundation

struct Banner {
    let text: String
    let fill: NSColor
    let ink: NSColor
}

func rgb(_ hex: UInt32) -> NSColor {
    NSColor(
        srgbRed: CGFloat((hex >> 16) & 0xFF) / 255,
        green: CGFloat((hex >> 8) & 0xFF) / 255,
        blue: CGFloat(hex & 0xFF) / 255,
        alpha: 1
    )
}

// Colours are per channel and carry more than the word does: at 32 points the
// text is unreadable and the colour is the whole signal.
let banners: [String: Banner] = [
    "canary": Banner(text: "CANARY", fill: rgb(0xE8A21C), ink: rgb(0x16130B)),
    "preview": Banner(text: "PREVIEW", fill: rgb(0x3B6FD4), ink: rgb(0xFFFFFF)),
    "local": Banner(text: "LOCAL", fill: rgb(0x6E6E73), ink: rgb(0xFFFFFF)),
]

func fail(_ message: String, _ code: Int32) -> Never {
    FileHandle.standardError.write(Data((message + "\n").utf8))
    exit(code)
}

let args = CommandLine.arguments
guard args.count == 4 else {
    fail("usage: icon-label.swift <channel> <input.png> <output.png>", 2)
}
let channel = args[1]
let input = args[2]
let output = args[3]

// Stable is not labelled, and is not re-encoded either: it is the source asset
// every channel shares, and rewriting its bytes would churn it for nothing.
if channel == "stable" {
    try? FileManager.default.removeItem(atPath: output)
    do {
        try FileManager.default.copyItem(atPath: input, toPath: output)
    } catch {
        fail("could not copy \(input) to \(output): \(error)", 1)
    }
    exit(0)
}

guard let banner = banners[channel] else {
    fail("unknown channel: \(channel)", 1)
}
guard let data = FileManager.default.contents(atPath: input),
    let source = NSBitmapImageRep(data: data)
else {
    fail("could not read an image from \(input)", 1)
}

let side = CGFloat(source.pixelsWide)
guard source.pixelsHigh == source.pixelsWide else {
    fail("the icon must be square, got \(source.pixelsWide)x\(source.pixelsHigh)", 1)
}

guard
    let canvas = NSBitmapImageRep(
        bitmapDataPlanes: nil,
        pixelsWide: source.pixelsWide, pixelsHigh: source.pixelsHigh,
        bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
        colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0
    )
else { fail("could not allocate a \(source.pixelsWide)px canvas", 1) }

NSGraphicsContext.saveGraphicsState()
NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: canvas)
source.draw(in: CGRect(x: 0, y: 0, width: side, height: side))

let context = NSGraphicsContext.current!.cgContext
context.saveGState()
// Rotate about the centre so the band crosses the bottom-right corner at 45°.
context.translateBy(x: side / 2, y: side / 2)
context.rotate(by: -.pi / 4)

let bandHeight = side * 0.17
// Overlong on purpose: the band must run past both edges of the icon after
// rotation, or its ends appear as cut corners inside the artwork.
let band = CGRect(x: -side, y: -side * 0.60, width: side * 2, height: bandHeight)
banner.fill.setFill()
context.fill(band)

let font = NSFont.systemFont(ofSize: bandHeight * 0.58, weight: .black)
let attributes: [NSAttributedString.Key: Any] = [
    .font: font,
    .foregroundColor: banner.ink,
    .kern: bandHeight * 0.05,
]
let text = banner.text as NSString
let measured = text.size(withAttributes: attributes)
text.draw(
    at: CGPoint(x: -measured.width / 2, y: band.midY - measured.height / 2),
    withAttributes: attributes
)

context.restoreGState()
NSGraphicsContext.restoreGraphicsState()

guard let png = canvas.representation(using: .png, properties: [:]) else {
    fail("could not encode a PNG", 1)
}
do {
    try png.write(to: URL(fileURLWithPath: output))
} catch {
    fail("could not write \(output): \(error)", 1)
}
```

- [ ] **Step 4: Run the tests and watch them pass**

Run: `./scripts/icon-test.sh`
Expected: PASS, `6 passed, 0 failed`.

- [ ] **Step 5: Look at one, once**

The tests prove four different images exist; only an eye proves the band is in the right place.

```bash
swift scripts/icon-label.swift canary \
  apps/shared/Assets.xcassets/AppIcon.appiconset/AppIcon.png /tmp/canary-icon.png
open /tmp/canary-icon.png
```

Expected: an amber band across the bottom-right corner reading CANARY, running clear off both edges, with the bear's face unobscured. If the band clips the muzzle, reduce `0.60`; if it sits off the corner, increase it.

- [ ] **Step 6: Commit**

```bash
git add scripts/icon-label.swift scripts/icon-test.sh
git commit -m "feat(icon): a channel's icon says which channel it is"
```

---

### Task 3: The macOS bundle takes its channel's identity

**Files:**
- Modify: `apps/macos/build-app.sh` (line 14 `APP=`; the stamp block around lines 58–63; the LaunchAgent copy near line 138; the iconset block near line 155)

**Interfaces:**
- Consumes: `version.sh app-name`, `app-suffix` (Task 1); `scripts/icon-label.swift` (Task 2).
- Produces: a bundle at `apps/macos/build/$(version.sh app-name).app` whose `CFBundleIdentifier` is `com.farcooler.FarCooler$(app-suffix)`, containing `Contents/Library/LaunchAgents/com.farcooler.daemon$(app-suffix).plist` whose `Label` matches. Task 4 reads that filename; Task 6 uses that bundle path.

- [ ] **Step 1: Derive the bundle path**

In `apps/macos/build-app.sh`, replace line 14:

```bash
APP="build/Far Cooler.app"
```

with:

```bash
# Named for the channel, so a canary installs BESIDE the build someone depends
# on rather than over it. Stable is still `Far Cooler.app`, which is what every
# existing install is called.
APP="build/$(../../scripts/version.sh app-name).app"
```

- [ ] **Step 2: Stamp the identity keys**

After the existing `stamp FarCoolerDisplayVersion …` line, add:

```bash
# The identity that decides whether two channels are two apps or one.
#
# `UserDefaults` keys off the bundle identifier, so this partitions preferences
# with no code — a canary starts from defaults rather than rewriting the
# settings of the app someone works in.
stamp CFBundleIdentifier "com.farcooler.FarCooler$(../../scripts/version.sh app-suffix)"
stamp CFBundleName "$(../../scripts/version.sh app-name)"
stamp CFBundleDisplayName "$(../../scripts/version.sh app-name)"
```

- [ ] **Step 3: Name the login agent per channel**

Find the line that copies the LaunchAgent plist (near line 138):

```bash
cp Resources/com.farcooler.daemon.plist "$APP/Contents/Library/LaunchAgents/"
```

Replace it with:

```bash
# One launchd label per channel. Two apps registering `com.farcooler.daemon`
# means the second registration replaces the first and which daemon starts at
# login becomes a question of install order — silently, since SMAppService
# reports success either way.
AGENT_LABEL="com.farcooler.daemon$(../../scripts/version.sh app-suffix)"
AGENT_PLIST="$APP/Contents/Library/LaunchAgents/$AGENT_LABEL.plist"
cp Resources/com.farcooler.daemon.plist "$AGENT_PLIST"
/usr/libexec/PlistBuddy -c "Set :Label $AGENT_LABEL" "$AGENT_PLIST" >/dev/null
echo "    login agent $AGENT_LABEL"
```

- [ ] **Step 4: Label the icon**

Find the iconset block near line 155 (`ICONSET="build/AppIcon.iconset"`). Immediately **before** the first `sips` call that reads the source PNG, insert:

```bash
# The channel's banner, drawn onto a copy in build/ — never onto the source.
#
# Writing a generated icon into the tracked asset catalog would dirty the tree,
# and `version.sh channel` answers `local` for a dirty tree: every step after
# that point would believe it was building local, and a canary would install at
# local's identifier with no error anywhere.
LABELLED="build/AppIcon-$(../../scripts/version.sh channel).png"
swift ../../scripts/icon-label.swift \
  "$(../../scripts/version.sh channel)" \
  ../shared/Assets.xcassets/AppIcon.appiconset/AppIcon.png \
  "$LABELLED"
```

Then change every `sips` invocation in that block to read `"$LABELLED"` instead of the path under `../shared/Assets.xcassets/`.

- [ ] **Step 5: Build it and read the bundle back**

```bash
cd apps/macos && ./build-app.sh && cd ../..
APP="apps/macos/build/$(scripts/version.sh app-name).app"
/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$APP/Contents/Info.plist"
/usr/libexec/PlistBuddy -c 'Print :CFBundleName' "$APP/Contents/Info.plist"
ls "$APP/Contents/Library/LaunchAgents/"
/usr/libexec/PlistBuddy -c 'Print :Label' \
  "$APP/Contents/Library/LaunchAgents/com.farcooler.daemon$(scripts/version.sh app-suffix).plist"
```

Expected on a dirty working tree (channel `local`): `com.farcooler.FarCooler.local`, `Far Cooler Local`, a single `com.farcooler.daemon.local.plist`, and `Label` printing `com.farcooler.daemon.local`.

- [ ] **Step 6: Commit**

```bash
git add apps/macos/build-app.sh
git commit -m "feat(macos): a channel's Mac app is its own app"
```

---

### Task 4: The app looks for its own login agent

**Files:**
- Modify: `apps/macos/Sources/FarCooler/ServiceRegistration.swift:33` (the `plistName` constant)

**Interfaces:**
- Consumes: `AppVersion.channel` from AgentKit, which reads `FarCoolerChannel` from the bundle's Info.plist and returns `local` when unstamped; the plist filename produced by Task 3.
- Produces: nothing other tasks consume.

- [ ] **Step 1: Derive the name**

Replace:

```swift
    private let plistName = "com.farcooler.daemon.plist"
```

with:

```swift
    /// This channel's login agent, not the shared one.
    ///
    /// Each channel's bundle carries a plist named for itself — see
    /// `build-app.sh` — because two apps registering one launchd label means the
    /// second registration replaces the first, and which daemon starts at login
    /// becomes a question of install order. `SMAppService` reports success for
    /// both, so the collision is invisible from here.
    ///
    /// Read from the channel this build was stamped with rather than passed in:
    /// the bundle already knows what it is, and a second way of saying so is a
    /// second way to be wrong.
    private let plistName: String = {
        let channel = AppVersion.channel
        return channel == "stable"
            ? "com.farcooler.daemon.plist"
            : "com.farcooler.daemon.\(channel).plist"
    }()
```

If `AppVersion` is not already visible in this file, add `import AgentKit` at the top.

- [ ] **Step 2: Build and confirm the app agrees with its own bundle**

```bash
cd apps/macos && ./build-app.sh && cd ../..
APP="apps/macos/build/$(scripts/version.sh app-name).app"
CHANNEL="$(scripts/version.sh channel)"
EXPECTED="com.farcooler.daemon$(scripts/version.sh app-suffix).plist"
ls "$APP/Contents/Library/LaunchAgents/$EXPECTED" \
  && echo "the plist the app will ask SMAppService for exists: $EXPECTED"
```

Expected: the file lists. A mismatch here is exactly the `.notFound` case `plistIsBundled` was written to distinguish, and would show in the app as "This build has no LaunchAgent to register."

- [ ] **Step 3: Commit**

```bash
git add apps/macos/Sources/FarCooler/ServiceRegistration.swift
git commit -m "fix(macos): register this channel's login agent, not the shared one"
```

---

### Task 5: The iOS app takes its channel's name and icon

**Files:**
- Modify: `apps/ios/generate-project.py` (`ASSET_CATALOG` at line 122; the `PBXFileReference` path at line 205; `TARGET_COMMON` near line 348)
- Modify: `apps/ios/FarCooler/Info.plist` (`CFBundleDisplayName`)

**Interfaces:**
- Consumes: `version.sh app-name-short` (Task 1); `scripts/icon-label.swift` (Task 2).
- Produces: a generated project whose display name is the channel's short name and whose asset catalog is `apps/ios/build/Assets.xcassets`.

- [ ] **Step 1: Generate the catalog copy**

In `apps/ios/generate-project.py`, after the `CHANNEL = version("channel")` line, add:

```python
# The asset catalog this project will use, which is a COPY.
#
# The shared catalog is tracked, and writing a generated icon into a tracked
# path dirties the tree — after which `version.sh channel` answers `local` and
# every later step believes it is building local. So the catalog is copied to
# build/ (already gitignored) and only AppIcon.png is replaced inside the copy;
# Contents.json and every other asset travel unchanged.
GENERATED_CATALOG = Path(__file__).parent / "build" / "Assets.xcassets"
SHARED_CATALOG = Path(__file__).parent.parent / "shared" / "Assets.xcassets"
if GENERATED_CATALOG.exists():
    shutil.rmtree(GENERATED_CATALOG)
GENERATED_CATALOG.parent.mkdir(parents=True, exist_ok=True)
shutil.copytree(SHARED_CATALOG, GENERATED_CATALOG)
subprocess.run(
    [
        "swift",
        str(Path(__file__).parent.parent.parent / "scripts" / "icon-label.swift"),
        CHANNEL,
        str(SHARED_CATALOG / "AppIcon.appiconset" / "AppIcon.png"),
        str(GENERATED_CATALOG / "AppIcon.appiconset" / "AppIcon.png"),
    ],
    check=True,
)
```

Add `import shutil` and `from pathlib import Path` at the top if they are not already imported (`subprocess` already is — it is used by `version()`).

- [ ] **Step 2: Point the project at the copy**

At line 205, change:

```python
        f"path = ../shared/{ASSET_CATALOG}; sourceTree = SOURCE_ROOT; }};"
```

to:

```python
        f"path = build/{ASSET_CATALOG}; sourceTree = SOURCE_ROOT; }};"
```

- [ ] **Step 3: Add the display name build setting**

In `TARGET_COMMON`, after the `FARCOOLER_URL_SCHEME` line, add:

```python
\t\t\t\tFARCOOLER_APP_NAME = "{version("app-name-short")}";
```

- [ ] **Step 4: Use it in the plist**

In `apps/ios/FarCooler/Info.plist`, replace:

```xml
    <key>CFBundleDisplayName</key>
    <string>Far Cooler</string>
```

with:

```xml
    <!-- Per channel, and SHORT: iOS truncates a home screen label at roughly
         twelve characters, so `Far Cooler Canary` would arrive as
         `Far Cooler C…`. See `version.sh app-name-short`. -->
    <key>CFBundleDisplayName</key>
    <string>$(FARCOOLER_APP_NAME)</string>
```

- [ ] **Step 5: Generate, build, and read it back**

```bash
cd apps/ios && ./generate-project.py && cd ../..
cd apps/ios && xcodebuild -project FarCooler.xcodeproj -scheme FarCooler \
  -destination 'generic/platform=iOS Simulator' -configuration Debug ARCHS=arm64 \
  -derivedDataPath /tmp/fc-identity build 2>&1 | tail -3; cd ../..
/usr/libexec/PlistBuddy -c 'Print :CFBundleDisplayName' \
  /tmp/fc-identity/Build/Products/Debug-iphonesimulator/FarCooler.app/Info.plist
```

Expected: `** BUILD SUCCEEDED **`, then `FC Local` on a dirty tree.

- [ ] **Step 6: Confirm the tracked catalog was not touched**

```bash
git status --short apps/shared/Assets.xcassets
```

Expected: empty. Anything here means the generator wrote into a tracked path, which is the failure this design exists to prevent — fix before committing.

- [ ] **Step 7: Revert the generated project file**

`apps/ios/FarCooler.xcodeproj/project.pbxproj` is tracked but regenerated per channel, so a local run leaves `local` bundle identifiers in it.

```bash
git checkout -- apps/ios/FarCooler.xcodeproj/project.pbxproj
```

- [ ] **Step 8: Commit**

```bash
git add apps/ios/generate-project.py apps/ios/FarCooler/Info.plist
git commit -m "feat(ios): a channel's phone app says which channel it is"
```

---

### Task 6: The workflows stop hardcoding the app's name

**Files:**
- Modify: `.github/workflows/canary.yml` (the `Sign and notarise` and `Package the disk image` steps)
- Modify: `.github/workflows/release.yml` (the `Sign and notarise` and `Package the disk image` steps)

**Interfaces:**
- Consumes: `version.sh app-name` (Task 1); the bundle path from Task 3.
- Produces: nothing other tasks consume.

- [ ] **Step 1: Replace the literal in both files**

Every occurrence of `apps/macos/build/Far Cooler.app` becomes a derived path. In each `run:` block that references it, add near the top:

```bash
          app="apps/macos/build/$(scripts/version.sh app-name).app"
```

and use `"$app"` in place of the literal. In `canary.yml`'s `Package the disk image` step the `ditto` source becomes `"$app"`; the staged name inside the image should stay `Far Cooler Canary.app` — that is, `ditto "$app" "$staging/$(basename "$app")"`.

- [ ] **Step 2: Check no literal survives**

Run: `grep -rn 'build/Far Cooler.app' .github/workflows/`
Expected: no output.

- [ ] **Step 3: Check both files still parse**

```bash
ruby -ryaml -e "YAML.load_file('.github/workflows/canary.yml'); puts 'canary ok'"
ruby -ryaml -e "YAML.load_file('.github/workflows/release.yml'); puts 'release ok'"
```

Expected: both print `ok`.

- [ ] **Step 4: Commit**

```bash
git add .github/workflows/canary.yml .github/workflows/release.yml
git commit -m "fix(ci): the Mac app's name depends on its channel"
```

---

### Task 7: Say so in the docs

**Files:**
- Modify: `docs/releasing.md` (the channels section)

**Interfaces:** none.

- [ ] **Step 1: Document the identity and the one-time migration**

Add to `docs/releasing.md`, after the section describing the four channels:

```markdown
## A channel's app is its own app

| | stable | canary |
| --- | --- | --- |
| macOS bundle id | `com.farcooler.FarCooler` | `com.farcooler.FarCooler.canary` |
| macOS app | `Far Cooler.app` | `Far Cooler Canary.app` |
| iOS home screen | Far Cooler | FC Canary |
| login agent | `com.farcooler.daemon` | `com.farcooler.daemon.canary` |
| icon | the bear | the bear, amber CANARY banner |

All of it derives from `scripts/version.sh` — `app-suffix`, `app-name`,
`app-name-short` — for the reason the CLI's name and the URL scheme do: a rule
written once cannot disagree with itself. Stable keeps every bare name, because
that is what existing installs answer to.

The icons are drawn at build time by `scripts/icon-label.swift` from the one
source asset, into `build/` directories only. A generated icon written into the
repository would dirty the tree, and a dirty tree makes `version.sh channel`
answer `local` — so the act of labelling a canary would make every later step
build local instead, silently.

**One-time migration.** A canary Mac app installed before this change carries
the STABLE bundle identifier and is called `Far Cooler.app`. The next canary
installs beside it as `Far Cooler Canary.app`, leaving the old one looking
exactly like your stable app. Delete the old one — and if it ever registered the
daemon, switch that off first, or a launchd job under the shared label outlives
the app that created it.
```

- [ ] **Step 2: Commit**

```bash
git add docs/releasing.md
git commit -m "docs: what a channel's app is called, and the one migration"
```

---

## Self-Review

**Spec coverage.** Identity table → Task 1 (the answers), Tasks 3–5 (applied). LaunchAgent collision → Tasks 3 and 4. Preferences partition → falls out of Task 3's bundle identifier, noted there. Icon generation, colours, stable byte-identical → Task 2. Never-write-to-tracked-paths → enforced in Task 3 step 4, Task 5 steps 1 and 6. Workflow paths → Task 6. Migration note → Task 7. `paths.rs` verification → **resolved before planning**: `runtime_dir_for(channel)` already partitions the daemon's directory per channel, so no task is needed.

**Placeholders.** None: every step carries the code or the command it needs.

**Type consistency.** `app-suffix` / `app-name` / `app-name-short` are spelled identically in Tasks 1, 3, 5, 6 and 7. `icon-label.swift <channel> <in> <out>` is called with that argument order in Task 2's test, Task 3 step 4 and Task 5 step 1. `AGENT_LABEL` in Task 3 and `plistName` in Task 4 produce the same string for a given channel.
