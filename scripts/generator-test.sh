#!/bin/bash
# What generate-project.py must emit, checked against the real project.pbxproj.
#
# Every assertion here is for something that BUILDS FINE and fails afterwards.
# A watch app copied into PlugIns/ instead of Watch/ compiles, links, signs, and
# then fails at install or launch. A bundle identifier that does not nest under
# the app's compiles and fails to sign against the app's profile. A target that
# names the wrong App Group — or none — compiles and produces a surface that is
# permanently, silently empty on a device where everything else works. None of
# these appear as an `error:` line anywhere, so they are asserted mechanically
# or they are not caught at all.
#
# It leaves the working tree exactly as it found it. See `restore` below for
# why that is correctness rather than tidiness.
#
#   ./scripts/generator-test.sh
set -euo pipefail

cd "$(dirname "$0")/.."
PBX="apps/ios/FarCooler.xcodeproj/project.pbxproj"
PASS=0
FAIL=0

# Put back whatever was there before, however this script exits.
#
# generate-project.py writes its output to a path fixed relative to its own
# location, so this test cannot avoid rewriting a TRACKED file in place. Leaving
# it modified would not merely be untidy: `scripts/version.sh` answers `local`
# for ANY dirty tree, and the channel decides the bundle identifier, the App
# Group, the URL scheme and the APNs topic. So a test that dirties the tree
# makes every later step that reads the channel believe it is building local —
# the same hazard generate-project.py's own GENERATED_CATALOG comment guards
# against, and it matters more here because CI runs this ahead of steps that ask.
#
# Restoring rather than refusing to run on a dirty tree, because a regenerated
# project.pbxproj sitting in the working tree is the NORMAL state here: it is
# generated state that is never staged, so anyone who has built recently has one.
BEFORE="$(mktemp)"
HAD_ONE=no
if [ -f "$PBX" ]; then
  cp "$PBX" "$BEFORE"
  HAD_ONE=yes
fi
restore() {
  if [ "$HAD_ONE" = yes ]; then
    cp "$BEFORE" "$PBX"
  else
    rm -f "$PBX"
  fi
  rm -f "$BEFORE"
}
trap restore EXIT

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

# "yes" or "no", so a missing thing reads as a value rather than as an exit code
# `set -e` would turn into a crash.
has() { if grep -q "$1" <<< "$2"; then echo yes; else echo no; fi; }

python3 apps/ios/generate-project.py > /dev/null

# --- the build settings of one target ------------------------------------
#
# The XCBuildConfiguration blocks carry no target name, but each carries
# PRODUCT_NAME, which is unique per target. Matching on `PRODUCT_NAME = X;` with
# the semicolon is what keeps `FarCooler` from also matching `FarCoolerWatch`.
settings_for() {
  awk -v want="$1" '
    /isa = XCBuildConfiguration;/ { block = ""; inblock = 1 }
    inblock { block = block $0 "\n" }
    inblock && /^\t\t\};$/ {
      inblock = 0
      if (block ~ ("PRODUCT_NAME = " want ";")) printf "%s", block
    }
  ' "$PBX"
}

# One setting's value, unquoted, across BOTH of a target's configurations.
# Empty when the target does not carry it.
#
# Every target here has a Debug block and a Release block, and this reads both
# rather than `grep -m1`-ing whichever appears first. With -m1, a value that
# differs between the two configurations is invisible: the assertion reads Debug,
# passes, and the Release build — the one that ships — carries something else.
# Catching exactly that class of thing is this file's entire job.
#
# Divergence returns a string no caller can be expecting, so the assertion fails
# and its `got:` line names both values rather than silently preferring one.
setting() {
  local values count
  values="$(
    settings_for "$1" \
      | { grep "^[[:space:]]*$2 = " || true; } \
      | sed 's/^[^=]*= //; s/;$//; s/^"//; s/"$//' \
      | sort -u
  )"
  [ -n "$values" ] || return 0
  count="$(wc -l <<< "$values" | tr -d ' ')"
  if [ "$count" -gt 1 ]; then
    echo "DEBUG AND RELEASE DISAGREE: $(paste -sd'|' - <<< "$values")"
  else
    printf '%s' "$values"
  fi
}

# --- the watch app is embedded as a watch app, not as an extension -------
#
# `dstSubfolderSpec = 16` with `dstPath = $(CONTENTS_FOLDER_PATH)/Watch` is
# "Embed Watch Content". The extensions' phase is `13` / PlugIns. To Xcode these
# are genuinely different operations, and the wrong one produces an app that
# builds and then never offers the watch app to the paired watch.
#
# Scoped to the PBXCopyFilesBuildPhase section and anchored on `= {` at end of
# line, because "… in Embed Foundation Extensions */ = {isa = PBXBuildFile; …}"
# is a one-line object carrying the same phase name and would otherwise open the
# range and swallow half the file.
copy_phases="$(awk '/Begin PBXCopyFilesBuildPhase/,/End PBXCopyFilesBuildPhase/' "$PBX")"
watch_phase="$(awk '/Embed Watch Content \*\/ = \{$/,/^\t\t\};$/' <<< "$copy_phases")"
check "there is an Embed Watch Content phase" \
  "yes" "$([ -n "$watch_phase" ] && echo yes || echo no)"
check "the watch app is embedded at dstSubfolderSpec 16" \
  "yes" "$(has 'dstSubfolderSpec = 16;' "$watch_phase")"
check "the watch app is embedded into Watch/" \
  "yes" "$(has 'dstPath = "\$(CONTENTS_FOLDER_PATH)/Watch";' "$watch_phase")"
check "it is the watch app that is embedded there" \
  "yes" "$(has 'FarCoolerWatch.app' "$watch_phase")"

# And the converse, which is the mistake this is guarding against: the watch app
# must NOT also be a file in the extensions' phase.
extensions_phase="$(awk '/Embed Foundation Extensions \*\/ = \{$/,/^\t\t\};$/' <<< "$copy_phases")"
check "the extensions phase is still the 13/PlugIns slot" \
  "yes" "$(has 'dstSubfolderSpec = 13;' "$extensions_phase")"
check "the watch app is not in the extensions phase" \
  "no" "$(has 'FarCoolerWatch.app' "$extensions_phase")"

# --- the complication is embedded in the WATCH app, not the phone --------
#
# 13/PlugIns like any other .appex — a complication is an extension on either
# platform — but in a copy phase belonging to the WATCH target, because the
# bundle it goes inside is FarCoolerWatch.app. Put in the phone's phase it
# compiles, links, signs, installs, and produces a watch with no complication
# to add: nothing anywhere reports it, because nothing about it is wrong except
# which bundle it ended up in.
#
# So this checks the phase's contents AND its owner. A phase with the right
# subfolder spec attached to the wrong target is exactly the failure, and the
# first half alone would pass on it.
watch_ext_phase="$(awk '/Embed Watch Extensions \*\/ = \{$/,/^\t\t\};$/' <<< "$copy_phases")"
check "there is an Embed Watch Extensions phase" \
  "yes" "$([ -n "$watch_ext_phase" ] && echo yes || echo no)"
check "the complication is embedded at dstSubfolderSpec 13" \
  "yes" "$(has 'dstSubfolderSpec = 13;' "$watch_ext_phase")"
check "it is the complication that is embedded there" \
  "yes" "$(has 'FarCoolerWatchWidgets.appex' "$watch_ext_phase")"
check "the complication is not in the phone's extensions phase" \
  "no" "$(has 'FarCoolerWatchWidgets.appex' "$extensions_phase")"

# The owner. Every native target's object opens with its own id and a name
# comment, so the id of a phase and the id list of a target can be matched
# without hardcoding either.
native_targets="$(awk '/Begin PBXNativeTarget section/,/End PBXNativeTarget section/' "$PBX")"
target_block() { awk -v want="/* $1 */ = {" 'index($0, want) { on = 1 } on { print } on && /^\t\t\};$/ { on = 0 }' <<< "$native_targets"; }
object_id() { grep -o '^[0-9A-F]\{24\}' <<< "$(grep -F "$1" <<< "$2" | tr -d '\t')"; }

watch_target_block="$(target_block FarCoolerWatch)"
app_target_block="$(target_block FarCooler)"
watch_ext_phase_id="$(object_id '/* Embed Watch Extensions */ = {' "$copy_phases")"
check "the Embed Watch Extensions phase has an id" \
  "yes" "$([ -n "$watch_ext_phase_id" ] && echo yes || echo no)"
check "that phase is a build phase of the watch app" \
  "yes" "$(has "$watch_ext_phase_id" "$watch_target_block")"
check "it is not a build phase of the phone app" \
  "no" "$(has "$watch_ext_phase_id" "$app_target_block")"

# --- the app builds the watch app before embedding it --------------------
#
# Without the dependency, the app can embed a stale watch app or none at all,
# and the only symptom is a watch showing yesterday's build.
check "the app depends on the watch target" \
  "yes" "$(has 'remoteInfo = FarCoolerWatch;' "$(cat "$PBX")")"

# And the watch app on the complication, for the same reason one level down: a
# watch app that does not depend on it embeds whatever .appex was last built,
# which on a clean tree is none at all — a watch app that ships with an empty
# PlugIns folder and nothing to say about it.
#
# Followed through the objects rather than asserted on `remoteInfo` alone,
# because a PBXTargetDependency that exists but is listed under no target is
# precisely the shape a hand-edit leaves behind, and it reads the same as a
# correct one from any single line.
deps="$(awk '/Begin PBXTargetDependency section/,/End PBXTargetDependency section/' "$PBX")"
watch_ext_target_id="$(object_id '/* FarCoolerWatchWidgets */ = {' "$native_targets")"
watch_ext_dep_id="$(
  awk -v want="$watch_ext_target_id;" '
    $2 == "=" && $3 == "{" { id = $1 }
    $1 == "target" && $3 == want { print id }
  ' <<< "$deps"
)"
check "there is a dependency on the complication's target" \
  "yes" "$([ -n "$watch_ext_dep_id" ] && echo yes || echo no)"
check "the watch app is what depends on it" \
  "yes" "$(has "$watch_ext_dep_id" "$watch_target_block")"

# --- bundle identifiers nest ---------------------------------------------
#
# A watch app's identifier has to be the phone app's with one component
# appended. That is the rule for the relationship, not a convention: a name that
# does not nest fails to sign against the app's profile, at install time.
app_id="$(setting FarCooler PRODUCT_BUNDLE_IDENTIFIER)"
check "the app has a bundle id at all" \
  "yes" "$([ -n "$app_id" ] && echo yes || echo no)"
check "the watch bundle id is the app's plus .watchkitapp" \
  "$app_id.watchkitapp" "$(setting FarCoolerWatch PRODUCT_BUNDLE_IDENTIFIER)"
check "the activity extension's bundle id nests too" \
  "$app_id.activity" "$(setting FarCoolerActivity PRODUCT_BUNDLE_IDENTIFIER)"
check "the notification service extension's bundle id nests too" \
  "$app_id.notify" "$(setting FarCoolerNotify PRODUCT_BUNDLE_IDENTIFIER)"

# The watch app is a companion of THIS app. Wrong or absent, it installs as a
# standalone watch app and never pairs with the app it shipped inside.
check "the watch names its companion app" \
  "$app_id" "$(setting FarCoolerWatch INFOPLIST_KEY_WKCompanionAppBundleIdentifier)"

# --- every target that reads the snapshot names the same App Group -------
#
# The group cannot be derived from a target's own bundle identifier, because
# every target's differs — so it is a build setting repeated on each, and the
# whole point is that the repetitions agree. One disagreeing target is a widget,
# an extension or a watch reading a container nobody writes: permanently empty,
# and nothing says why.
for t in FarCooler FarCoolerActivity FarCoolerNotify FarCoolerWatch FarCoolerWatchWidgets; do
  group="$(setting "$t" FARCOOLER_APP_GROUP)"
  check "$t carries FARCOOLER_APP_GROUP" \
    "yes" "$([ -n "$group" ] && echo yes || echo no)"
  check "$t names this channel's group" "group.$app_id" "$group"
done

distinct="$(grep -o 'FARCOOLER_APP_GROUP = [^;]*;' "$PBX" | sort -u | wc -l | tr -d ' ')"
check "no target names a different App Group" "1" "$distinct"

# --- and each of them can actually READ it at runtime --------------------
#
# `SnapshotStore.groupIdentifier` reads the `FarCoolerAppGroup` Info.plist key,
# not the build setting — so the setting alone buys nothing. It must reach a
# plist, and it cannot do that through `INFOPLIST_KEY_FarCoolerAppGroup`:
# Xcode honors `INFOPLIST_KEY_<key>` only for keys it already knows, and drops a
# custom one with no warning and no error. That was measured on the built watch
# bundle, where the setting produced no key at all. So every one of these
# targets names a real Info.plist file, and every one of those files expands
# $(FARCOOLER_APP_GROUP) under that key. Missing, `SnapshotStore.read()` returns
# nil — indistinguishable from "no snapshot has ever arrived".
for t in FarCooler FarCoolerActivity FarCoolerNotify FarCoolerWatch FarCoolerWatchWidgets; do
  plist="$(setting "$t" INFOPLIST_FILE)"
  check "$t names an Info.plist file" \
    "yes" "$([ -f "apps/ios/$plist" ] && echo yes || echo no)"
  check "$t's Info.plist carries FarCoolerAppGroup" \
    "yes" \
    "$(grep -A1 '<key>FarCoolerAppGroup</key>' "apps/ios/$plist" 2>/dev/null \
        | grep -q '<string>\$(FARCOOLER_APP_GROUP)</string>' && echo yes || echo no)"
done

# --- the activity extension can deep-link back into the app --------------
#
# The widgets' tap targets open `$(FARCOOLER_URL_SCHEME)://…`, and the scheme is
# per channel. Unset, the expression expands to nothing and the tap opens no app
# at all — a widget that looks right and does nothing.
scheme="$(setting FarCoolerActivity FARCOOLER_URL_SCHEME)"
check "the activity extension carries FARCOOLER_URL_SCHEME" \
  "yes" "$([ -n "$scheme" ] && echo yes || echo no)"
check "it is the same scheme the app registers" \
  "$(setting FarCooler FARCOOLER_URL_SCHEME)" "$scheme"

# --- the watch really is built for watchOS -------------------------------
#
# The project-level settings fix SDKROOT = iphoneos and TARGETED_DEVICE_FAMILY
# = "1,2" for everything. A watch target that fails to override them compiles
# for the phone and produces a bundle no watch can run.
check "the watch target builds against the watchOS SDK" \
  "watchos" "$(setting FarCoolerWatch SDKROOT)"
check "the watch target is watch-only" \
  "4" "$(setting FarCoolerWatch TARGETED_DEVICE_FAMILY)"
check "the watch target has a watchOS deployment target" \
  "yes" \
  "$([ -n "$(setting FarCoolerWatch WATCHOS_DEPLOYMENT_TARGET)" ] && echo yes || echo no)"

# --- no build file is in two sources phases ------------------------------
#
# A PBXBuildFile is "this file, compiled into THIS target", so a file several
# targets compile needs one per target over the single file reference. Reusing
# one puts the same object in two phases and Xcode fails the build with
# "multiple commands produce" — which is at least loud, but it is also trivially
# checkable, and the four parallel id maps in the generator exist only to
# prevent it.
# `|| true` on the grep, so that finding nothing reaches the assertion below
# instead of killing the script. Under `set -e` with `pipefail` an empty grep
# exits 1 here and the whole run dies at this line having printed neither a FAIL
# nor the tally — a real failure reported as no output at all.
refs="$(
  awk '/Begin PBXSourcesBuildPhase section/,/End PBXSourcesBuildPhase section/' "$PBX" \
    | { grep -o '^\t\t\t\t[0-9A-F]\{24\}' || true; } | tr -d '\t'
)"
# Non-empty FIRST, because the comparison below cannot fail on an empty list:
# `wc -l` and `sort -u | wc -l` both answer 1 for no input, so a pattern that
# stopped matching — a changed indent, a renamed section — would turn this into
# an assertion that passes without reading anything.
check "the sources phases reference build files at all" \
  "yes" "$([ -n "$refs" ] && echo yes || echo no)"
check "no build file appears in two sources phases" \
  "$(wc -l <<< "$refs" | tr -d ' ')" \
  "$(sort -u <<< "$refs" | wc -l | tr -d ' ')"

echo "$PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ]
