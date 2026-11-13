#!/usr/bin/env python3
"""Make a disposable app copy. No shipping file or project is changed.

Usage: python3 reports/ov-1008/prepare.py /tmp/ov1008 --frameworks <built xcframework directory>
The copied Release configuration uses -O, with DEBUG defined for the existing
runnerless fixtures and FrameWatch only. Both variants are the same binary.
"""
import argparse
import shutil
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
HERE = Path(__file__).resolve().parent


def prepare(destination, frameworks):
    if destination.exists():
        raise SystemExit(f"Refusing to overwrite {destination}")
    for relative in ("apps/ios", "apps/shared", "scripts"):
        shutil.copytree(ROOT / relative, destination / relative,
                        ignore=shutil.ignore_patterns("build", ".build", "*.xcodeproj", "__pycache__", "Frameworks"))
    shutil.copy(ROOT / "Cargo.toml", destination / "Cargo.toml")
    ios = destination / "apps/ios"
    (ios / "Frameworks").symlink_to(frameworks.resolve(), target_is_directory=True)
    # A private identity, even if the source checkout is a release tag.
    generator = ios / "generate-project.py"
    text = generator.read_text().replace('CHANNEL = version("channel")', 'CHANNEL = "local"')
    text = text.replace('BUNDLE_ID = "com.farcooler.ios" if CHANNEL == "stable" else f"com.farcooler.ios.{CHANNEL}"',
                        'BUNDLE_ID = "com.farcooler.spike.ov1008"')
    text = text.replace('"FarCoolerApp.swift",', '"FarCoolerApp.swift", "CollectionTranscript.swift",')
    text = text.replace('UI_TEST_SOURCES = [', 'UI_TEST_SOURCES = ["TranscriptSpikeUITests.swift",')
    generator.write_text(text)
    shutil.copy(HERE / "TranscriptSpikeUITests.swift", ios / "FarCoolerUITests/TranscriptSpikeUITests.swift")
    shutil.copy(HERE / "CollectionTranscript.swift", ios / "FarCooler/CollectionTranscript.swift")
    view = ios / "FarCooler/NativeAgentView.swift"
    text = view.read_text()
    start = text.index("        return ScrollView {", text.index("private var conversation:"))
    end = text.index("\n    }\n\n    #if DEBUG\n    /// What the list", start)
    original = text[start:end].replace("        return ScrollView {", "        ScrollView {", 1)
    replacement = '''        return Group {
            if CommandLine.arguments.contains("-uikit-transcript") {
                SpikeCollectionTranscript(model: model, margin: margin, items: store.items)
                    .safeAreaInset(edge: .bottom, spacing: 0) {
                        NativeComposer(model: model, showTerminal: showTerminal, room: room, raised: raised)
                    }
            } else {
''' + original + "\n            }\n        }"
    replacement += '''
        .onReceive(NotificationCenter.default.publisher(for: Notification.Name("ov1008-fling"))) { _ in
            pinned = false
            readerIsDriving = true
            place.release(oldest: store.ids.first)
        }
'''
    text = text[:start] + "        SpikeStats.store = store\n" + replacement + text[end:]
    view.write_text(text)
    live = ios / "FarCooler/NativeLiveHarness.swift"
    text = live.read_text().replace("max(1, end - AgentRowStore.pageSize)", "1")
    text = text.replace('"moreBefore": true, "rows": rows', '"moreBefore": false, "rows": rows')
    live.write_text(text)
    watch = ios / "FarCooler/FrameWatch.swift"
    text = watch.read_text().replace("private func start()", "func start()")
    text = text.replace("    private var frames = 0", "    private var frames = 0\n    private var rowsBefore = 0")
    text = text.replace("        madeBefore = SelectableTextView.made", "        rowsBefore = SpikeStats.store?.ids.count ?? 0\n        madeBefore = SelectableTextView.made")
    text = text.replace("        self.scroll = scroll", '''        self.scroll = scroll
        NotificationCenter.default.post(name: Notification.Name("ov1008-fling"), object: nil)
        SpikeStats.store?.scrollInProgress = true''')
    text = text.replace("        link?.invalidate()", "        SpikeStats.store?.scrollEnded()\n        link?.invalidate()")
    text = text.replace("    private var frames = 0", "    private var travel: CGFloat = 0\n    private var frames = 0")
    text = text.replace("        frames = 0", "        travel = 0\n        frames = 0")
    text = text.replace("        scroll.contentOffset.y = y", "        travel += abs(y - scroll.contentOffset.y)\n        scroll.contentOffset.y = y")
    text = text.replace('        NSLog("FRAME-WATCH', '''        report += " script-distance=\(Int(travel)) rows-start=\(rowsBefore) container=\(scroll is UICollectionView ? "UIKit" : "SwiftUI") rows=\(SpikeStats.store?.ids.count ?? 0) items=\(SpikeStats.store?.items.count ?? 0)"
        let url = URL.documentsDirectory.appending(path: "fling.txt")
        try? report.write(to: url, atomically: true, encoding: .utf8)
        NSLog("FRAME-WATCH''')
    text = text.replace('.accessibilityValue(watch.report)', '''.accessibilityValue(watch.report)
            .task {
                try? await Task.sleep(for: .seconds(5))
                watch.start()
            }''')
    watch.write_text(text)
    subprocess.run(["python3", str(generator)], check=True)
    project = ios / "FarCooler.xcodeproj/project.pbxproj"
    text = project.read_text().replace('SWIFT_OPTIMIZATION_LEVEL = "-O";',
                                     'SWIFT_OPTIMIZATION_LEVEL = "-O"; SWIFT_ACTIVE_COMPILATION_CONDITIONS = DEBUG;')
    project.write_text(text)
    scheme = ios / "FarCooler.xcodeproj/xcshareddata/xcschemes"
    scheme.mkdir(parents=True, exist_ok=True)
    shutil.copy(ROOT / "apps/ios/FarCooler.xcodeproj/xcshareddata/xcschemes/FarCooler.xcscheme", scheme)
    print(ios / "FarCooler.xcodeproj")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("destination", type=Path)
    parser.add_argument("--frameworks", type=Path, required=True)
    args = parser.parse_args()
    prepare(args.destination.resolve(), args.frameworks)
