import Foundation
import Testing

/// Every test that starts the CLI or a daemon under its own `FARCOOLER_HOME`
/// gives it its own `FARCOOLER_CONFIG` too (ov-394). The daemon's config.toml
/// is shared by every runner on a Mac unless told otherwise, and carries
/// settings (`[agents] projector`, ov-372) that change what a test sees, and
/// that a test could write to the owner's file.
///
/// A scan of the test sources, since each file builds its own environment: a
/// file that assigns `FARCOOLER_HOME` and never names `FARCOOLER_CONFIG`
/// fails here, so a new one can't slip past.
struct ScratchConfigTests {
    @Test("Tests that set a scratch FARCOOLER_HOME also set FARCOOLER_CONFIG")
    func every_scratch_home_has_its_own_config() throws {
        let tests = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent()
        let assigns = #/(\["FARCOOLER_HOME"\]\s*=[^=])|(\[\s*"FARCOOLER_HOME"\s*:)/#
        // AppControlTests only hands a dictionary to a pure function; it starts nothing.
        let startsNothing: Set = ["AppControlTests.swift", "ScratchConfigTests.swift"]
        var seen = 0
        var missing: [String] = []
        let files = try #require(FileManager.default.enumerator(at: tests, includingPropertiesForKeys: nil))
        for case let file as URL in files where file.pathExtension == "swift" {
            let text = try String(contentsOf: file, encoding: .utf8)
            guard text.contains(assigns), !startsNothing.contains(file.lastPathComponent) else { continue }
            seen += 1
            if !text.contains("\"FARCOOLER_CONFIG\"") { missing.append(file.lastPathComponent) }
        }
        #expect(seen >= 8, "the scan found only \(seen) files that set a home, so it isn't scanning")
        #expect(missing.isEmpty, "set FARCOOLER_CONFIG beside FARCOOLER_HOME in: \(missing.sorted())")
    }
}
