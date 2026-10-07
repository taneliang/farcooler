import Foundation
import Testing

/// Every test that starts the CLI or a daemon under its own `FARCOOLER_HOME`
/// gives it its own `FARCOOLER_CONFIG` too (ov-394). The daemon's config.toml
/// is shared by every runner on a Mac unless told otherwise, and carries
/// settings (`[agents] projector`, ov-372) that change what a test sees, and
/// that a test could write to the owner's file.
///
/// So the key is named in one place, `ScratchDaemon.isolate`, which sets both.
/// This scans the test sources, since each file builds its own environment:
/// a source that names `"FARCOOLER_HOME"` anywhere but there fails, however
/// it assigns it (a subscript, a dictionary literal, `setenv`), unless it's
/// on the short list below of what only reads it or can't reach the helper.
struct ScratchConfigTests {
    /// Sources that name the key without launching a scratch daemon of their
    /// own, and why that's safe.
    static let reads: [String: String] = [
        "AppControlTests.swift": "hands a dictionary to a pure function and starts nothing",
        "RealWindowCaptures.swift": "reads the home the capture script set, which sets the config too",
        "SelectionSwapTimingTests+Workspaces.swift": "reads the home a script set, which sets the config too",
    ]

    /// In another test target, where the helper isn't visible: the same line
    /// must also name the config.
    static let sameLine: Set<String> = ["PlanSpecimenTests.swift"]

    @Test("Only ScratchDaemon.isolate names FARCOOLER_HOME")
    func every_scratch_home_has_its_own_config() throws {
        let tests = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent()
        var seen = 0
        var strays: [String] = []
        let files = try #require(FileManager.default.enumerator(at: tests, includingPropertiesForKeys: nil))
        for case let file as URL in files where file.pathExtension == "swift" {
            let name = file.lastPathComponent
            let text = try String(contentsOf: file, encoding: .utf8)
            let lines = text.split(separator: "\n").filter { $0.contains("\"FARCOOLER_HOME\"") && !$0.hasPrefix("///") }
            guard !lines.isEmpty, !["ScratchDaemon.swift", "ScratchConfigTests.swift"].contains(name) else { continue }
            seen += 1
            if Self.reads[name] != nil { continue }
            if Self.sameLine.contains(name), lines.allSatisfy({ $0.contains("\"FARCOOLER_CONFIG\"") }) { continue }
            strays.append(name)
        }
        #expect(seen >= 2, "the scan found only \(seen) files naming the key, so it isn't scanning")
        #expect(strays.isEmpty, "use ScratchDaemon.isolate instead of naming FARCOOLER_HOME in: \(strays.sorted())")
        let helper = try String(contentsOf: tests.appendingPathComponent("CeremonyTests/ScratchDaemon.swift"), encoding: .utf8)
        #expect(helper.contains("\"FARCOOLER_HOME\"") && helper.contains("\"FARCOOLER_CONFIG\""), "the helper must set both")
    }
}
