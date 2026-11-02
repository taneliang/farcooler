"""Source-phase and isolation checks; run only in a separately allocated test gate."""
import hashlib
import importlib.util
import plistlib
import subprocess
import tempfile
import unittest
import xml.etree.ElementTree as ET
from pathlib import Path

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("fixture_generator", HERE / "generate-project.py")
generator = importlib.util.module_from_spec(spec)
spec.loader.exec_module(generator)

class FixtureProjectTests(unittest.TestCase):
    def project(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "project.pbxproj"
            path.write_text(generator.render())
            xml = subprocess.run(["/usr/bin/plutil", "-convert", "xml1", "-o", "-", str(path)], check=True, capture_output=True).stdout
            return plistlib.loads(xml)["objects"]

    def test_exact_host_membership_and_separate_identity(self):
        objects = self.project()
        host = objects[generator.oid("target/NativeRowsFixtureHost")]
        phases = [objects[key] for key in host["buildPhases"]]
        self.assertEqual([x["isa"] for x in phases], ["PBXSourcesBuildPhase", "PBXFrameworksBuildPhase", "PBXResourcesBuildPhase"])
        source_paths = {objects[objects[key]["fileRef"]]["path"] for key in phases[0]["files"]}
        expected = {
            "FixtureHost.swift", "FixtureLoader.swift", "../FarCooler/NativeToolRows.swift",
            "../FarCooler/NativeMachineryHelpers.swift",
            *{"../../shared/AgentKit/Sources/AgentKit/" + name for name in [
                "AgentRowWire.swift", "LiteralOperation.swift", "AgentConversationBase.swift", "ConversationItems.swift",
                "AgentTrayActivity.swift", "AgentRowBox.swift", "DesignTokens.swift", "GlancePalette.swift",
                "SelectableText.swift", "ReusePool.swift",
            ]},
        }
        self.assertEqual(source_paths, expected)
        self.assertEqual(len(source_paths), len(generator.HOST_SOURCES))
        self.assertEqual(phases[1]["files"], [])
        self.assertEqual([objects[objects[key]["fileRef"]]["path"] for key in phases[2]["files"]], [generator.FIXTURE])
        self.assertEqual(hashlib.sha256((HERE / generator.FIXTURE).read_bytes()).hexdigest(), "0f963393dc148ad45290ce4b90116d80ca76ef9294e72882cd3e287d5c4419de")
        self.assertEqual(host["dependencies"], [])
        forbidden = ("FarCoolerApp.swift", "AppLockGate", "Connection.swift", "AgentRowStore", "AgentRowCache", "AgentRowLedger", "AgentTray.swift", "GlancePaletteFleet", "NativeRows.swift", "NativeAgentHarness", "Notifications.swift")
        self.assertFalse(any(part in path for path in source_paths for part in forbidden))
        config = objects[objects[host["buildConfigurationList"]]["buildConfigurations"][0]]["buildSettings"]
        self.assertEqual(config["PRODUCT_BUNDLE_IDENTIFIER"], "com.farcooler.fixture.native-rows")
        self.assertEqual(config["SUPPORTED_PLATFORMS"], "iphonesimulator")
        self.assertEqual(config["INFOPLIST_FILE"], "Info.plist")
        self.assertNotIn("CODE_SIGN_ENTITLEMENTS", config)
        self.assertNotIn("FARCOOLER_APP_GROUP", config)
        self.assertNotIn("OTHER_LDFLAGS", config)
        plist = plistlib.loads((HERE / "Info.plist").read_bytes())
        self.assertFalse({"UIBackgroundModes", "CFBundleURLTypes", "FarCoolerAppGroup"} & set(plist))

    def test_target_ids_dependencies_and_scheme_are_separate(self):
        objects = self.project()
        ids = []
        for name in generator.BUNDLE_IDS:
            target = objects[generator.oid("target/" + name)]
            for phase in target["buildPhases"]:
                ids.extend(objects[phase]["files"])
            if name != "NativeRowsFixtureHost":
                dep = objects[target["dependencies"][0]]
                self.assertEqual(dep["target"], generator.oid("target/NativeRowsFixtureHost"))
        self.assertEqual(len(ids), len(set(ids)), "build-file IDs must be target-specific")
        self.assertEqual(set(generator.BUNDLE_IDS.values()), {"com.farcooler.fixture.native-rows", "com.farcooler.fixture.native-rows.tests", "com.farcooler.fixture.native-rows.uitests"})
        scheme = ET.fromstring(generator.scheme())
        refs = scheme.findall(".//TestableReference/BuildableReference")
        self.assertEqual({ref.attrib["BlueprintName"] for ref in refs}, set(generator.TEST_SOURCES))
        ui = objects[generator.oid("target/FixtureHostUITests")]
        settings = objects[objects[ui["buildConfigurationList"]]["buildConfigurations"][0]]["buildSettings"]
        self.assertEqual(settings["TEST_TARGET_NAME"], "NativeRowsFixtureHost")
        unit = objects[generator.oid("target/FixtureLoaderTests")]
        unit_settings = objects[objects[unit["buildConfigurationList"]]["buildConfigurations"][0]]["buildSettings"]
        self.assertEqual(unit_settings["TEST_HOST"], "$(BUILT_PRODUCTS_DIR)/NativeRowsFixtureHost.app/NativeRowsFixtureHost")
        self.assertEqual(unit_settings["BUNDLE_LOADER"], "$(TEST_HOST)")

if __name__ == "__main__":
    unittest.main()
