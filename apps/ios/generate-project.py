#!/usr/bin/env python3
"""Generate the Xcode project for the iOS app.

An .xcodeproj is generated state, not source. Keeping the generator in git
rather than the 300-line plist it produces means the project can be reviewed,
diffed, and regenerated after a source file is added — and it cannot drift into
the unreadable mess that a hand-edited pbxproj becomes.

    ./apps/ios/generate-project.py
"""

import os
import pathlib
import shutil
import subprocess
import uuid

from pathlib import Path

SOURCES = [
    "FarCoolerApp.swift",
    # Every add-shaped screen in one place, and the hub the five old entry
    # points collapsed into. See its own doc comment.
    "AddFlow.swift",
    "ClientCore.swift",
    "Connection.swift",
    "FleetSnapshotWriter.swift",
    # What is left of the app's navigation: the connection, the four phases,
    # and the deep link. `NeedsYou.swift` was here beside it — the inbox that
    # used to be the front door — and is gone: the shell's overview is the
    # fleet screen, sorted by what needs you.
    "FleetView.swift",
    # `Model.swift` was here. It is `CoreModel.swift` in `AGENTKIT_SOURCES`
    # below now — moved so the AgentKit test target can decode a fixture into
    # `Fleet`, `Workspace` and `Terminal`, which nothing could while they sat in
    # this list. See its own header for why that move is not a merge with the
    # Mac's model.
    "Notifications.swift",
    "LiveActivities.swift",
    "Reachability.swift",
    "Settings.swift",
    "Store.swift",
    "Theme.swift",
    "QuickTask.swift",
    "TaskComposer.swift",
    "VTCore.swift",
    "TerminalSession.swift",
    "TerminalView.swift",
    # One tab of a workspace, and which tab somebody chose. Both were nested in
    # screens that no longer exist — `Pane` in `WorkspaceView.swift`, the pane
    # host the shell replaced, and `PaneFocus` as `Route.Focus` inside
    # `FleetView`'s navigation enum. `TerminalTabStrip.swift` went with them:
    # the ribbon and the column on the shell's bar are what a workspace's tabs
    # look like now.
    "Pane.swift",
    "PaneFocus.swift",
    # The navigation shell: one bar that IS the workspace, the column of its
    # tabs, and the all-workspaces view. It IS the app's navigation now —
    # `-shell-harness` stands it on a canned fleet and is the only flag left.
    # Its pure model lives in AgentKit so `swift test` can reach it — only the
    # views are here.
    "ShellBar.swift",
    "ShellOverview.swift",
    "ShellRootView.swift",
    # The other two thirds of what `ShellRootView.swift` used to be. One type,
    # three files: the container, the page's own layer, and the finger. The
    # seam is what each of them can be reviewed against — a change to the
    # motion is not a change to the gesture, and neither is a change to the
    # layering.
    "ShellPageLayer.swift",
    "ShellDrag.swift",
    # The retained pane set, which is the invariant rather than a detail: a
    # pane is mounted once, keyed by tab id, and MOVED. See its header.
    "ShellPaneTrack.swift",
    # A bar that belongs to the PANE rather than to the shell, at the top where
    # the keyboard cannot cover it. It carries what a pane can do — the diff's
    # review options, an image into a tty, the worktree's own removal — which
    # is what the pane host's toolbar carried before the shell replaced it.
    "ShellPaneBar.swift",
    # The shell over a real runner: the fleet mapped onto its vocabulary, and
    # terminals in the slots.
    "ShellScreen.swift",
    "ShellHarness.swift",
    "DockedBar.swift",
    "ImagePaste.swift",
    "AgentStream.swift",
    "AgentView.swift",
    # The phone's half of the watch link. Not under `FarCoolerWatch/`: it runs
    # on the phone, holds the connection, and makes the core calls the watch
    # cannot.
    "WatchLinkHost.swift",
    "RunnerSettings.swift",
    "ThemeEditorView.swift",
    "AdapterEditorView.swift",
    "Changes.swift",
    "ChangesView.swift",
    # Where the reader was, and this app's half of getting what they want to
    # say to an agent — see its own doc comment. The queue itself is shared with
    # the Mac now; it is `ReviewComments.swift` in `AGENTKIT_SOURCES` below.
    "ChangesReview.swift",
    "BranchAndStack.swift",
]

# The enrollment ceremony, in `Far Cooler/Ceremony/`.
#
# A directory of its own rather than five more names in SOURCES, because
# SOURCES is basenames the generator assumes sit directly in `FarCooler/` —
# the same reason `Fonts/` has `fontsGroup`. `ceremonyGroup` below supplies
# the directory part, and these files are compiled into the app target exactly
# like every other source.
CEREMONY_SOURCES = [
    "CodeImage.swift",
    "CodeScanner.swift",
    "CeremonyStore.swift",
    "AddDeviceView.swift",
    "JoinView.swift",
]

# `AgentKit`'s own sources, compiled directly into this target rather than
# vended as a real module the way `apps/macos/Package.swift` vends it via
# SwiftPM — iOS has no SwiftPM project here, only this generated one. So
# `Transcript`, `AgentEvent` and friends simply become part of the "Far Cooler"
# module, and nothing under `Far Cooler/` writes `import AgentKit`. They live
# outside `Far Cooler/`, which is why `SOURCES` above — basenames the generator
# assumes sit in that one directory — cannot hold them; `agentKitGroup` below
# gives them a group of their own instead, the same way `fontsGroup` does for
# `Fonts/`.
AGENTKIT_SOURCES = [
    # The navigation shell's pure model: the flat sequence across the fleet,
    # the axis lock, the release decisions, precedence, search. In AgentKit
    # rather than beside its views because the iOS target has no unit tests —
    # only UI tests — and these are the rules with no screen in them. See
    # `ShellNavigationTests`.
    "ShellNavigation.swift",
    # The other half of the same argument: what the shell MOVES by, and the
    # flying page's geometry. Here rather than beside the views because the
    # iOS target has no unit tests — a transform inside a `View` can be checked
    # by nothing but a person swiping at it. See `ShellFlightTests`.
    "ShellFlight.swift",
    # The glance surfaces' colour, their one state mark and their type scale, in
    # FOUR lists: this
    # one, `activity_build_ids`, `WATCH_AGENTKIT_SOURCES` and
    # `WATCH_WIDGET_AGENTKIT_SOURCES`. Same argument as `FleetSnapshot.swift`
    # below — several targets, several binaries, one file — and here the file
    # is the reason three copies of one colour rule stopped existing: the two
    # `glanceTint` functions in the widget extensions and `ShellMarkView`'s four
    # literals were three places one amber could drift.
    #
    # NOT in `FleetSnapshot.swift`, deliberately. That file's own doc
    # (`FleetSnapshot.swift:466-470`) says it "has no business importing
    # SwiftUI: it is the wire's shape, compiled into a notification service
    # extension and a watch complication that have no views at all" — and the
    # notification service extension really does compile it and really does
    # draw nothing, so a `Color` in there would break `FarCoolerNotify`. This is
    # where the SwiftUI half lives instead, and `NOTIFY_SOURCES` does not carry
    # it.
    "GlancePalette.swift",
    "GlanceMark.swift",
    "GlanceType.swift",
    # The account, and the two screens it makes meaningful. Shared because
    # signing in is identical on both platforms and because the words under the
    # button — that an account buys notifications and nothing else — must not
    # come to differ between them.
    "Account.swift",
    "AccountDevicesView.swift",
    "AccountSection.swift",
    "AppVersion.swift",
    # The one type the app and the widget extension both have to agree on, down
    # to the field names — ActivityKit matches a push to a running card by the
    # type's name and decodes the payload straight into its ContentState. It is
    # therefore in BOTH this list and `ACTIVITY_SOURCES` below: two targets, two
    # binaries, one file. See `activity_build_ids` for why that needs a second
    # set of build ids rather than reusing these.
    "AgentActivityAttributes.swift",
    "AgentEvent.swift",
    # What the Test button in an adapter editor says. Shared because the type
    # and its words were declared three times — here, on the Mac and on
    # Android — and one of the three sentences had quietly become false on one
    # platform only. Not in `WATCH_AGENTKIT_SOURCES`: the watch edits nothing.
    "AdapterTest.swift",
    # The intent layer, and in `activity_build_ids` below for the same reason
    # `AgentActivityAttributes.swift` is: TWO targets, one file. The widget
    # extension has to construct `AnswerPermissionIntent` to put it on a button
    # and the app has to perform it, and a second declaration of the type would
    # be two intents with one name — which the system resolves by picking one,
    # silently. `GlancePermissions.swift` is the App Group file they pass the
    # agent's own option names through; neither target can reach a runner from a
    # render, so the words on those buttons have to be written down somewhere
    # both can open.
    "AnswerPermissionIntent.swift",
    "GlancePermissions.swift",
    "Composer.swift",
    # The shapes the FFI sends, which this app used to call `Model.swift` and
    # keep in `SOURCES`. Only in THIS list, and that is the point of the move
    # rather than a detail of it: everything in the file is `internal`, so the
    # Mac — which depends on AgentKit as a real module and has its own
    # `Workspace` and `Terminal` that DISAGREE with these — cannot see a single
    # name from it. The phone compiles AgentKit's sources into its own module,
    # so on this side it is exactly the file it was, under a new name. What it
    # bought is a test: `FleetDecodeTests` decodes a fixture transcribed from
    # `Session::fleet` into these types on the host, with no simulator, which
    # the app target's UI-testing bundle could never do.
    "CoreModel.swift",
    # In this list AND in `WATCH_AGENTKIT_SOURCES` below: the phone and the
    # watch are two binaries that have to agree about these messages down to the
    # key names, which is the whole reason the file exists. A `WatchRequest` the
    # phone could not construct would leave `WatchLinkHost` unable to answer
    # anything.
    "WatchLink.swift",
    "DiffComputation.swift",
    # Which changed files a tool wrote. Only in THIS list: the watch and the
    # two extensions show a count and a status, never a file list, so nothing
    # there has a reading order to put a lockfile at the end of. It is here
    # rather than in `SOURCES` because the Mac's diff pane walks the same order
    # off the same rule, and the day the daemon answers this question there has
    # to be one caller-side rule to demote instead of two.
    "GeneratedFiles.swift",
    # In this list AND in `activity_build_ids` AND in `notify_build_ids` below,
    # for the same reason `AgentActivityAttributes.swift` is in two: THREE
    # targets, three binaries, one file. The widget renders the snapshot the app
    # writes and the notification service extension refreshes it while the app
    # sleeps; a second copy of either type is two definitions of the file all
    # three share.
    "FleetSnapshot.swift",
    "SnapshotStore.swift",
    # The markdown renderer, shared for the same reason the reducer is: the
    # phone drew agent replies as plain `Text`, so a table arrived as a wall of
    # pipes and a heading as a line beginning with a hash. Same conversation,
    # unreadable on one of the two clients.
    "MarkdownView.swift",
    # The half of push registration that is not platform-specific. Both apps
    # had it verbatim; only the device label differs.
    "PushRegistration.swift",
    "RelaySection.swift",
    # The review comment queue, which the phone wrote and the Mac's diff pane
    # now shares. Only in THIS list: it holds unsent notes keyed by workspace,
    # and the watch and the two extensions neither review a diff nor have a
    # composer to put one in. The send itself is a closure the app supplies —
    # `ReviewCommentQueue.phone` in `ChangesReview.swift` — so nothing in
    # AgentKit reaches the FFI.
    "ReviewComments.swift",
    "TokenStore.swift",
    "VersionSection.swift",
    "Transcript.swift",
]
# The widget extension's own sources, in `apps/ios/FarCoolerActivity/`.
#
# A second target and a second binary, which is not a structural choice: WidgetKit
# renders Live Activities out of process so a card keeps drawing when the app is
# not running, and that is the only case the feature exists for. An app cannot
# draw its own Live Activity.
#
# Nothing from `SOURCES` is available here. The extension has no Rust core, no
# connection, and no fleet — everything it draws arrives in the push that started
# it, which is why `AgentActivityAttributes` carries plain strings rather than
# ids to look up.
#
# The home screen and lock screen widgets live here too, and they are the reason
# `FleetSnapshot.swift` and `SnapshotStore.swift` appear in `activity_build_ids`
# below: those two are the extension's only window onto the fleet, since it has
# no connection to ask.
ACTIVITY_SOURCES = [
    "FarCoolerActivityBundle.swift",
    "AgentActivityWidget.swift",
    "FleetWidget.swift",
]

# The notification service extension's own sources, in `apps/ios/FarCoolerNotify/`.
#
# A third target because a service extension is a separate process that iOS
# starts for an incoming push whether or not the app is running — which is the
# only moment the snapshot can be refreshed on a phone in a pocket.
NOTIFY_SOURCES = ["NotificationService.swift"]

# The watchOS app's own sources, in `apps/ios/FarCoolerWatch/`.
#
# A fourth target and a fourth binary because a watchOS app is a separate
# executable that runs on separate hardware. It is not an extension of the
# phone app; it is embedded in it only so the phone's install carries it to the
# paired watch.
WATCH_SOURCES = [
    "FarCoolerWatchApp.swift",
    "WatchLinkClient.swift",
    "FleetListView.swift",
    "AgentDetailView.swift",
    "ComposeView.swift",
    "PermissionView.swift",
    "TranscriptView.swift",
]

# AgentKit files the watch target compiles, named one by one — the same way
# `AGENTKIT_SOURCES` names the phone's.
#
# It is one file at a time and NOT the whole package, and that is not fastidious-
# ness. `Account.swift` is built on `ASWebAuthenticationSession`, whose
# `ASWebAuthenticationPresentationContextProviding` and `ASPresentationAnchor`
# simply do not exist on watchOS, so compiling AgentKit wholesale for
# `arm64_32-apple-watchos` fails outright. These three were measured to
# typecheck for that triple and they are the three the watch actually needs:
# `WatchLink` is the vocabulary it speaks to the phone, and `FleetSnapshot` /
# `SnapshotStore` are the model it renders and the container it caches into.
#
# `AgentActivityAttributes.swift` is deliberately NOT here even though the other
# two extension lists carry it. It is `#if os(iOS)` guarded end to end, so on
# watchOS it compiles to an empty translation unit — a build id and a phase
# entry claiming a dependency the watch does not have. ActivityKit has no
# watchOS counterpart and the watch renders no Live Activity; listing the file
# for symmetry would tell the next reader of this list something untrue.
WATCH_AGENTKIT_SOURCES = [
    "WatchLink.swift",
    # The three reachability states. In AgentKit and not in `FarCoolerWatch/`
    # for one reason: `swift test` cannot build a watchOS app target, so the
    # rule that a watch must never offer an action it cannot deliver had no
    # test standing behind it while it lived over there. Compiled by the watch
    # alone — the phone has no use for it.
    "WatchState.swift",
    "FleetSnapshot.swift",
    "SnapshotStore.swift",
    # The colour, the mark and the type scale, which the watch app's own detail
    # header draws at
    # the 22pt lone-indicator size. Measured to typecheck for
    # `arm64_32-apple-watchos` before being listed here, which is the bar this
    # list sets a few lines above — the conversion in `GlancePalette` is plain
    # `Double` arithmetic and the mark is plain SwiftUI shapes, precisely so it
    # clears it. Nothing in either file reaches for `UIColor`, an asset catalog
    # or a dynamic colour provider, none of which the watch has.
    "GlancePalette.swift",
    "GlanceMark.swift",
    "GlanceType.swift",
]

# The subset of the above that no OTHER target compiles.
#
# Those files still need a `PBXFileReference` and a place in `agentKitGroup`,
# and they must NOT join `AGENTKIT_SOURCES` — that list is precisely "what the
# phone app compiles", and a file the watch needs quietly becoming a file the
# phone builds is how a target grows sources nobody chose for it.
#
# `WatchState.swift` and nothing else, at the time of writing: `WatchLink.swift`
# is in both lists because the phone genuinely needs to construct the same
# messages, and the state machine is the one piece the watch alone renders. The
# derivation is the rule rather than the list — the next file the watch alone
# needs lands here without anybody remembering to put it here.
WATCH_ONLY_AGENTKIT_SOURCES = [n for n in WATCH_AGENTKIT_SOURCES if n not in AGENTKIT_SOURCES]

# The watch's complication, in `apps/ios/FarCoolerWatchWidgets/`.
#
# A FIFTH target and a fifth binary, and the one that makes the watch app worth
# having: a complication is what a person sees without raising the app, and the
# whole premise of the watch here is that you learn an agent is waiting without
# deciding to go and look.
#
# It cannot live inside the watch app for the same structural reason
# FarCoolerActivity cannot live inside the phone app: WidgetKit renders out of
# process, so a complication keeps drawing while its app is not running, which
# is every moment this feature exists for. An app cannot draw its own
# complication.
#
# It is embedded in the WATCH app's PlugIns, never the phone's — see
# `watchWidgetEmbedPhase`.
WATCH_WIDGET_SOURCES = ["WatchFleetWidget.swift"]

# AgentKit files the complication compiles.
#
# Five, and no more: the extension has no connection, no `WatchLinkClient` and
# no screens. Its only window onto the fleet is the file the watch app writes
# into the shared container, so `FleetSnapshot` is the model it renders and
# `SnapshotStore` is how it opens the file. `WatchLink.swift` is deliberately
# absent — this target never speaks to the phone, and listing the vocabulary for
# symmetry would claim a capability it does not have.
#
# The other two are what it renders WITH. A complication is the surface a person
# sees without deciding to look, so it is the one that most needs to draw the
# same mark in the same amber as the phone widget beside it — which is what
# these two files are for, and why the alternative was a second copy of
# `glanceTint` on the wrist. There already was one.
WATCH_WIDGET_AGENTKIT_SOURCES = [
    "FleetSnapshot.swift",
    "SnapshotStore.swift",
    "GlancePalette.swift",
    "GlanceMark.swift",
    "GlanceType.swift",
]

UI_TEST_SOURCES = [
    "ChangesPullRequestTests.swift",
    "KeyboardTabStripTests.swift",
    "ShellGestureTests.swift",
    # Photographs the terminal renderer's own fixture and compares cells. Needs
    # no runner: the grid it draws is built in the app, so this one cannot skip
    # itself green when the demo daemon is down.
    "TerminalLigatureTests.swift",
    # The other half of the terminal's pair, for panes the app did not write
    # the scroller of. See its header.
    "ShellPaneScrollTests.swift",
    "TerminalScrollTests.swift",
]

FRAMEWORKS = ["farcooler_vt.xcframework", "farcooler_client.xcframework"]

# Bundled rather than downloaded — see Fonts/README.md for why a terminal
# app cannot treat its own typeface as optional. Basenames only: they sit in
# their own `fontsGroup` below (path "Fonts"), which is what supplies the
# directory part. `Info.plist`'s `UIAppFonts` lists the same two filenames —
# the name CoreText scans the bundle root for at launch — and the two lists
# have to agree or a font that "shipped" never actually registers.
FONTS = [
    "IosevkaNerdFontMono-Regular.ttf",
    "IosevkaNerdFontMono-Bold.ttf",
]

# One icon source for both Apple apps. The asset catalog lives under `apps/shared`
# so iOS can compile it directly and the macOS renderer can use the same master.
ASSET_CATALOG = "Assets.xcassets"

KEYS = [
    "project", "target", "mainGroup", "productsGroup", "sourcesGroup",
    "frameworksGroup", "fontsGroup", "ceremonyGroup", "agentKitGroup", "product",
    "sourcesPhase",
    "frameworksPhase", "resourcesPhase", "buildConfigList", "targetConfigList",
    "debug", "release", "targetDebug", "targetRelease",
    # The widget extension: its own target, product, group, sources phase and
    # configuration pair, plus the three objects that attach it to the app —
    # `embedPhase` copies the built .appex into the app's PlugIns, and
    # `activityDependency`/`activityProxy` are the pair Xcode requires to say
    # "build that target first". Without the dependency the app can be built
    # against a stale extension, or none at all, and the only symptom is a Live
    # Activity that never appears.
    "activityTarget", "activityProduct", "activityGroup", "activitySourcesPhase",
    "activityConfigList", "activityDebug", "activityRelease", "embedPhase",
    "activityDependency", "activityProxy",
    # The notification service extension: the same nine objects the activity
    # extension needs, for the same reasons. It is a second .appex embedded in
    # the same Embed Foundation Extensions phase.
    "notifyTarget", "notifyProduct", "notifyGroup", "notifySourcesPhase",
    "notifyConfigList", "notifyDebug", "notifyRelease",
    "notifyDependency", "notifyProxy",
    # The watchOS app: the same nine objects again, but `watchEmbedPhase` is a
    # phase of its OWN rather than a second file in `embedPhase`. A watch app
    # goes into `Watch/` inside the app bundle, an .appex goes into
    # `PlugIns/`, and those are two different `dstSubfolderSpec` values — 16 and
    # 13. Putting the watch app in the extensions' phase builds perfectly and
    # then fails at install or at launch, which is why it gets its own object
    # here rather than sharing one.
    "watchTarget", "watchProduct", "watchGroup", "watchSourcesPhase",
    "watchConfigList", "watchDebug", "watchRelease", "watchEmbedPhase",
    "watchDependency", "watchProxy",
    # The watch's own resources phase, carrying the asset catalog. A watch app
    # with no icon builds and signs perfectly and is then REFUSED AT UPLOAD --
    # "Missing Info.plist value ... CFBundleIconName" and "No icons found for
    # watch application" -- so nothing before App Store validation catches it.
    "watchResourcesPhase",
    # The watch's complication: the same nine objects a third time, plus
    # `watchWidgetEmbedPhase`. That phase is 13/PlugIns like the phone's
    # `embedPhase` — an .appex is an .appex on either platform — but it belongs
    # to the WATCH target's `buildPhases` rather than the app's, because the
    # bundle it goes inside is FarCoolerWatch.app. Put in the phone's phase it
    # builds, signs, and installs a complication that watchOS never sees.
    # `watchWidgetDependency` is likewise a dependency of the WATCH target: the
    # watch app has to be told to build this before embedding it, or it embeds
    # whatever was there last.
    "watchWidgetTarget", "watchWidgetProduct", "watchWidgetGroup",
    "watchWidgetSourcesPhase", "watchWidgetConfigList", "watchWidgetDebug",
    "watchWidgetRelease", "watchWidgetEmbedPhase", "watchWidgetDependency",
    "watchWidgetProxy",
    # Real-device interaction regressions. The target depends on the app but is
    # never embedded in it; Xcode installs its runner only for a test action.
    "uiTestTarget", "uiTestProduct", "uiTestGroup", "uiTestSourcesPhase",
    "uiTestFrameworksPhase", "uiTestConfigList", "uiTestDebug", "uiTestRelease",
    "uiTestDependency", "uiTestProxy",
]


def oid(seed):
    """A stable 24-hex object id, so regenerating produces an identical file."""
    return uuid.uuid5(uuid.NAMESPACE_URL, "farcooler-ios/" + seed).hex[:24].upper()


ids = {
    name: oid(name)
    for name in SOURCES + CEREMONY_SOURCES + AGENTKIT_SOURCES + ACTIVITY_SOURCES
    + NOTIFY_SOURCES + WATCH_SOURCES + WATCH_ONLY_AGENTKIT_SOURCES
    + WATCH_WIDGET_SOURCES + UI_TEST_SOURCES
    + FRAMEWORKS + FONTS + [ASSET_CATALOG]
}
build_ids = {
    name: oid("build/" + name)
    for name in SOURCES + CEREMONY_SOURCES + AGENTKIT_SOURCES + ACTIVITY_SOURCES
    + UI_TEST_SOURCES + FRAMEWORKS + FONTS + [ASSET_CATALOG]
}

# A PBXBuildFile is "this file, compiled into THIS target", so a file two targets
# both compile needs two of them pointing at one PBXFileReference. Reusing a
# single build id puts the same object in two sources phases, which Xcode reads
# as one target's file appearing twice and fails with "multiple commands
# produce". `AgentActivityAttributes.swift` is the whole reason this exists; the
# extension's own sources are in here too so the two lists stay symmetrical.
activity_build_ids = {
    name: oid("activity-build/" + name)
    for name in ACTIVITY_SOURCES
    + [
        "AgentActivityAttributes.swift",
        "FleetSnapshot.swift",
        "SnapshotStore.swift",
        # The card's buttons. `AnswerPermissionIntent` is what a button is wired
        # to and `GlancePermissions` is where its labels come from — the
        # extension can reach no runner, so the agent's own option names arrive
        # through the App Group or not at all.
        "AnswerPermissionIntent.swift",
        "GlancePermissions.swift",
        # What every family in this extension is drawn in and drawn WITH. Six
        # home-screen and lock-screen families plus four Dynamic Island
        # presentations all read one amber and one mark out of these, which is
        # the point: the widget and the Live Activity are on the same phone and
        # a person meets both within a minute.
        "GlancePalette.swift",
        "GlanceMark.swift",
        "GlanceType.swift",
    ]
}

# A third set, on exactly the reasoning above. `FleetSnapshot.swift` and
# `SnapshotStore.swift` are now compiled into three targets, so they need three
# build ids over the one file reference — the shared types are the whole reason
# this extension can write a snapshot the widget will understand.
notify_build_ids = {
    name: oid("notify-build/" + name)
    for name in NOTIFY_SOURCES + ["FleetSnapshot.swift", "SnapshotStore.swift"]
}

# A fourth set, on exactly the reasoning above. `FleetSnapshot.swift` and
# `SnapshotStore.swift` are now compiled into FOUR targets over the one file
# reference each — the watch renders the same snapshot the phone writes, and a
# second copy of either type on the watch side is a second definition of the
# file the whole seam is built on.
watch_build_ids = {
    name: oid("watch-build/" + name)
    for name in WATCH_SOURCES + WATCH_AGENTKIT_SOURCES
}

# A FIFTH set, on exactly the reasoning above. `FleetSnapshot.swift` and
# `SnapshotStore.swift` are now compiled into five targets over the one file
# reference each: the app, the widget extension, the notification service
# extension, the watch app, and the watch's complication. Five build ids, one
# definition — which is the point, because the complication reads bytes the
# watch app wrote and a second copy of either type is two answers to what those
# bytes mean.
#
# It has to be a set of its OWN rather than a reuse of `watch_build_ids`: a
# PBXBuildFile is "this file, compiled into THIS target", so sharing one would
# put a single object in two sources phases and Xcode fails with "multiple
# commands produce".
watch_widget_build_ids = {
    name: oid("watch-widget-build/" + name)
    for name in WATCH_WIDGET_SOURCES + WATCH_WIDGET_AGENTKIT_SOURCES
}

# And the catalog the watch compiles, on the same rule as every set above: a
# PBXBuildFile is "this file, compiled into THIS target", so the phone's
# `build_ids[ASSET_CATALOG]` cannot be reused here.
watch_asset_build_id = oid("watch-build/" + ASSET_CATALOG)

P = {key: oid(key) for key in KEYS}

# The built .appex bundles, copied into the app's PlugIns folder.
#
# `RemoveHeadersOnCopy` is what Xcode's own template writes here. It is not
# optional decoration: without it the copy carries headers into the bundle and
# App Store validation rejects the upload.
#
# Two ids and ONE phase. The app embeds two extensions now, and a second copy
# phase would be a second folder to keep in step rather than a second file in
# the one that already exists.
EMBED_BUILD_ID = oid("build/embed-activity")
EMBED_NOTIFY_BUILD_ID = oid("build/embed-notify")

# The built watch app, copied into the app's `Watch/` folder.
#
# A third id but NOT a third file in the phase above, because `Watch/` is not
# `PlugIns/`. See `watchEmbedPhase` for the two subfolder specs and what
# picking the wrong one costs.
EMBED_WATCH_BUILD_ID = oid("build/embed-watch")

# The built complication, copied into the WATCH app's PlugIns folder.
#
# A fourth id and a phase of its own — not a fourth file in either phase above,
# because neither of those phases belongs to the watch app. See
# `watchWidgetEmbedPhase`.
EMBED_WATCH_WIDGET_BUILD_ID = oid("build/embed-watch-widgets")


def file_refs():
    lines = []
    for name in SOURCES + CEREMONY_SOURCES + AGENTKIT_SOURCES + ACTIVITY_SOURCES \
            + NOTIFY_SOURCES + WATCH_SOURCES + WATCH_ONLY_AGENTKIT_SOURCES \
            + WATCH_WIDGET_SOURCES + UI_TEST_SOURCES:
        lines.append(
            f"\t\t{ids[name]} /* {name} */ = {{isa = PBXFileReference; "
            f"lastKnownFileType = sourcecode.swift; path = {name}; sourceTree = \"<group>\"; }};"
        )
    for name in FRAMEWORKS:
        lines.append(
            f"\t\t{ids[name]} /* {name} */ = {{isa = PBXFileReference; "
            f"lastKnownFileType = wrapper.xcframework; name = {name}; "
            f"path = Frameworks/{name}; sourceTree = \"<group>\"; }};"
        )
    for name in FONTS:
        lines.append(
            f"\t\t{ids[name]} /* {name} */ = {{isa = PBXFileReference; "
            f"lastKnownFileType = file; path = {name}; sourceTree = \"<group>\"; }};"
        )
    lines.append(
        f"\t\t{ids[ASSET_CATALOG]} /* {ASSET_CATALOG} */ = {{isa = PBXFileReference; "
        f"lastKnownFileType = folder.assetcatalog; name = {ASSET_CATALOG}; "
        f"path = build/{ASSET_CATALOG}; sourceTree = SOURCE_ROOT; }};"
    )
    lines.append(
        f"\t\t{P['product']} /* FarCooler.app */ = {{isa = PBXFileReference; "
        "explicitFileType = wrapper.application; includeInIndex = 0; "
        "path = FarCooler.app; sourceTree = BUILT_PRODUCTS_DIR; };"
    )
    lines.append(
        f"\t\t{P['activityProduct']} /* FarCoolerActivity.appex */ = "
        "{isa = PBXFileReference; explicitFileType = \"wrapper.app-extension\"; "
        "includeInIndex = 0; path = FarCoolerActivity.appex; "
        "sourceTree = BUILT_PRODUCTS_DIR; };"
    )
    lines.append(
        f"\t\t{P['notifyProduct']} /* FarCoolerNotify.appex */ = "
        "{isa = PBXFileReference; explicitFileType = \"wrapper.app-extension\"; "
        "includeInIndex = 0; path = FarCoolerNotify.appex; "
        "sourceTree = BUILT_PRODUCTS_DIR; };"
    )
    # `wrapper.application`, not `wrapper.app-extension`: a watch app is a real
    # application, which is why it gets an app's product type and an app's
    # embed slot rather than an extension's.
    lines.append(
        f"\t\t{P['watchProduct']} /* FarCoolerWatch.app */ = "
        "{isa = PBXFileReference; explicitFileType = wrapper.application; "
        "includeInIndex = 0; path = FarCoolerWatch.app; "
        "sourceTree = BUILT_PRODUCTS_DIR; };"
    )
    # `wrapper.app-extension`, unlike the watch app above it: a complication is
    # an extension whichever platform it runs on, which is exactly why it goes
    # into a PlugIns folder rather than a Watch one.
    lines.append(
        f"\t\t{P['watchWidgetProduct']} /* FarCoolerWatchWidgets.appex */ = "
        "{isa = PBXFileReference; explicitFileType = \"wrapper.app-extension\"; "
        "includeInIndex = 0; path = FarCoolerWatchWidgets.appex; "
        "sourceTree = BUILT_PRODUCTS_DIR; };"
    )
    lines.append(
        f"\t\t{P['uiTestProduct']} /* FarCoolerUITests.xctest */ = "
        "{isa = PBXFileReference; explicitFileType = wrapper.cfbundle; "
        "includeInIndex = 0; path = FarCoolerUITests.xctest; "
        "sourceTree = BUILT_PRODUCTS_DIR; };"
    )
    return "\n".join(lines)


def build_files():
    lines = []
    for name in SOURCES + CEREMONY_SOURCES + AGENTKIT_SOURCES:
        lines.append(
            f"\t\t{build_ids[name]} /* {name} in Sources */ = {{isa = PBXBuildFile; "
            f"fileRef = {ids[name]}; }};"
        )
    for name in FRAMEWORKS:
        lines.append(
            f"\t\t{build_ids[name]} /* {name} in Frameworks */ = {{isa = PBXBuildFile; "
            f"fileRef = {ids[name]}; }};"
        )
    for name in FONTS:
        lines.append(
            f"\t\t{build_ids[name]} /* {name} in Resources */ = {{isa = PBXBuildFile; "
            f"fileRef = {ids[name]}; }};"
        )
    lines.append(
        f"\t\t{build_ids[ASSET_CATALOG]} /* {ASSET_CATALOG} in Resources */ = "
        f"{{isa = PBXBuildFile; fileRef = {ids[ASSET_CATALOG]}; }};"
    )
    # The same catalog again, for the watch. One file reference, two build
    # files -- see `watch_asset_build_id`.
    lines.append(
        f"\t\t{watch_asset_build_id} /* {ASSET_CATALOG} in Resources */ = "
        f"{{isa = PBXBuildFile; fileRef = {ids[ASSET_CATALOG]}; }};"
    )
    for name, build_id in activity_build_ids.items():
        lines.append(
            f"\t\t{build_id} /* {name} in Sources */ = {{isa = PBXBuildFile; "
            f"fileRef = {ids[name]}; }};"
        )
    for name, build_id in notify_build_ids.items():
        lines.append(
            f"\t\t{build_id} /* {name} in Sources */ = {{isa = PBXBuildFile; "
            f"fileRef = {ids[name]}; }};"
        )
    for name, build_id in watch_build_ids.items():
        lines.append(
            f"\t\t{build_id} /* {name} in Sources */ = {{isa = PBXBuildFile; "
            f"fileRef = {ids[name]}; }};"
        )
    for name, build_id in watch_widget_build_ids.items():
        lines.append(
            f"\t\t{build_id} /* {name} in Sources */ = {{isa = PBXBuildFile; "
            f"fileRef = {ids[name]}; }};"
        )
    for name in UI_TEST_SOURCES:
        lines.append(
            f"\t\t{build_ids[name]} /* {name} in Sources */ = {{isa = PBXBuildFile; "
            f"fileRef = {ids[name]}; }};"
        )
    lines.append(
        f"\t\t{EMBED_BUILD_ID} /* FarCoolerActivity.appex in Embed Foundation Extensions */ = "
        f"{{isa = PBXBuildFile; fileRef = {P['activityProduct']}; "
        "settings = {ATTRIBUTES = (RemoveHeadersOnCopy, ); }; };"
    )
    lines.append(
        f"\t\t{EMBED_NOTIFY_BUILD_ID} /* FarCoolerNotify.appex in Embed Foundation Extensions */ = "
        f"{{isa = PBXBuildFile; fileRef = {P['notifyProduct']}; "
        "settings = {ATTRIBUTES = (RemoveHeadersOnCopy, ); }; };"
    )
    lines.append(
        f"\t\t{EMBED_WATCH_BUILD_ID} /* FarCoolerWatch.app in Embed Watch Content */ = "
        f"{{isa = PBXBuildFile; fileRef = {P['watchProduct']}; "
        "settings = {ATTRIBUTES = (RemoveHeadersOnCopy, ); }; };"
    )
    lines.append(
        f"\t\t{EMBED_WATCH_WIDGET_BUILD_ID} "
        "/* FarCoolerWatchWidgets.appex in Embed Watch Extensions */ = "
        f"{{isa = PBXBuildFile; fileRef = {P['watchWidgetProduct']}; "
        "settings = {ATTRIBUTES = (RemoveHeadersOnCopy, ); }; };"
    )
    return "\n".join(lines)


source_list = "\n".join(
    f"\t\t\t\t{build_ids[n]} /* {n} in Sources */,"
    for n in SOURCES + CEREMONY_SOURCES + AGENTKIT_SOURCES
)
framework_list = "\n".join(f"\t\t\t\t{build_ids[n]} /* {n} in Frameworks */," for n in FRAMEWORKS)
resource_list = "\n".join(
    f"\t\t\t\t{build_ids[n]} /* {n} in Resources */," for n in FONTS + [ASSET_CATALOG]
)
source_children = "\n".join(f"\t\t\t\t{ids[n]} /* {n} */," for n in SOURCES)
framework_children = "\n".join(f"\t\t\t\t{ids[n]} /* {n} */," for n in FRAMEWORKS)
font_children = "\n".join(f"\t\t\t\t{ids[n]} /* {n} */," for n in FONTS)
ceremony_children = "\n".join(f"\t\t\t\t{ids[n]} /* {n} */," for n in CEREMONY_SOURCES)
# The group lists every AgentKit file any iOS-side target compiles, which is a
# superset of what the phone compiles: `WATCH_ONLY_AGENTKIT_SOURCES` has file
# references but no place in the app's sources phase. A file with no group is
# still built; it is just invisible in Xcode's navigator, which is how a source
# nobody can find gets edited by nobody.
agentkit_children = "\n".join(
    f"\t\t\t\t{ids[n]} /* {n} */,"
    for n in AGENTKIT_SOURCES + WATCH_ONLY_AGENTKIT_SOURCES
)
activity_children = "\n".join(f"\t\t\t\t{ids[n]} /* {n} */," for n in ACTIVITY_SOURCES)
activity_source_list = "\n".join(
    f"\t\t\t\t{activity_build_ids[n]} /* {n} in Sources */," for n in activity_build_ids
)
notify_children = "\n".join(f"\t\t\t\t{ids[n]} /* {n} */," for n in NOTIFY_SOURCES)
notify_source_list = "\n".join(
    f"\t\t\t\t{notify_build_ids[n]} /* {n} in Sources */," for n in notify_build_ids
)
watch_children = "\n".join(f"\t\t\t\t{ids[n]} /* {n} */," for n in WATCH_SOURCES)
watch_source_list = "\n".join(
    f"\t\t\t\t{watch_build_ids[n]} /* {n} in Sources */," for n in watch_build_ids
)
watch_widget_children = "\n".join(
    f"\t\t\t\t{ids[n]} /* {n} */," for n in WATCH_WIDGET_SOURCES
)
watch_widget_source_list = "\n".join(
    f"\t\t\t\t{watch_widget_build_ids[n]} /* {n} in Sources */," for n in watch_widget_build_ids
)
ui_test_children = "\n".join(f"\t\t\t\t{ids[n]} /* {n} */," for n in UI_TEST_SOURCES)
ui_test_source_list = "\n".join(
    f"\t\t\t\t{build_ids[n]} /* {n} in Sources */," for n in UI_TEST_SOURCES
)
# `agentKitGroup`'s `path` is "../shared/AgentKit/Sources/AgentKit", which
# only resolves to the right directory (`apps/shared/AgentKit/Sources/AgentKit`)
# if the group sits directly under `mainGroup` — a sibling of `sourcesGroup`,
# not nested inside it. Nested one level deeper, inside "Far Cooler", the same
# relative path would land one directory short. See where `P['agentKitGroup']`
# is added to `mainGroup`'s children below.

COMMON = """\t\t\t\tCLANG_ENABLE_MODULES = YES;
\t\t\t\tSWIFT_VERSION = 5.0;
\t\t\t\tIPHONEOS_DEPLOYMENT_TARGET = 26.0;
\t\t\t\tSDKROOT = iphoneos;
\t\t\t\tTARGETED_DEVICE_FAMILY = "1,2";
\t\t\t\tALWAYS_SEARCH_USER_PATHS = NO;"""

# Signed ad-hoc, which needs no developer account and still produces the
# entitlements the keychain requires. It used to be CODE_SIGNING_ALLOWED = NO —
# no signature, therefore no entitlements, therefore every keychain write failed
# with errSecMissingEntitlement and the device could never keep its own key.
# A device build overrides this, which is where a real identity is required.
# A checked-in Info.plist, not GENERATE_INFOPLIST_FILE — the bundled fonts are
# why. INFOPLIST_KEY_<key> settings round-trip one scalar string per key, and
# UIAppFonts is an array of two filenames; there is no INFOPLIST_KEY_ that
# expresses that. Far Cooler/Info.plist says it directly, and every other key
# it carries is only what GENERATE_INFOPLIST_FILE was already synthesizing —
# nothing lost switching, one array gained.
def version(kind):
    """The one version number, asked of the one thing that knows it.

    Shelling out rather than re-parsing Cargo.toml here: two implementations of
    "what version is this" is exactly the drift the script exists to prevent,
    and the whole point is that the phone, the Mac, and the daemon report the
    same string. Info.plist holds $(MARKETING_VERSION) and
    $(CURRENT_PROJECT_VERSION) so that neither this file nor that one can carry
    a literal that goes stale.
    """
    script = pathlib.Path(__file__).resolve().parents[2] / "scripts" / "version.sh"
    return subprocess.run(
        [str(script), kind], capture_output=True, text=True, check=True
    ).stdout.strip()


# Public — it names the app to WorkOS, it authenticates as nothing — but read
# from the environment rather than committed, so a fork points at its own WorkOS
# project without editing source. Empty is fine: the app works, minus a sign-in
# button, and sign-in buys notifications and nothing else.
WORKOS_CLIENT_ID = os.environ.get("FARCOOLER_WORKOS_CLIENT_ID", "")

CHANNEL = version("channel")

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

# One bundle identifier per channel, so all four install side by side and none
# can see another's data — the same partition the daemon's runtime directory and
# binary name follow.
#
# It is also what decides which APNs topic a notification must be sent under,
# because `apns-topic` has to EQUAL this string. The relay for this channel
# holds it as a secret; the two have to agree or the push is rejected and
# nothing says so.
#
# Stable keeps the bare identifier. It is the App Store record that already
# exists, and changing it would create a second app rather than update the one
# people have.
BUNDLE_ID = "com.farcooler.ios" if CHANNEL == "stable" else f"com.farcooler.ios.{CHANNEL}"

# The App Group the app and its extensions share, one per channel.
#
# The keychain names its group `$(PRODUCT_BUNDLE_IDENTIFIER)` and that trick
# does NOT transfer here. An app extension's identifier is the app's with a
# component appended, so each target would expand that expression to a
# DIFFERENT group and the app and its widget would share nothing at all — which
# presents as a widget that is permanently empty on a device where the app is
# working fine.
#
# So it is a build setting instead, set on every target that needs it and
# written into each one's Info.plist for `SnapshotStore` to read. One
# definition, four channels, no second list.
APP_GROUP = f"group.{BUNDLE_ID}"

TARGET_COMMON = f"""\t\t\t\tMARKETING_VERSION = {version("marketing")};
\t\t\t\tFARCOOLER_CHANNEL = {CHANNEL};
\t\t\t\tFARCOOLER_URL_SCHEME = "{version("scheme")}";
\t\t\t\tFARCOOLER_APP_NAME = "{version("app-name-short")}";
\t\t\t\tFARCOOLER_DISPLAY_VERSION = "{version("display")}";
\t\t\t\tFARCOOLER_WORKOS_CLIENT_ID = "{WORKOS_CLIENT_ID}";
\t\t\t\tFARCOOLER_APP_GROUP = {APP_GROUP};
\t\t\t\tCURRENT_PROJECT_VERSION = {version("build")};
\t\t\t\tPRODUCT_NAME = FarCooler;
\t\t\t\tPRODUCT_BUNDLE_IDENTIFIER = {BUNDLE_ID};
\t\t\t\tINFOPLIST_FILE = FarCooler/Info.plist;
\t\t\t\tCODE_SIGN_STYLE = Manual;
\t\t\t\tCODE_SIGN_IDENTITY = "-";
\t\t\t\tCODE_SIGNING_ALLOWED = YES;
\t\t\t\tCODE_SIGNING_REQUIRED = NO;
\t\t\t\tCODE_SIGN_ENTITLEMENTS = FarCooler/FarCooler.entitlements;
\t\t\t\tENABLE_USER_SCRIPT_SANDBOXING = NO;
\t\t\t\tOTHER_LDFLAGS = "-lc++";
\t\t\t\tSWIFT_INCLUDE_PATHS = "$(BUILT_PRODUCTS_DIR)/include/vt $(BUILT_PRODUCTS_DIR)/include/client";
\t\t\t\tHEADER_SEARCH_PATHS = "$(BUILT_PRODUCTS_DIR)/include/**";
\t\t\t\tASSETCATALOG_COMPILER_APPICON_NAME = AppIcon;"""

# The widget extension's settings, deliberately NOT sharing TARGET_COMMON.
#
# It links no Rust core, so the header and framework search paths above would
# point it at an include directory it has no business in. What it must share is
# the version pair: an extension whose CFBundleVersion disagrees with the app it
# is embedded in is refused at INSTALL time, not at build time, so the mismatch
# only ever shows up on a device.
#
# FARCOOLER_APP_NAME is shared for the same reason: this NOT sharing
# TARGET_COMMON is what let the Live Activity's Info.plist hardcode "Far
# Cooler" and go unnoticed — a canary's widget and its Settings entry said "Far
# Cooler" while the app's own Home Screen tile said "FC Canary". Same
# `version.sh app-name-short` value the app target uses, so the two either say
# the same channel or neither builds.
#
# SKIP_INSTALL is required of every embedded extension. Without it the .appex is
# copied to the archive's root as well as into the app, and the upload is
# rejected for containing a top-level product that is not an application.
#
# The bundle id has to be the app's with one component appended — that is the
# rule for an app extension, not a convention, and a name that does not nest
# fails to sign against the app's profile.
#
# FARCOOLER_APP_GROUP is repeated here rather than inherited for the same reason
# it exists as a setting at all: the group has to be the SAME string in both
# targets, and every expression that derives it from this target's own bundle id
# would derive `…ios.activity`'s group instead of the app's. A widget pointed at
# a container nobody writes is permanently empty and says nothing about why.
#
# FARCOOLER_URL_SCHEME reaches this target for the first time here. The
# extension's tap targets deep-link back into the app, and the scheme is per
# channel — unset, `$(FARCOOLER_URL_SCHEME)` expands to nothing and the link
# opens no app at all.
#
# CODE_SIGN_ENTITLEMENTS: an extension that requests no entitlements gets none,
# so the App Group grant has to be asked for here too. The app's entitlement
# does not cover the .appex inside it.
ACTIVITY_COMMON = f"""\t\t\t\tMARKETING_VERSION = {version("marketing")};
\t\t\t\tCURRENT_PROJECT_VERSION = {version("build")};
\t\t\t\tFARCOOLER_APP_NAME = "{version("app-name-short")}";
\t\t\t\tFARCOOLER_APP_GROUP = {APP_GROUP};
\t\t\t\tFARCOOLER_URL_SCHEME = "{version("scheme")}";
\t\t\t\tPRODUCT_NAME = FarCoolerActivity;
\t\t\t\tPRODUCT_BUNDLE_IDENTIFIER = {BUNDLE_ID}.activity;
\t\t\t\tINFOPLIST_FILE = FarCoolerActivity/Info.plist;
\t\t\t\tCODE_SIGN_STYLE = Manual;
\t\t\t\tCODE_SIGN_IDENTITY = "-";
\t\t\t\tCODE_SIGNING_ALLOWED = YES;
\t\t\t\tCODE_SIGNING_REQUIRED = NO;
\t\t\t\tCODE_SIGN_ENTITLEMENTS = FarCoolerActivity/FarCoolerActivity.entitlements;
\t\t\t\tSKIP_INSTALL = YES;"""

# The notification service extension's settings, on the same reasoning as
# ACTIVITY_COMMON above and deliberately not sharing it either — the two
# extensions agree on everything except which extension point they are, and
# folding them into one block would make the next difference invisible.
#
# The version pair, SKIP_INSTALL, the nested bundle id and the repeated
# FARCOOLER_APP_GROUP are all required for exactly the reasons written there.
# What is NOT here is FARCOOLER_URL_SCHEME: this extension draws nothing and
# links nothing, so it has no tap target to deep-link from.
NOTIFY_COMMON = f"""\t\t\t\tMARKETING_VERSION = {version("marketing")};
\t\t\t\tCURRENT_PROJECT_VERSION = {version("build")};
\t\t\t\tFARCOOLER_APP_NAME = "{version("app-name-short")}";
\t\t\t\tFARCOOLER_APP_GROUP = {APP_GROUP};
\t\t\t\tPRODUCT_NAME = FarCoolerNotify;
\t\t\t\tPRODUCT_BUNDLE_IDENTIFIER = {BUNDLE_ID}.notify;
\t\t\t\tINFOPLIST_FILE = FarCoolerNotify/Info.plist;
\t\t\t\tCODE_SIGN_ENTITLEMENTS = FarCoolerNotify/FarCoolerNotify.entitlements;
\t\t\t\tCODE_SIGN_STYLE = Manual;
\t\t\t\tCODE_SIGN_IDENTITY = "-";
\t\t\t\tCODE_SIGNING_ALLOWED = YES;
\t\t\t\tCODE_SIGNING_REQUIRED = NO;
\t\t\t\tSKIP_INSTALL = YES;"""

# The watchOS app's settings, sharing neither TARGET_COMMON nor ACTIVITY_COMMON
# for the reasons written above and one more: this target is not built for the
# same operating system, so every setting the project level fixes for iOS has to
# be overridden here or it will be wrong rather than merely unused.
#
# SDKROOT / WATCHOS_DEPLOYMENT_TARGET / TARGETED_DEVICE_FAMILY = 4 are that
# override. The project's COMMON block says `SDKROOT = iphoneos` and
# `TARGETED_DEVICE_FAMILY = "1,2"`; a target-level setting wins, and without
# these three the target compiles for the phone and produces something no watch
# can run.
#
# The version pair, SKIP_INSTALL and the nested bundle id are all required for
# exactly the reasons ACTIVITY_COMMON gives, and the nesting rule is if anything
# stricter here: a watch app whose identifier is not the phone app's plus one
# component fails to sign against the app's profile, which is a signing error at
# install rather than anything a build reports.
#
# GENERATE_INFOPLIST_FILE = YES *and* INFOPLIST_FILE, which is not a
# contradiction: Xcode merges the synthesized keys into the named file rather
# than replacing it. So almost every key in the built bundle comes from a build
# setting, and FarCoolerWatch/Info.plist carries exactly one that could not.
#
# That one is FarCoolerAppGroup, and the reason is worth stating because it
# contradicts what a spike concluded. `INFOPLIST_KEY_<key>` is NOT a general
# mechanism for putting arbitrary keys in a synthesized Info.plist. Xcode
# honors it only for keys it already knows about; a custom key is dropped with
# no warning and no error. Measured on the built bundle:
# INFOPLIST_KEY_WKCompanionAppBundleIdentifier arrived, and an
# INFOPLIST_KEY_FarCoolerAppGroup set right beside it did not — the synthesized
# plist did not contain the key at all. `SnapshotStore.groupIdentifier` reads
# that key by name and returns nil when it is absent, which makes
# `SnapshotStore.read()` answer "nothing known" — the same answer it gives when
# no snapshot has ever arrived. A watch that is permanently empty while the
# phone sends perfectly well, with nothing in any log telling the two apart.
#
# INFOPLIST_KEY_WKCompanionAppBundleIdentifier IS one of the keys Xcode knows,
# and it is what makes this a companion of THIS app rather than a standalone
# watch app. Wrong or absent, the watch app installs as its own thing and never
# pairs with the phone app it shipped inside. Xcode synthesizes
# WKApplication = YES itself from the watchOS application product type — do not
# add it here.
#
# FARCOOLER_APP_GROUP is still a build setting because it is what the
# entitlement and the Info.plist key BOTH expand; the group the code asks for
# and the group the signature grants are then one string by construction. Note
# that the container it resolves to is the WATCH's own — a watch and a phone
# never share a container, only the identifier that names one on each device.
#
# INFOPLIST_KEY_CFBundleDisplayName, because GENERATE_INFOPLIST_FILE otherwise
# falls back to PRODUCT_NAME and the watch's home screen would read
# "FarCoolerWatch" while the phone beside it reads "FC Canary". That exact
# mismatch already happened once with the Live Activity's hardcoded name; see
# ACTIVITY_COMMON.
#
# CODE_SIGN_ENTITLEMENTS: an App Group is granted by an entitlement, not by
# naming it. Without this file `FileManager.containerURL(for…)` returns nil on a
# signed build, `SnapshotStore.read()` answers "nothing known", and the watch is
# permanently empty with nothing anywhere saying why.
WATCH_COMMON = f"""\t\t\t\tMARKETING_VERSION = {version("marketing")};
\t\t\t\tCURRENT_PROJECT_VERSION = {version("build")};
\t\t\t\tFARCOOLER_APP_NAME = "{version("app-name-short")}";
\t\t\t\tFARCOOLER_APP_GROUP = {APP_GROUP};
\t\t\t\tPRODUCT_NAME = FarCoolerWatch;
\t\t\t\tPRODUCT_BUNDLE_IDENTIFIER = {BUNDLE_ID}.watchkitapp;
\t\t\t\tSDKROOT = watchos;
\t\t\t\tWATCHOS_DEPLOYMENT_TARGET = 26.0;
\t\t\t\tTARGETED_DEVICE_FAMILY = 4;
\t\t\t\tGENERATE_INFOPLIST_FILE = YES;
\t\t\t\tINFOPLIST_FILE = FarCoolerWatch/Info.plist;
\t\t\t\tINFOPLIST_KEY_WKCompanionAppBundleIdentifier = {BUNDLE_ID};
\t\t\t\tINFOPLIST_KEY_CFBundleDisplayName = "{version("app-name-short")}";
\t\t\t\tCODE_SIGN_ENTITLEMENTS = FarCoolerWatch/FarCoolerWatch.entitlements;
\t\t\t\tCODE_SIGN_STYLE = Manual;
\t\t\t\tCODE_SIGN_IDENTITY = "-";
\t\t\t\tCODE_SIGNING_ALLOWED = YES;
\t\t\t\tCODE_SIGNING_REQUIRED = NO;
\t\t\t\tASSETCATALOG_COMPILER_APPICON_NAME = AppIcon;
\t\t\t\tSKIP_INSTALL = YES;"""

# The complication's settings: WATCH_COMMON's platform overrides and
# ACTIVITY_COMMON's extension rules, and sharing neither, because it is the only
# target in the project that needs both and folding it into either would make
# the next difference between them invisible.
#
# From the watch side: SDKROOT / WATCHOS_DEPLOYMENT_TARGET /
# TARGETED_DEVICE_FAMILY = 4. The project's COMMON block fixes `iphoneos` and
# `"1,2"` for everything, a target-level setting wins, and without these three
# this compiles for the phone and produces a bundle no watch can load.
#
# From the extension side: SKIP_INSTALL, so the .appex is not also copied to the
# archive root and the upload rejected for a top-level product that is not an
# application. And the nested identifier, which nests TWICE here —
# `…watchkitapp.widgets`. The rule is one appended component per containment,
# and the containment really is two deep: this extension is inside the watch
# app, which is inside the phone app. A name that does not nest fails to sign
# against the profile, at install time, with nothing at build time to say so.
#
# No FARCOOLER_URL_SCHEME. The complication sets no `widgetURL`: watchOS opens
# the containing app on a tap, and the watch app registers no scheme to be
# opened WITH — so a URL here would be a link that resolves to nothing. See
# `WatchFleetWidget` for what that costs and what it would take to have.
#
# GENERATE_INFOPLIST_FILE = YES *and* INFOPLIST_FILE, which is not a
# contradiction — Xcode merges the synthesized keys into the named file rather
# than replacing it. FarCoolerWatchWidgets/Info.plist therefore carries only the
# two keys Xcode does not synthesize: NSExtension, which is what makes this a
# WidgetKit extension at all, and FarCoolerAppGroup.
#
# FarCoolerAppGroup is in that file and NOT set as
# INFOPLIST_KEY_FarCoolerAppGroup, and the reason is measured rather than
# guessed: `INFOPLIST_KEY_<key>` is not a general mechanism. Xcode honors it
# only for keys it already knows and drops a custom one with no warning and no
# error. It is worse here than for the watch app, where the same thing was
# found: `Bundle.main` inside an .appex is the EXTENSION's bundle, so this
# process cannot fall back on the watch app's merged plist even in principle.
# Missing, `SnapshotStore.groupIdentifier` is nil, `SnapshotStore.read()`
# answers "nothing known" — the same answer as "no snapshot has ever arrived" —
# and the complication is permanently empty on a watch receiving the fleet
# perfectly well.
#
# INFOPLIST_KEY_CFBundleDisplayName IS one of the keys Xcode knows, and this
# target needs it because `WatchFleetWidget.appName` reads CFBundleDisplayName
# for the one sentence that has to name the app. Without it the synthesized
# value falls back to PRODUCT_NAME and a canary's complication would say
# "Open FarCoolerWatchWidgets".
#
# CODE_SIGN_ENTITLEMENTS: an extension that requests no entitlements gets none.
# The watch app's App Group grant does not cover the .appex inside it, and
# naming a group is not being granted one.
WATCH_WIDGET_COMMON = f"""\t\t\t\tMARKETING_VERSION = {version("marketing")};
\t\t\t\tCURRENT_PROJECT_VERSION = {version("build")};
\t\t\t\tFARCOOLER_APP_NAME = "{version("app-name-short")}";
\t\t\t\tFARCOOLER_APP_GROUP = {APP_GROUP};
\t\t\t\tPRODUCT_NAME = FarCoolerWatchWidgets;
\t\t\t\tPRODUCT_BUNDLE_IDENTIFIER = {BUNDLE_ID}.watchkitapp.widgets;
\t\t\t\tSDKROOT = watchos;
\t\t\t\tWATCHOS_DEPLOYMENT_TARGET = 26.0;
\t\t\t\tTARGETED_DEVICE_FAMILY = 4;
\t\t\t\tGENERATE_INFOPLIST_FILE = YES;
\t\t\t\tINFOPLIST_FILE = FarCoolerWatchWidgets/Info.plist;
\t\t\t\tINFOPLIST_KEY_CFBundleDisplayName = "{version("app-name-short")}";
\t\t\t\tCODE_SIGN_ENTITLEMENTS = FarCoolerWatchWidgets/FarCoolerWatchWidgets.entitlements;
\t\t\t\tCODE_SIGN_STYLE = Manual;
\t\t\t\tCODE_SIGN_IDENTITY = "-";
\t\t\t\tCODE_SIGNING_ALLOWED = YES;
\t\t\t\tCODE_SIGNING_REQUIRED = NO;
\t\t\t\tSKIP_INSTALL = YES;"""

UI_TEST_COMMON = f"""\t\t\t\tPRODUCT_NAME = "$(TARGET_NAME)";
\t\t\t\tPRODUCT_BUNDLE_IDENTIFIER = {BUNDLE_ID}.uitests;
\t\t\t\tGENERATE_INFOPLIST_FILE = YES;
\t\t\t\tTEST_TARGET_NAME = FarCooler;
\t\t\t\tLD_RUNPATH_SEARCH_PATHS = "$(inherited) @executable_path/Frameworks @loader_path/Frameworks";
\t\t\t\tCODE_SIGN_STYLE = Manual;
\t\t\t\tCODE_SIGN_IDENTITY = "-";
\t\t\t\tCODE_SIGNING_ALLOWED = YES;
\t\t\t\tCODE_SIGNING_REQUIRED = NO;
\t\t\t\tSKIP_INSTALL = YES;"""

PBXPROJ = f"""// !$*UTF8*$!
{{
\tarchiveVersion = 1;
\tclasses = {{}};
\tobjectVersion = 56;
\tobjects = {{

/* Begin PBXBuildFile section */
{build_files()}
/* End PBXBuildFile section */

/* Begin PBXFileReference section */
{file_refs()}
/* End PBXFileReference section */

/* Begin PBXFrameworksBuildPhase section */
\t\t{P['frameworksPhase']} = {{
\t\t\tisa = PBXFrameworksBuildPhase;
\t\t\tbuildActionMask = 2147483647;
\t\t\tfiles = (
{framework_list}
\t\t\t);
\t\t\trunOnlyForDeploymentPostprocessing = 0;
\t\t}};
\t\t{P['uiTestFrameworksPhase']} = {{
\t\t\tisa = PBXFrameworksBuildPhase;
\t\t\tbuildActionMask = 2147483647;
\t\t\tfiles = ();
\t\t\trunOnlyForDeploymentPostprocessing = 0;
\t\t}};
/* End PBXFrameworksBuildPhase section */

/* Begin PBXGroup section */
\t\t{P['mainGroup']} = {{
\t\t\tisa = PBXGroup;
\t\t\tchildren = (
\t\t\t\t{P['sourcesGroup']} /* FarCooler */,
\t\t\t\t{P['activityGroup']} /* FarCoolerActivity */,
\t\t\t\t{P['notifyGroup']} /* FarCoolerNotify */,
\t\t\t\t{P['watchGroup']} /* FarCoolerWatch */,
\t\t\t\t{P['watchWidgetGroup']} /* FarCoolerWatchWidgets */,
\t\t\t\t{P['uiTestGroup']} /* FarCoolerUITests */,
\t\t\t\t{P['agentKitGroup']} /* AgentKit */,
\t\t\t\t{ids[ASSET_CATALOG]} /* {ASSET_CATALOG} */,
\t\t\t\t{P['frameworksGroup']} /* Frameworks */,
\t\t\t\t{P['productsGroup']} /* Products */,
\t\t\t);
\t\t\tsourceTree = "<group>";
\t\t}};
\t\t{P['sourcesGroup']} /* FarCooler */ = {{
\t\t\tisa = PBXGroup;
\t\t\tchildren = (
{source_children}
\t\t\t\t{P['ceremonyGroup']} /* Ceremony */,
\t\t\t\t{P['fontsGroup']} /* Fonts */,
\t\t\t);
\t\t\tpath = FarCooler;
\t\t\tsourceTree = "<group>";
\t\t}};
\t\t{P['activityGroup']} /* FarCoolerActivity */ = {{
\t\t\tisa = PBXGroup;
\t\t\tchildren = (
{activity_children}
\t\t\t);
\t\t\tpath = FarCoolerActivity;
\t\t\tsourceTree = "<group>";
\t\t}};
\t\t{P['notifyGroup']} /* FarCoolerNotify */ = {{
\t\t\tisa = PBXGroup;
\t\t\tchildren = (
{notify_children}
\t\t\t);
\t\t\tpath = FarCoolerNotify;
\t\t\tsourceTree = "<group>";
\t\t}};
\t\t{P['watchGroup']} /* FarCoolerWatch */ = {{
\t\t\tisa = PBXGroup;
\t\t\tchildren = (
{watch_children}
\t\t\t);
\t\t\tpath = FarCoolerWatch;
\t\t\tsourceTree = "<group>";
\t\t}};
\t\t{P['watchWidgetGroup']} /* FarCoolerWatchWidgets */ = {{
\t\t\tisa = PBXGroup;
\t\t\tchildren = (
{watch_widget_children}
\t\t\t);
\t\t\tpath = FarCoolerWatchWidgets;
\t\t\tsourceTree = "<group>";
\t\t}};
\t\t{P['uiTestGroup']} /* FarCoolerUITests */ = {{
\t\t\tisa = PBXGroup;
\t\t\tchildren = (
{ui_test_children}
\t\t\t);
\t\t\tpath = FarCoolerUITests;
\t\t\tsourceTree = "<group>";
\t\t}};
\t\t{P['ceremonyGroup']} /* Ceremony */ = {{
\t\t\tisa = PBXGroup;
\t\t\tchildren = (
{ceremony_children}
\t\t\t);
\t\t\tpath = Ceremony;
\t\t\tsourceTree = "<group>";
\t\t}};
\t\t{P['fontsGroup']} /* Fonts */ = {{
\t\t\tisa = PBXGroup;
\t\t\tchildren = (
{font_children}
\t\t\t);
\t\t\tpath = Fonts;
\t\t\tsourceTree = "<group>";
\t\t}};
\t\t{P['agentKitGroup']} /* AgentKit */ = {{
\t\t\tisa = PBXGroup;
\t\t\tchildren = (
{agentkit_children}
\t\t\t);
\t\t\tname = AgentKit;
\t\t\tpath = "../shared/AgentKit/Sources/AgentKit";
\t\t\tsourceTree = "<group>";
\t\t}};
\t\t{P['frameworksGroup']} /* Frameworks */ = {{
\t\t\tisa = PBXGroup;
\t\t\tchildren = (
{framework_children}
\t\t\t);
\t\t\tname = Frameworks;
\t\t\tsourceTree = "<group>";
\t\t}};
\t\t{P['productsGroup']} /* Products */ = {{
\t\t\tisa = PBXGroup;
\t\t\tchildren = (
\t\t\t\t{P['product']} /* FarCooler.app */,
\t\t\t\t{P['activityProduct']} /* FarCoolerActivity.appex */,
\t\t\t\t{P['notifyProduct']} /* FarCoolerNotify.appex */,
\t\t\t\t{P['watchProduct']} /* FarCoolerWatch.app */,
\t\t\t\t{P['watchWidgetProduct']} /* FarCoolerWatchWidgets.appex */,
\t\t\t\t{P['uiTestProduct']} /* FarCoolerUITests.xctest */,
\t\t\t);
\t\t\tname = Products;
\t\t\tsourceTree = "<group>";
\t\t}};
/* End PBXGroup section */

/* Begin PBXNativeTarget section */
\t\t{P['target']} /* FarCooler */ = {{
\t\t\tisa = PBXNativeTarget;
\t\t\tbuildConfigurationList = {P['targetConfigList']};
\t\t\tbuildPhases = (
\t\t\t\t{P['sourcesPhase']},
\t\t\t\t{P['frameworksPhase']},
\t\t\t\t{P['resourcesPhase']},
\t\t\t\t{P['embedPhase']},
\t\t\t\t{P['watchEmbedPhase']},
\t\t\t);
\t\t\tbuildRules = ();
\t\t\tdependencies = (
\t\t\t\t{P['activityDependency']},
\t\t\t\t{P['notifyDependency']},
\t\t\t\t{P['watchDependency']},
\t\t\t);
\t\t\tname = FarCooler;
\t\t\tproductName = FarCooler;
\t\t\tproductReference = {P['product']};
\t\t\tproductType = "com.apple.product-type.application";
\t\t}};
\t\t{P['activityTarget']} /* FarCoolerActivity */ = {{
\t\t\tisa = PBXNativeTarget;
\t\t\tbuildConfigurationList = {P['activityConfigList']};
\t\t\tbuildPhases = (
\t\t\t\t{P['activitySourcesPhase']},
\t\t\t);
\t\t\tbuildRules = ();
\t\t\tdependencies = ();
\t\t\tname = FarCoolerActivity;
\t\t\tproductName = FarCoolerActivity;
\t\t\tproductReference = {P['activityProduct']};
\t\t\tproductType = "com.apple.product-type.app-extension";
\t\t}};
\t\t{P['notifyTarget']} /* FarCoolerNotify */ = {{
\t\t\tisa = PBXNativeTarget;
\t\t\tbuildConfigurationList = {P['notifyConfigList']};
\t\t\tbuildPhases = (
\t\t\t\t{P['notifySourcesPhase']},
\t\t\t);
\t\t\tbuildRules = ();
\t\t\tdependencies = ();
\t\t\tname = FarCoolerNotify;
\t\t\tproductName = FarCoolerNotify;
\t\t\tproductReference = {P['notifyProduct']};
\t\t\tproductType = "com.apple.product-type.app-extension";
\t\t}};
\t\t{P['watchTarget']} /* FarCoolerWatch */ = {{
\t\t\tisa = PBXNativeTarget;
\t\t\tbuildConfigurationList = {P['watchConfigList']};
\t\t\tbuildPhases = (
\t\t\t\t{P['watchSourcesPhase']},
\t\t\t\t{P['watchResourcesPhase']},
\t\t\t\t{P['watchWidgetEmbedPhase']},
\t\t\t);
\t\t\tbuildRules = ();
\t\t\tdependencies = (
\t\t\t\t{P['watchWidgetDependency']},
\t\t\t);
\t\t\tname = FarCoolerWatch;
\t\t\tproductName = FarCoolerWatch;
\t\t\tproductReference = {P['watchProduct']};
\t\t\tproductType = "com.apple.product-type.application";
\t\t}};
\t\t{P['watchWidgetTarget']} /* FarCoolerWatchWidgets */ = {{
\t\t\tisa = PBXNativeTarget;
\t\t\tbuildConfigurationList = {P['watchWidgetConfigList']};
\t\t\tbuildPhases = (
\t\t\t\t{P['watchWidgetSourcesPhase']},
\t\t\t);
\t\t\tbuildRules = ();
\t\t\tdependencies = ();
\t\t\tname = FarCoolerWatchWidgets;
\t\t\tproductName = FarCoolerWatchWidgets;
\t\t\tproductReference = {P['watchWidgetProduct']};
\t\t\tproductType = "com.apple.product-type.app-extension";
\t\t}};
\t\t{P['uiTestTarget']} /* FarCoolerUITests */ = {{
\t\t\tisa = PBXNativeTarget;
\t\t\tbuildConfigurationList = {P['uiTestConfigList']};
\t\t\tbuildPhases = (
\t\t\t\t{P['uiTestSourcesPhase']},
\t\t\t\t{P['uiTestFrameworksPhase']},
\t\t\t);
\t\t\tbuildRules = ();
\t\t\tdependencies = ({P['uiTestDependency']}, );
\t\t\tname = FarCoolerUITests;
\t\t\tproductName = FarCoolerUITests;
\t\t\tproductReference = {P['uiTestProduct']};
\t\t\tproductType = "com.apple.product-type.bundle.ui-testing";
\t\t}};
/* End PBXNativeTarget section */

/* Begin PBXProject section */
\t\t{P['project']} /* Project object */ = {{
\t\t\tisa = PBXProject;
\t\t\tattributes = {{
\t\t\t\tBuildIndependentTargetsInParallel = 1;
\t\t\t\tLastSwiftUpdateCheck = 1600;
\t\t\t\tLastUpgradeCheck = 1600;
\t\t\t}};
\t\t\tbuildConfigurationList = {P['buildConfigList']};
\t\t\tcompatibilityVersion = "Xcode 14.0";
\t\t\tdevelopmentRegion = en;
\t\t\thasScannedForEncodings = 0;
\t\t\tknownRegions = (en, Base);
\t\t\tmainGroup = {P['mainGroup']};
\t\t\tproductRefGroup = {P['productsGroup']};
\t\t\tprojectDirPath = "";
\t\t\tprojectRoot = "";
\t\t\ttargets = (
\t\t\t\t{P['target']} /* FarCooler */,
\t\t\t\t{P['activityTarget']} /* FarCoolerActivity */,
\t\t\t\t{P['notifyTarget']} /* FarCoolerNotify */,
\t\t\t\t{P['watchTarget']} /* FarCoolerWatch */,
\t\t\t\t{P['watchWidgetTarget']} /* FarCoolerWatchWidgets */,
\t\t\t\t{P['uiTestTarget']} /* FarCoolerUITests */,
\t\t\t);
\t\t}};
/* End PBXProject section */

/* Begin PBXContainerItemProxy section */
\t\t{P['activityProxy']} = {{
\t\t\tisa = PBXContainerItemProxy;
\t\t\tcontainerPortal = {P['project']};
\t\t\tproxyType = 1;
\t\t\tremoteGlobalIDString = {P['activityTarget']};
\t\t\tremoteInfo = FarCoolerActivity;
\t\t}};
\t\t{P['notifyProxy']} = {{
\t\t\tisa = PBXContainerItemProxy;
\t\t\tcontainerPortal = {P['project']};
\t\t\tproxyType = 1;
\t\t\tremoteGlobalIDString = {P['notifyTarget']};
\t\t\tremoteInfo = FarCoolerNotify;
\t\t}};
\t\t{P['watchProxy']} = {{
\t\t\tisa = PBXContainerItemProxy;
\t\t\tcontainerPortal = {P['project']};
\t\t\tproxyType = 1;
\t\t\tremoteGlobalIDString = {P['watchTarget']};
\t\t\tremoteInfo = FarCoolerWatch;
\t\t}};
\t\t{P['watchWidgetProxy']} = {{
\t\t\tisa = PBXContainerItemProxy;
\t\t\tcontainerPortal = {P['project']};
\t\t\tproxyType = 1;
\t\t\tremoteGlobalIDString = {P['watchWidgetTarget']};
\t\t\tremoteInfo = FarCoolerWatchWidgets;
\t\t}};
\t\t{P['uiTestProxy']} = {{
\t\t\tisa = PBXContainerItemProxy;
\t\t\tcontainerPortal = {P['project']};
\t\t\tproxyType = 1;
\t\t\tremoteGlobalIDString = {P['target']};
\t\t\tremoteInfo = FarCooler;
\t\t}};
/* End PBXContainerItemProxy section */

/* Begin PBXCopyFilesBuildPhase section */
\t\t{P['embedPhase']} /* Embed Foundation Extensions */ = {{
\t\t\tisa = PBXCopyFilesBuildPhase;
\t\t\tbuildActionMask = 2147483647;
\t\t\tdstPath = "";
\t\t\tdstSubfolderSpec = 13;
\t\t\tfiles = (
\t\t\t\t{EMBED_BUILD_ID} /* FarCoolerActivity.appex in Embed Foundation Extensions */,
\t\t\t\t{EMBED_NOTIFY_BUILD_ID} /* FarCoolerNotify.appex in Embed Foundation Extensions */,
\t\t\t);
\t\t\tname = "Embed Foundation Extensions";
\t\t\trunOnlyForDeploymentPostprocessing = 0;
\t\t}};
\t\t{P['watchEmbedPhase']} /* Embed Watch Content */ = {{
\t\t\tisa = PBXCopyFilesBuildPhase;
\t\t\tbuildActionMask = 2147483647;
\t\t\tdstPath = "$(CONTENTS_FOLDER_PATH)/Watch";
\t\t\tdstSubfolderSpec = 16;
\t\t\tfiles = (
\t\t\t\t{EMBED_WATCH_BUILD_ID} /* FarCoolerWatch.app in Embed Watch Content */,
\t\t\t);
\t\t\tname = "Embed Watch Content";
\t\t\trunOnlyForDeploymentPostprocessing = 0;
\t\t}};
\t\t{P['watchWidgetEmbedPhase']} /* Embed Watch Extensions */ = {{
\t\t\tisa = PBXCopyFilesBuildPhase;
\t\t\tbuildActionMask = 2147483647;
\t\t\tdstPath = "";
\t\t\tdstSubfolderSpec = 13;
\t\t\tfiles = (
\t\t\t\t{EMBED_WATCH_WIDGET_BUILD_ID} /* FarCoolerWatchWidgets.appex in Embed Watch Extensions */,
\t\t\t);
\t\t\tname = "Embed Watch Extensions";
\t\t\trunOnlyForDeploymentPostprocessing = 0;
\t\t}};
/* End PBXCopyFilesBuildPhase section */

/* Begin PBXTargetDependency section */
\t\t{P['activityDependency']} = {{
\t\t\tisa = PBXTargetDependency;
\t\t\ttarget = {P['activityTarget']};
\t\t\ttargetProxy = {P['activityProxy']};
\t\t}};
\t\t{P['notifyDependency']} = {{
\t\t\tisa = PBXTargetDependency;
\t\t\ttarget = {P['notifyTarget']};
\t\t\ttargetProxy = {P['notifyProxy']};
\t\t}};
\t\t{P['watchDependency']} = {{
\t\t\tisa = PBXTargetDependency;
\t\t\ttarget = {P['watchTarget']};
\t\t\ttargetProxy = {P['watchProxy']};
\t\t}};
\t\t{P['watchWidgetDependency']} = {{
\t\t\tisa = PBXTargetDependency;
\t\t\ttarget = {P['watchWidgetTarget']};
\t\t\ttargetProxy = {P['watchWidgetProxy']};
\t\t}};
\t\t{P['uiTestDependency']} = {{
\t\t\tisa = PBXTargetDependency;
\t\t\ttarget = {P['target']};
\t\t\ttargetProxy = {P['uiTestProxy']};
\t\t}};
/* End PBXTargetDependency section */

/* Begin PBXResourcesBuildPhase section */
\t\t{P['resourcesPhase']} = {{
\t\t\tisa = PBXResourcesBuildPhase;
\t\t\tbuildActionMask = 2147483647;
\t\t\tfiles = (
{resource_list}
\t\t\t);
\t\t\trunOnlyForDeploymentPostprocessing = 0;
\t\t}};
\t\t{P['watchResourcesPhase']} = {{
\t\t\tisa = PBXResourcesBuildPhase;
\t\t\tbuildActionMask = 2147483647;
\t\t\tfiles = (
\t\t\t\t{watch_asset_build_id} /* {ASSET_CATALOG} in Resources */,
\t\t\t);
\t\t\trunOnlyForDeploymentPostprocessing = 0;
\t\t}};
/* End PBXResourcesBuildPhase section */

/* Begin PBXSourcesBuildPhase section */
\t\t{P['sourcesPhase']} = {{
\t\t\tisa = PBXSourcesBuildPhase;
\t\t\tbuildActionMask = 2147483647;
\t\t\tfiles = (
{source_list}
\t\t\t);
\t\t\trunOnlyForDeploymentPostprocessing = 0;
\t\t}};
\t\t{P['activitySourcesPhase']} = {{
\t\t\tisa = PBXSourcesBuildPhase;
\t\t\tbuildActionMask = 2147483647;
\t\t\tfiles = (
{activity_source_list}
\t\t\t);
\t\t\trunOnlyForDeploymentPostprocessing = 0;
\t\t}};
\t\t{P['notifySourcesPhase']} = {{
\t\t\tisa = PBXSourcesBuildPhase;
\t\t\tbuildActionMask = 2147483647;
\t\t\tfiles = (
{notify_source_list}
\t\t\t);
\t\t\trunOnlyForDeploymentPostprocessing = 0;
\t\t}};
\t\t{P['watchSourcesPhase']} = {{
\t\t\tisa = PBXSourcesBuildPhase;
\t\t\tbuildActionMask = 2147483647;
\t\t\tfiles = (
{watch_source_list}
\t\t\t);
\t\t\trunOnlyForDeploymentPostprocessing = 0;
\t\t}};
\t\t{P['watchWidgetSourcesPhase']} = {{
\t\t\tisa = PBXSourcesBuildPhase;
\t\t\tbuildActionMask = 2147483647;
\t\t\tfiles = (
{watch_widget_source_list}
\t\t\t);
\t\t\trunOnlyForDeploymentPostprocessing = 0;
\t\t}};
\t\t{P['uiTestSourcesPhase']} = {{
\t\t\tisa = PBXSourcesBuildPhase;
\t\t\tbuildActionMask = 2147483647;
\t\t\tfiles = (
{ui_test_source_list}
\t\t\t);
\t\t\trunOnlyForDeploymentPostprocessing = 0;
\t\t}};
/* End PBXSourcesBuildPhase section */

/* Begin XCBuildConfiguration section */
\t\t{P['debug']} /* Debug */ = {{
\t\t\tisa = XCBuildConfiguration;
\t\t\tbuildSettings = {{
{COMMON}
\t\t\t\tONLY_ACTIVE_ARCH = YES;
\t\t\t\tSWIFT_OPTIMIZATION_LEVEL = "-Onone";
\t\t\t\tSWIFT_ACTIVE_COMPILATION_CONDITIONS = DEBUG;
\t\t\t\tGCC_OPTIMIZATION_LEVEL = 0;
\t\t\t\tENABLE_TESTABILITY = YES;
\t\t\t}};
\t\t\tname = Debug;
\t\t}};
\t\t{P['release']} /* Release */ = {{
\t\t\tisa = XCBuildConfiguration;
\t\t\tbuildSettings = {{
{COMMON}
\t\t\t\tSWIFT_OPTIMIZATION_LEVEL = "-O";
\t\t\t}};
\t\t\tname = Release;
\t\t}};
\t\t{P['targetDebug']} /* Debug */ = {{
\t\t\tisa = XCBuildConfiguration;
\t\t\tbuildSettings = {{
{TARGET_COMMON}
\t\t\t}};
\t\t\tname = Debug;
\t\t}};
\t\t{P['targetRelease']} /* Release */ = {{
\t\t\tisa = XCBuildConfiguration;
\t\t\tbuildSettings = {{
{TARGET_COMMON}
\t\t\t}};
\t\t\tname = Release;
\t\t}};
\t\t{P['activityDebug']} /* Debug */ = {{
\t\t\tisa = XCBuildConfiguration;
\t\t\tbuildSettings = {{
{ACTIVITY_COMMON}
\t\t\t}};
\t\t\tname = Debug;
\t\t}};
\t\t{P['activityRelease']} /* Release */ = {{
\t\t\tisa = XCBuildConfiguration;
\t\t\tbuildSettings = {{
{ACTIVITY_COMMON}
\t\t\t}};
\t\t\tname = Release;
\t\t}};
\t\t{P['notifyDebug']} /* Debug */ = {{
\t\t\tisa = XCBuildConfiguration;
\t\t\tbuildSettings = {{
{NOTIFY_COMMON}
\t\t\t}};
\t\t\tname = Debug;
\t\t}};
\t\t{P['notifyRelease']} /* Release */ = {{
\t\t\tisa = XCBuildConfiguration;
\t\t\tbuildSettings = {{
{NOTIFY_COMMON}
\t\t\t}};
\t\t\tname = Release;
\t\t}};
\t\t{P['watchDebug']} /* Debug */ = {{
\t\t\tisa = XCBuildConfiguration;
\t\t\tbuildSettings = {{
{WATCH_COMMON}
\t\t\t}};
\t\t\tname = Debug;
\t\t}};
\t\t{P['watchRelease']} /* Release */ = {{
\t\t\tisa = XCBuildConfiguration;
\t\t\tbuildSettings = {{
{WATCH_COMMON}
\t\t\t}};
\t\t\tname = Release;
\t\t}};
\t\t{P['watchWidgetDebug']} /* Debug */ = {{
\t\t\tisa = XCBuildConfiguration;
\t\t\tbuildSettings = {{
{WATCH_WIDGET_COMMON}
\t\t\t}};
\t\t\tname = Debug;
\t\t}};
\t\t{P['watchWidgetRelease']} /* Release */ = {{
\t\t\tisa = XCBuildConfiguration;
\t\t\tbuildSettings = {{
{WATCH_WIDGET_COMMON}
\t\t\t}};
\t\t\tname = Release;
\t\t}};
\t\t{P['uiTestDebug']} /* Debug */ = {{
\t\t\tisa = XCBuildConfiguration;
\t\t\tbuildSettings = {{
{UI_TEST_COMMON}
\t\t\t}};
\t\t\tname = Debug;
\t\t}};
\t\t{P['uiTestRelease']} /* Release */ = {{
\t\t\tisa = XCBuildConfiguration;
\t\t\tbuildSettings = {{
{UI_TEST_COMMON}
\t\t\t}};
\t\t\tname = Release;
\t\t}};
/* End XCBuildConfiguration section */

/* Begin XCConfigurationList section */
\t\t{P['buildConfigList']} = {{
\t\t\tisa = XCConfigurationList;
\t\t\tbuildConfigurations = (
\t\t\t\t{P['debug']} /* Debug */,
\t\t\t\t{P['release']} /* Release */,
\t\t\t);
\t\t\tdefaultConfigurationIsVisible = 0;
\t\t\tdefaultConfigurationName = Release;
\t\t}};
\t\t{P['targetConfigList']} = {{
\t\t\tisa = XCConfigurationList;
\t\t\tbuildConfigurations = (
\t\t\t\t{P['targetDebug']} /* Debug */,
\t\t\t\t{P['targetRelease']} /* Release */,
\t\t\t);
\t\t\tdefaultConfigurationIsVisible = 0;
\t\t\tdefaultConfigurationName = Release;
\t\t}};
\t\t{P['activityConfigList']} = {{
\t\t\tisa = XCConfigurationList;
\t\t\tbuildConfigurations = (
\t\t\t\t{P['activityDebug']} /* Debug */,
\t\t\t\t{P['activityRelease']} /* Release */,
\t\t\t);
\t\t\tdefaultConfigurationIsVisible = 0;
\t\t\tdefaultConfigurationName = Release;
\t\t}};
\t\t{P['notifyConfigList']} = {{
\t\t\tisa = XCConfigurationList;
\t\t\tbuildConfigurations = (
\t\t\t\t{P['notifyDebug']} /* Debug */,
\t\t\t\t{P['notifyRelease']} /* Release */,
\t\t\t);
\t\t\tdefaultConfigurationIsVisible = 0;
\t\t\tdefaultConfigurationName = Release;
\t\t}};
\t\t{P['watchConfigList']} = {{
\t\t\tisa = XCConfigurationList;
\t\t\tbuildConfigurations = (
\t\t\t\t{P['watchDebug']} /* Debug */,
\t\t\t\t{P['watchRelease']} /* Release */,
\t\t\t);
\t\t\tdefaultConfigurationIsVisible = 0;
\t\t\tdefaultConfigurationName = Release;
\t\t}};
\t\t{P['watchWidgetConfigList']} = {{
\t\t\tisa = XCConfigurationList;
\t\t\tbuildConfigurations = (
\t\t\t\t{P['watchWidgetDebug']} /* Debug */,
\t\t\t\t{P['watchWidgetRelease']} /* Release */,
\t\t\t);
\t\t\tdefaultConfigurationIsVisible = 0;
\t\t\tdefaultConfigurationName = Release;
\t\t}};
\t\t{P['uiTestConfigList']} = {{
\t\t\tisa = XCConfigurationList;
\t\t\tbuildConfigurations = (
\t\t\t\t{P['uiTestDebug']} /* Debug */,
\t\t\t\t{P['uiTestRelease']} /* Release */,
\t\t\t);
\t\t\tdefaultConfigurationIsVisible = 0;
\t\t\tdefaultConfigurationName = Release;
\t\t}};
/* End XCConfigurationList section */
\t}};
\trootObject = {P['project']} /* Project object */;
}}
"""

here = pathlib.Path(__file__).parent

# The frameworks this project links are BUILT from this workspace, and staleness
# in them is invisible.
#
# `apps/macos/build-app.sh` used to copy whatever binary happened to be sitting
# in `target/release`, which meant a Rust fix that had been compiled and tested
# shipped as the binary from whenever someone last built by hand — the bug under
# investigation went on reproducing with the fix nowhere in the app. The phone
# has exactly the same hole and it is worse: an `.xcframework` is a directory of
# static archives that Xcode links happily whatever its age, so a change to
# `crates/client` simply does not reach the app and nothing says so.
#
# Warned about rather than rebuilt: the framework build is slow and cross-
# compiles two targets, so making every project generation pay for it would be
# its own kind of wrong. Naming the file that is newer is enough to act on.
frameworks = here / "Frameworks"
if frameworks.is_dir():
    newest_framework = max(
        (p.stat().st_mtime for p in frameworks.rglob("*") if p.is_file()), default=0
    )
    # Only the crates that are actually compiled INTO the frameworks. The
    # daemon and the CLI are binaries the phone talks to over ssh, never links,
    # so naming one of their files here would send someone to rebuild for a
    # change that could not have reached the app.
    rust = here.parent.parent / "crates"
    linked = {"client", "vt", "protocol", "core", "transport", "agent", "store", "tmux"}
    stale = [
        p
        for p in rust.rglob("*.rs")
        if p.stat().st_mtime > newest_framework
        and "target" not in p.parts
        and p.relative_to(rust).parts[0] in linked
    ]
    if stale:
        print(
            f"WARNING: {len(stale)} Rust source files are newer than "
            f"Frameworks/ (e.g. {stale[0].relative_to(here.parent.parent)}).\n"
            "         The app will link a STALE client core. Rebuild first:\n"
            "           ./scripts/build-ios-frameworks.sh"
        )

project = here / "FarCooler.xcodeproj"
project.mkdir(exist_ok=True)
(project / "project.pbxproj").write_text(PBXPROJ)
print(f"wrote {project / 'project.pbxproj'}")
