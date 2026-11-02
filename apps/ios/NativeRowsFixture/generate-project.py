#!/usr/bin/env python3
"""Generate a separate, simulator-only fixture host; never edits the production project."""
from pathlib import Path
import uuid

ROOT = Path(__file__).resolve().parent
FIXTURE = "../../../test/fixtures/agent-rows-ov893-producer.json"
HOST_SOURCES = [
    "FixtureHost.swift", "FixtureLoader.swift",
    "../FarCooler/NativeToolRows.swift", "../FarCooler/NativeMachineryHelpers.swift",
    "../../shared/AgentKit/Sources/AgentKit/AgentRowWire.swift",
    "../../shared/AgentKit/Sources/AgentKit/LiteralOperation.swift",
    "../../shared/AgentKit/Sources/AgentKit/AgentConversationBase.swift",
    "../../shared/AgentKit/Sources/AgentKit/ConversationItems.swift",
    "../../shared/AgentKit/Sources/AgentKit/AgentTrayActivity.swift",
    "../../shared/AgentKit/Sources/AgentKit/AgentRowBox.swift",
    "../../shared/AgentKit/Sources/AgentKit/DesignTokens.swift",
    "../../shared/AgentKit/Sources/AgentKit/GlancePalette.swift",
    "../../shared/AgentKit/Sources/AgentKit/SelectableText.swift",
    "../../shared/AgentKit/Sources/AgentKit/ReusePool.swift",
]
TEST_SOURCES = {"FixtureLoaderTests": ["FixtureLoaderTests.swift"], "FixtureHostUITests": ["FixtureHostUITests.swift"]}
BUNDLE_IDS = {"NativeRowsFixtureHost": "com.farcooler.fixture.native-rows", "FixtureLoaderTests": "com.farcooler.fixture.native-rows.tests", "FixtureHostUITests": "com.farcooler.fixture.native-rows.uitests"}

def oid(value):
    return uuid.uuid5(uuid.NAMESPACE_URL, "farcooler-fixture/" + value).hex[:24].upper()

def quote(value):
    return '"' + value.replace('\\', '\\\\').replace('"', '\\"') + '"'

def render():
    objects = []
    def add(key, body):
        objects.append(f"\t\t{oid(key)} = {{ {body} }};")
    targets = ["NativeRowsFixtureHost", "FixtureLoaderTests", "FixtureHostUITests"]
    sources = {"NativeRowsFixtureHost": HOST_SOURCES, **TEST_SOURCES}
    paths = list(dict.fromkeys(HOST_SOURCES + sum(TEST_SOURCES.values(), []) + [FIXTURE]))
    for path in paths:
        kind = "text.json" if path == FIXTURE else "sourcecode.swift"
        add("file/" + path, f"isa = PBXFileReference; lastKnownFileType = {kind}; path = {quote(path)}; sourceTree = SOURCE_ROOT;")
    for name in targets:
        app = name == targets[0]
        product = name + (".app" if app else ".xctest")
        product_type = "wrapper.application" if app else "wrapper.cfbundle"
        add("product/" + name, f"isa = PBXFileReference; explicitFileType = {product_type}; path = {quote(product)}; sourceTree = BUILT_PRODUCTS_DIR;")
        for path in sources[name]:
            add("build/" + name + "/" + path, f"isa = PBXBuildFile; fileRef = {oid('file/' + path)};")
        resources = [FIXTURE] if name != "FixtureHostUITests" else []
        for path in resources:
            add("resource/" + name + "/" + path, f"isa = PBXBuildFile; fileRef = {oid('file/' + path)};")
        src_ids = " ".join(oid("build/" + name + "/" + p) + "," for p in sources[name])
        res_ids = " ".join(oid("resource/" + name + "/" + p) + "," for p in resources)
        add("sources/" + name, f"isa = PBXSourcesBuildPhase; buildActionMask = 2147483647; files = ({src_ids}); runOnlyForDeploymentPostprocessing = 0;")
        add("resources/" + name, f"isa = PBXResourcesBuildPhase; buildActionMask = 2147483647; files = ({res_ids}); runOnlyForDeploymentPostprocessing = 0;")
        add("frameworks/" + name, "isa = PBXFrameworksBuildPhase; buildActionMask = 2147483647; files = (); runOnlyForDeploymentPostprocessing = 0;")
        dependencies = ""
        if not app:
            add("proxy/" + name, f"isa = PBXContainerItemProxy; containerPortal = {oid('project')}; proxyType = 1; remoteGlobalIDString = {oid('target/NativeRowsFixtureHost')}; remoteInfo = NativeRowsFixtureHost;")
            add("dependency/" + name, f"isa = PBXTargetDependency; target = {oid('target/NativeRowsFixtureHost')}; targetProxy = {oid('proxy/' + name)};")
            dependencies = oid("dependency/" + name) + ","
        settings = ["SDKROOT = iphonesimulator", 'SUPPORTED_PLATFORMS = "iphonesimulator"', "IPHONEOS_DEPLOYMENT_TARGET = 26.0", "SWIFT_VERSION = 5.0", "TARGETED_DEVICE_FAMILY = \"1,2\"", f"PRODUCT_NAME = {name}", f"PRODUCT_BUNDLE_IDENTIFIER = {BUNDLE_IDS[name]}", "CODE_SIGN_STYLE = Automatic"]
        if app:
            settings += ["INFOPLIST_FILE = Info.plist", "DEFINES_MODULE = YES", "ENABLE_TESTABILITY = YES"]
        else:
            settings += ["GENERATE_INFOPLIST_FILE = YES"]
            if name == "FixtureLoaderTests": settings += ['TEST_HOST = "$(BUILT_PRODUCTS_DIR)/NativeRowsFixtureHost.app/NativeRowsFixtureHost"', 'BUNDLE_LOADER = "$(TEST_HOST)"']
            else: settings += ["TEST_TARGET_NAME = NativeRowsFixtureHost"]
        for config in ["Debug", "Release"]:
            add("config/" + name + "/" + config, f"isa = XCBuildConfiguration; buildSettings = {{ {'; '.join(settings)}; }}; name = {config};")
        add("configlist/" + name, f"isa = XCConfigurationList; buildConfigurations = ({oid('config/' + name + '/Debug')}, {oid('config/' + name + '/Release')},); defaultConfigurationIsVisible = 0; defaultConfigurationName = Debug;")
        kind = "com.apple.product-type.application" if app else ("com.apple.product-type.bundle.unit-test" if name == "FixtureLoaderTests" else "com.apple.product-type.bundle.ui-testing")
        phases = " ".join(oid(p + "/" + name) + "," for p in ["sources", "frameworks", "resources"])
        add("target/" + name, f"isa = PBXNativeTarget; buildConfigurationList = {oid('configlist/' + name)}; buildPhases = ({phases}); buildRules = (); dependencies = ({dependencies}); name = {name}; productName = {name}; productReference = {oid('product/' + name)}; productType = {quote(kind)};")
    file_ids = " ".join(oid("file/" + p) + "," for p in paths)
    products = " ".join(oid("product/" + n) + "," for n in targets)
    add("group/products", f"isa = PBXGroup; children = ({products}); name = Products; sourceTree = \"<group>\";")
    add("group/main", f"isa = PBXGroup; children = ({file_ids} {oid('group/products')},); sourceTree = \"<group>\";")
    for config in ["Debug", "Release"]:
        add("config/project/" + config, f"isa = XCBuildConfiguration; buildSettings = {{ SWIFT_VERSION = 5.0; }}; name = {config};")
    add("configlist/project", f"isa = XCConfigurationList; buildConfigurations = ({oid('config/project/Debug')}, {oid('config/project/Release')},); defaultConfigurationIsVisible = 0; defaultConfigurationName = Debug;")
    target_ids = " ".join(oid("target/" + n) + "," for n in targets)
    add("project", f"isa = PBXProject; attributes = {{ LastUpgradeCheck = 2700; }}; buildConfigurationList = {oid('configlist/project')}; compatibilityVersion = \"Xcode 14.0\"; developmentRegion = en; hasScannedForEncodings = 0; knownRegions = (en, Base,); mainGroup = {oid('group/main')}; productRefGroup = {oid('group/products')}; projectDirPath = \"\"; projectRoot = \"\"; targets = ({target_ids});")
    return "// !$*UTF8*$!\n{ archiveVersion = 1; classes = {}; objectVersion = 56; objects = {\n" + "\n".join(objects) + f"\n\t}}; rootObject = {oid('project')};\n}}\n"

def scheme():
    def ref(name, suffix):
        return f'<BuildableReference BuildableIdentifier="primary" BlueprintIdentifier="{oid("target/" + name)}" BuildableName="{name}.{suffix}" BlueprintName="{name}" ReferencedContainer="container:NativeRowsFixture.xcodeproj"/>'
    host = ref("NativeRowsFixtureHost", "app")
    entries = "".join(f'<BuildActionEntry buildForTesting="YES" buildForRunning="NO" buildForProfiling="NO" buildForArchiving="NO" buildForAnalyzing="NO">{ref(n, "app" if n == "NativeRowsFixtureHost" else "xctest")}</BuildActionEntry>' for n in BUNDLE_IDS)
    tests = "".join(f'<TestableReference skipped="NO">{ref(n, "xctest")}</TestableReference>' for n in TEST_SOURCES)
    return f'''<?xml version="1.0" encoding="UTF-8"?><Scheme LastUpgradeVersion="2700" version="1.7"><BuildAction parallelizeBuildables="NO" buildImplicitDependencies="YES"><BuildActionEntries>{entries}</BuildActionEntries></BuildAction><TestAction buildConfiguration="Debug" selectedDebuggerIdentifier="Xcode.DebuggerFoundation.Debugger.LLDB" selectedLauncherIdentifier="Xcode.DebuggerFoundation.Launcher.LLDB" shouldUseLaunchSchemeArgsEnv="YES"><Testables>{tests}</Testables></TestAction><LaunchAction buildConfiguration="Debug"><BuildableProductRunnable runnableDebuggingMode="0">{host}</BuildableProductRunnable></LaunchAction></Scheme>'''

def main():
    project = ROOT / "NativeRowsFixture.xcodeproj"
    (project / "xcshareddata/xcschemes").mkdir(parents=True, exist_ok=True)
    (project / "project.pbxproj").write_text(render())
    (project / "xcshareddata/xcschemes/NativeRowsFixture.xcscheme").write_text(scheme())

if __name__ == "__main__":
    main()
