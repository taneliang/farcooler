#!/usr/bin/env python3
"""Generate the Xcode project for the iOS app.

An .xcodeproj is generated state, not source. Keeping the generator in git
rather than the 300-line plist it produces means the project can be reviewed,
diffed, and regenerated after a source file is added — and it cannot drift into
the unreadable mess that a hand-edited pbxproj becomes.

    ./apps/ios/generate-project.py
"""

import pathlib
import uuid

SOURCES = [
    "OvernightApp.swift",
    "ClientCore.swift",
    "Connection.swift",
    "FleetView.swift",
    "Model.swift",
    "Store.swift",
    "QuickTask.swift",
    "TaskComposer.swift",
    "VTCore.swift",
    "TerminalSession.swift",
    "TerminalView.swift",
]
FRAMEWORKS = ["overnight_vt.xcframework", "overnight_client.xcframework"]

KEYS = [
    "project", "target", "mainGroup", "productsGroup", "sourcesGroup",
    "frameworksGroup", "product", "sourcesPhase", "frameworksPhase",
    "resourcesPhase", "buildConfigList", "targetConfigList",
    "debug", "release", "targetDebug", "targetRelease",
]


def oid(seed):
    """A stable 24-hex object id, so regenerating produces an identical file."""
    return uuid.uuid5(uuid.NAMESPACE_URL, "overnight-ios/" + seed).hex[:24].upper()


ids = {name: oid(name) for name in SOURCES + FRAMEWORKS}
build_ids = {name: oid("build/" + name) for name in SOURCES + FRAMEWORKS}
P = {key: oid(key) for key in KEYS}


def file_refs():
    lines = []
    for name in SOURCES:
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
    lines.append(
        f"\t\t{P['product']} /* Overnight.app */ = {{isa = PBXFileReference; "
        "explicitFileType = wrapper.application; includeInIndex = 0; "
        "path = Overnight.app; sourceTree = BUILT_PRODUCTS_DIR; };"
    )
    return "\n".join(lines)


def build_files():
    lines = []
    for name in SOURCES:
        lines.append(
            f"\t\t{build_ids[name]} /* {name} in Sources */ = {{isa = PBXBuildFile; "
            f"fileRef = {ids[name]}; }};"
        )
    for name in FRAMEWORKS:
        lines.append(
            f"\t\t{build_ids[name]} /* {name} in Frameworks */ = {{isa = PBXBuildFile; "
            f"fileRef = {ids[name]}; }};"
        )
    return "\n".join(lines)


source_list = "\n".join(f"\t\t\t\t{build_ids[n]} /* {n} in Sources */," for n in SOURCES)
framework_list = "\n".join(f"\t\t\t\t{build_ids[n]} /* {n} in Frameworks */," for n in FRAMEWORKS)
source_children = "\n".join(f"\t\t\t\t{ids[n]} /* {n} */," for n in SOURCES)
framework_children = "\n".join(f"\t\t\t\t{ids[n]} /* {n} */," for n in FRAMEWORKS)

COMMON = """\t\t\t\tCLANG_ENABLE_MODULES = YES;
\t\t\t\tSWIFT_VERSION = 5.0;
\t\t\t\tIPHONEOS_DEPLOYMENT_TARGET = 17.0;
\t\t\t\tSDKROOT = iphoneos;
\t\t\t\tTARGETED_DEVICE_FAMILY = "1,2";
\t\t\t\tALWAYS_SEARCH_USER_PATHS = NO;"""

# CODE_SIGNING_ALLOWED = NO so a simulator build needs no developer account.
# A device build overrides it, which is where a real identity is required.
TARGET_COMMON = """\t\t\t\tPRODUCT_NAME = Overnight;
\t\t\t\tPRODUCT_BUNDLE_IDENTIFIER = com.overnight.ios;
\t\t\t\tGENERATE_INFOPLIST_FILE = YES;
\t\t\t\tINFOPLIST_KEY_UILaunchScreen_Generation = YES;
\t\t\t\tINFOPLIST_KEY_CFBundleDisplayName = Overnight;
\t\t\t\tINFOPLIST_KEY_UIApplicationSceneManifest_Generation = YES;
\t\t\t\tCODE_SIGN_STYLE = Automatic;
\t\t\t\tCODE_SIGNING_ALLOWED = NO;
\t\t\t\tCODE_SIGNING_REQUIRED = NO;
\t\t\t\tENABLE_USER_SCRIPT_SANDBOXING = NO;
\t\t\t\tOTHER_LDFLAGS = "-lc++";
\t\t\t\tSWIFT_INCLUDE_PATHS = "$(BUILT_PRODUCTS_DIR)/include/vt $(BUILT_PRODUCTS_DIR)/include/client";
\t\t\t\tHEADER_SEARCH_PATHS = "$(BUILT_PRODUCTS_DIR)/include/**";"""

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
\t\t\t\t{P['sourcesGroup']} /* Overnight */,
\t\t\t\t{P['frameworksGroup']} /* Frameworks */,
\t\t\t\t{P['productsGroup']} /* Products */,
\t\t\t);
\t\t\tsourceTree = "<group>";
\t\t}};
\t\t{P['sourcesGroup']} /* Overnight */ = {{
\t\t\tisa = PBXGroup;
\t\t\tchildren = (
{source_children}
\t\t\t);
\t\t\tpath = Overnight;
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
\t\t\t\t{P['product']} /* Overnight.app */,
\t\t\t);
\t\t\tname = Products;
\t\t\tsourceTree = "<group>";
\t\t}};
/* End PBXGroup section */

/* Begin PBXNativeTarget section */
\t\t{P['target']} /* Overnight */ = {{
\t\t\tisa = PBXNativeTarget;
\t\t\tbuildConfigurationList = {P['targetConfigList']};
\t\t\tbuildPhases = (
\t\t\t\t{P['sourcesPhase']},
\t\t\t\t{P['frameworksPhase']},
\t\t\t\t{P['resourcesPhase']},
\t\t\t);
\t\t\tbuildRules = ();
\t\t\tdependencies = ();
\t\t\tname = Overnight;
\t\t\tproductName = Overnight;
\t\t\tproductReference = {P['product']};
\t\t\tproductType = "com.apple.product-type.application";
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
\t\t\t\t{P['target']} /* Overnight */,
\t\t\t);
\t\t}};
/* End PBXProject section */

/* Begin PBXResourcesBuildPhase section */
\t\t{P['resourcesPhase']} = {{
\t\t\tisa = PBXResourcesBuildPhase;
\t\t\tbuildActionMask = 2147483647;
\t\t\tfiles = ();
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
/* End XCConfigurationList section */
\t}};
\trootObject = {P['project']} /* Project object */;
}}
"""

here = pathlib.Path(__file__).parent
project = here / "Overnight.xcodeproj"
project.mkdir(exist_ok=True)
(project / "project.pbxproj").write_text(PBXPROJ)
print(f"wrote {project / 'project.pbxproj'}")
