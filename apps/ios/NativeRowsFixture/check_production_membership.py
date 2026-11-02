#!/usr/bin/env python3
"""Inspect an already generated production pbxproj; never generate or edit it."""
import plistlib
import subprocess
import sys

xml = subprocess.run(["/usr/bin/plutil", "-convert", "xml1", "-o", "-", sys.argv[1]], check=True, capture_output=True).stdout
objects = plistlib.loads(xml)["objects"]
targets = {v["name"]: v for v in objects.values() if v.get("isa") == "PBXNativeTarget"}

def sources(name):
    result = []
    for phase_id in targets[name]["buildPhases"]:
        phase = objects[phase_id]
        if phase["isa"] == "PBXSourcesBuildPhase":
            for build_id in phase["files"]:
                result.append((build_id, objects[objects[build_id]["fileRef"]]["path"]))
    return result

phone = sources("FarCooler")
for name in ["NativeMachineryHelpers.swift", "NativeConversationSupplement.swift", "AgentConversationBase.swift", "AgentRowBox.swift", "AgentTrayActivity.swift", "GlancePaletteFleet.swift"]:
    assert sum(path == name for _, path in phone) == 1, ("phone", name)
all_ids = []
for target in ["FarCooler", "FarCoolerActivity", "FarCoolerWatch", "FarCoolerWatchWidgets"]:
    matches = [build for build, path in sources(target) if path == "GlancePaletteFleet.swift"]
    assert len(matches) == 1, (target, matches)
    all_ids.extend(matches)
assert len(set(all_ids)) == 4, all_ids
assert all(path != "GlancePaletteFleet.swift" for _, path in sources("FarCoolerNotify"))
