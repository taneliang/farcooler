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
import subprocess
import uuid

SOURCES = [
    "FarCoolerApp.swift",
    "ClientCore.swift",
    "Connection.swift",
    "FleetView.swift",
    "Model.swift",
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
    "TerminalTabStrip.swift",
    "TerminalView.swift",
    "ImagePaste.swift",
    "AgentStream.swift",
    "AgentView.swift",
    "MachineSettings.swift",
    "ThemeEditorView.swift",
    "AdapterEditorView.swift",
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
    "Composer.swift",
    "DiffComputation.swift",
    # The markdown renderer, shared for the same reason the reducer is: the
    # phone drew agent replies as plain `Text`, so a table arrived as a wall of
    # pipes and a heading as a line beginning with a hash. Same conversation,
    # unreadable on one of the two clients.
    "MarkdownView.swift",
    # The half of push registration that is not platform-specific. Both apps
    # had it verbatim; only the device label differs.
    "PushRegistration.swift",
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
ACTIVITY_SOURCES = [
    "FarCoolerActivityBundle.swift",
    "AgentActivityWidget.swift",
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
    "frameworksGroup", "fontsGroup", "agentKitGroup", "product", "sourcesPhase",
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
]


def oid(seed):
    """A stable 24-hex object id, so regenerating produces an identical file."""
    return uuid.uuid5(uuid.NAMESPACE_URL, "farcooler-ios/" + seed).hex[:24].upper()


ids = {
    name: oid(name)
    for name in SOURCES + AGENTKIT_SOURCES + ACTIVITY_SOURCES + FRAMEWORKS + FONTS
    + [ASSET_CATALOG]
}
build_ids = {
    name: oid("build/" + name)
    for name in SOURCES + AGENTKIT_SOURCES + ACTIVITY_SOURCES + FRAMEWORKS + FONTS
    + [ASSET_CATALOG]
}

# A PBXBuildFile is "this file, compiled into THIS target", so a file two targets
# both compile needs two of them pointing at one PBXFileReference. Reusing a
# single build id puts the same object in two sources phases, which Xcode reads
# as one target's file appearing twice and fails with "multiple commands
# produce". `AgentActivityAttributes.swift` is the whole reason this exists; the
# extension's own sources are in here too so the two lists stay symmetrical.
activity_build_ids = {
    name: oid("activity-build/" + name)
    for name in ACTIVITY_SOURCES + ["AgentActivityAttributes.swift"]
}

P = {key: oid(key) for key in KEYS}

# The built .appex, copied into the app's PlugIns folder.
#
# `RemoveHeadersOnCopy` is what Xcode's own template writes here. It is not
# optional decoration: without it the copy carries headers into the bundle and
# App Store validation rejects the upload.
EMBED_BUILD_ID = oid("build/embed-activity")


def file_refs():
    lines = []
    for name in SOURCES + AGENTKIT_SOURCES + ACTIVITY_SOURCES:
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
        f"path = ../shared/{ASSET_CATALOG}; sourceTree = SOURCE_ROOT; }};"
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
    return "\n".join(lines)


def build_files():
    lines = []
    for name in SOURCES + AGENTKIT_SOURCES:
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
    for name, build_id in activity_build_ids.items():
        lines.append(
            f"\t\t{build_id} /* {name} in Sources */ = {{isa = PBXBuildFile; "
            f"fileRef = {ids[name]}; }};"
        )
    lines.append(
        f"\t\t{EMBED_BUILD_ID} /* FarCoolerActivity.appex in Embed Foundation Extensions */ = "
        f"{{isa = PBXBuildFile; fileRef = {P['activityProduct']}; "
        "settings = {ATTRIBUTES = (RemoveHeadersOnCopy, ); }; };"
    )
    return "\n".join(lines)


source_list = "\n".join(
    f"\t\t\t\t{build_ids[n]} /* {n} in Sources */," for n in SOURCES + AGENTKIT_SOURCES
)
framework_list = "\n".join(f"\t\t\t\t{build_ids[n]} /* {n} in Frameworks */," for n in FRAMEWORKS)
resource_list = "\n".join(
    f"\t\t\t\t{build_ids[n]} /* {n} in Resources */," for n in FONTS + [ASSET_CATALOG]
)
source_children = "\n".join(f"\t\t\t\t{ids[n]} /* {n} */," for n in SOURCES)
framework_children = "\n".join(f"\t\t\t\t{ids[n]} /* {n} */," for n in FRAMEWORKS)
font_children = "\n".join(f"\t\t\t\t{ids[n]} /* {n} */," for n in FONTS)
agentkit_children = "\n".join(f"\t\t\t\t{ids[n]} /* {n} */," for n in AGENTKIT_SOURCES)
activity_children = "\n".join(f"\t\t\t\t{ids[n]} /* {n} */," for n in ACTIVITY_SOURCES)
activity_source_list = "\n".join(
    f"\t\t\t\t{activity_build_ids[n]} /* {n} in Sources */," for n in activity_build_ids
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

TARGET_COMMON = f"""\t\t\t\tMARKETING_VERSION = {version("marketing")};
\t\t\t\tFARCOOLER_CHANNEL = {version("channel")};
\t\t\t\tFARCOOLER_DISPLAY_VERSION = "{version("display")}";
\t\t\t\tFARCOOLER_WORKOS_CLIENT_ID = "{WORKOS_CLIENT_ID}";
\t\t\t\tCURRENT_PROJECT_VERSION = {version("build")};
\t\t\t\tPRODUCT_NAME = FarCooler;
\t\t\t\tPRODUCT_BUNDLE_IDENTIFIER = com.farcooler.ios;
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
# SKIP_INSTALL is required of every embedded extension. Without it the .appex is
# copied to the archive's root as well as into the app, and the upload is
# rejected for containing a top-level product that is not an application.
#
# The bundle id has to be the app's with one component appended — that is the
# rule for an app extension, not a convention, and a name that does not nest
# fails to sign against the app's profile.
ACTIVITY_COMMON = f"""\t\t\t\tMARKETING_VERSION = {version("marketing")};
\t\t\t\tCURRENT_PROJECT_VERSION = {version("build")};
\t\t\t\tPRODUCT_NAME = FarCoolerActivity;
\t\t\t\tPRODUCT_BUNDLE_IDENTIFIER = com.farcooler.ios.activity;
\t\t\t\tINFOPLIST_FILE = FarCoolerActivity/Info.plist;
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
/* End PBXFrameworksBuildPhase section */

/* Begin PBXGroup section */
\t\t{P['mainGroup']} = {{
\t\t\tisa = PBXGroup;
\t\t\tchildren = (
\t\t\t\t{P['sourcesGroup']} /* FarCooler */,
\t\t\t\t{P['activityGroup']} /* FarCoolerActivity */,
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
\t\t\t);
\t\t\tbuildRules = ();
\t\t\tdependencies = (
\t\t\t\t{P['activityDependency']},
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
/* End PBXContainerItemProxy section */

/* Begin PBXCopyFilesBuildPhase section */
\t\t{P['embedPhase']} /* Embed Foundation Extensions */ = {{
\t\t\tisa = PBXCopyFilesBuildPhase;
\t\t\tbuildActionMask = 2147483647;
\t\t\tdstPath = "";
\t\t\tdstSubfolderSpec = 13;
\t\t\tfiles = (
\t\t\t\t{EMBED_BUILD_ID} /* FarCoolerActivity.appex in Embed Foundation Extensions */,
\t\t\t);
\t\t\tname = "Embed Foundation Extensions";
\t\t\trunOnlyForDeploymentPostprocessing = 0;
\t\t}};
/* End PBXCopyFilesBuildPhase section */

/* Begin PBXTargetDependency section */
\t\t{P['activityDependency']} = {{
\t\t\tisa = PBXTargetDependency;
\t\t\ttarget = {P['activityTarget']};
\t\t\ttargetProxy = {P['activityProxy']};
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
/* End PBXSourcesBuildPhase section */

/* Begin XCBuildConfiguration section */
\t\t{P['debug']} /* Debug */ = {{
\t\t\tisa = XCBuildConfiguration;
\t\t\tbuildSettings = {{
{COMMON}
\t\t\t\tONLY_ACTIVE_ARCH = YES;
\t\t\t\tSWIFT_OPTIMIZATION_LEVEL = "-Onone";
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
