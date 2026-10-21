import AppKit
import Foundation
import Testing

/// A capture is kept only once the state it is named for is on screen (ov-642).
///
/// Four "Done" captures of the sign-in sheet were byte-identical to the code
/// step's, because the test shot before the model had reached Done, and a
/// capture of the wrong state passes and misleads every polish review after it.
/// So the way to keep a capture is `Capture.keep`, whose `expecting` (what the
/// shot is of, in words) and `until` (a check that is true once it is) are not
/// optional: it waits for `until`, throws, and writes nothing if the state is
/// never reached, and only then draws and writes the file.
///
/// Name a state by what only it has: the model's step, a notice that is set,
/// an identifier a probe has seen. (Not by reading the hosting view: its
/// accessibility tree is empty when no assistive technology has asked, which
/// is why the tests here use probes.)
@MainActor
enum Capture {
    /// How long a state has to arrive. Far under `Settle.limit`: a capture
    /// whose state hasn't come in this long isn't coming.
    static let limit: Duration = .seconds(15)

    struct Missed: Error, CustomStringConvertible {
        let name: String
        let state: String
        var description: String { "\(name): the state \"\(state)\" was never reached, so nothing was shot" }
    }

    /// Waits for `reached`, then writes what `png` draws to `url`. Throws
    /// `Missed` and writes nothing when the state doesn't come.
    static func keep(
        to url: URL, expecting state: String, until reached: @MainActor () -> Bool, png: @MainActor () throws -> Data?
    ) async throws {
        guard await Settle.until(limit: limit, reached) else {
            let missed = Missed(name: url.lastPathComponent, state: state)
            Issue.record(Comment(rawValue: missed.description))
            throw missed
        }
        let data = try #require(try png(), "\(url.lastPathComponent): nothing was drawn (screencapture -l may not be allowed here)")
        try data.write(to: url)
    }

    /// `view` drawn with `cacheDisplay` and written as a PNG, once `reached`.
    static func keep(
        _ view: NSView, to url: URL, expecting state: String, until reached: @MainActor () -> Bool
    ) async throws {
        try await keep(to: url, expecting: state, until: reached) {
            guard let rep = view.bitmapImageRepForCachingDisplay(in: view.bounds) else { return nil }
            view.cacheDisplay(in: view.bounds, to: rep)
            return rep.representation(using: .png, properties: [:])
        }
    }

    /// Waits for `reached` and throws `Missed` when it doesn't come: for a
    /// helper that draws and writes the file itself.
    static func wait(expecting state: String, until reached: @MainActor () -> Bool) async throws {
        guard await Settle.until(limit: limit, reached) else {
            let missed = Missed(name: "the capture", state: state)
            Issue.record(Comment(rawValue: missed.description))
            throw missed
        }
    }
}
