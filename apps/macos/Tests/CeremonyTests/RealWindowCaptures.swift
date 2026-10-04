import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// The real window, captured (ov-278): `ContentView` itself, in a real titled
/// window off every screen, against the seeded scratch daemon
/// `scripts/mac-capture.sh` starts, one PNG per place and appearance.
///
/// Opt-in, and never in CI: enabled only by `FARCOOLER_CAPTURE_OUT`, which the
/// script sets. It refuses to run unless `FARCOOLER_HOME` and `FARCOOLER_BIN`
/// both name the scratch daemon, so a capture can never open on the fleet the
/// person at this Mac is using. Nothing here sends input: each place is
/// reached through the window's own launch, by writing where the window was
/// as the app records it, so what's drawn is what a relaunch would draw.
///
/// See ui-lane-common.md, "Capturing the real window".
@MainActor
struct RealWindowCaptures {
    nonisolated static let environment = ProcessInfo.processInfo.environment

    /// One place to capture: a file name, and what the window reopens to, as
    /// `SelectionMemory.key` stores it. Nil opens where the launch rule says,
    /// which is Needs You while anything waits.
    struct Place: Equatable {
        let name: String
        let selection: String?
    }

    /// The places the seed has, by its ids: Needs You, the board, a task, the
    /// worktree's tiled pair of shells (the second sits beside the worktree's
    /// own), and History.
    static func places(seed: [String: String]) -> [Place] {
        let ws = seed["workspace"] ?? "", wt = seed["worktree"] ?? ""
        var places = [
            Place(name: "needs-you", selection: nil),
            Place(name: "board", selection: "|\(ws)|"),
            Place(name: "terminals", selection: "|\(ws)|worktree:\(wt):\(seed["second"] ?? "")"),
            Place(name: "history", selection: "|\(ws)|history:done"),
        ]
        if let task = seed["task"] { places.insert(Place(name: "task", selection: "|\(ws)|task:\(task)"), at: 2) }
        return places
    }

    /// `FARCOOLER_CAPTURE_PLACES`: one `name=<selection>` per line, for a
    /// place the seed doesn't name. `name=` alone is the launch rule's place.
    static func extraPlaces(_ text: String?) -> [Place] {
        (text ?? "").split(whereSeparator: \.isNewline).compactMap { line in
            let parts = line.split(separator: "=", maxSplits: 1, omittingEmptySubsequences: false)
            guard parts.count == 2, !parts[0].isEmpty else { return nil }
            return Place(name: String(parts[0]), selection: parts[1].isEmpty ? nil : String(parts[1]))
        }
    }

    /// Each appearance a place is drawn in, by the suffix its file takes.
    static let variants: [(name: String, appearance: NSAppearance.Name)] = [
        ("light", .aqua), ("dark", .darkAqua),
        ("light-contrast", .accessibilityHighContrastAqua), ("dark-contrast", .accessibilityHighContrastDarkAqua),
    ]

    @Test(.enabled(if: RealWindowCaptures.environment["FARCOOLER_CAPTURE_OUT"] != nil))
    func capture() async throws {
        let env = Self.environment
        // The scratch daemon, or nothing: never the one this Mac's person uses.
        let home = try #require(env["FARCOOLER_HOME"], "FARCOOLER_HOME must name the scratch daemon")
        try #require(home.hasPrefix("/tmp/") || home.hasPrefix("/private/tmp/"), "FARCOOLER_HOME isn't a scratch home: \(home)")
        try #require(env["FARCOOLER_BIN"] != nil, "FARCOOLER_BIN must name this checkout's CLI")
        unsetenv("FARCOOLER_WORKSPACE")

        let out = URL(fileURLWithPath: try #require(env["FARCOOLER_CAPTURE_OUT"]))
        let stage = env["FARCOOLER_CAPTURE_STAGE"] ?? "capture"
        let wait = Double(env["FARCOOLER_CAPTURE_WAIT"] ?? "") ?? 5
        let seed: [String: String] =
            env["FARCOOLER_CAPTURE_SEED"]
            .flatMap { FileManager.default.contents(atPath: $0) }
            .flatMap { try? JSONSerialization.jsonObject(with: $0) as? [String: String] } ?? [:]
        let wanted = env["FARCOOLER_CAPTURE_ONLY"]
        let places = (Self.places(seed: seed) + Self.extraPlaces(env["FARCOOLER_CAPTURE_PLACES"]))
            .filter { wanted == nil || $0.name == wanted }
        try #require(!places.isEmpty, "no place to capture")

        // What every local test run shares (the xctest domain): put back as
        // it was when the captures are done (review L9).
        let defaults = UserDefaults.standard
        let touched = ["window.sessions.v1", SelectionMemory.destinationKey, SelectionMemory.key]
        let saved = Dictionary(uniqueKeysWithValues: touched.map { ($0, defaults.object(forKey: $0)) })
        defer { Self.restore(saved, in: defaults) }
        for place in places {
            // Where the window was, as a relaunch reads it, and nothing else:
            // no window record, no newer destination.
            defaults.removeObject(forKey: "window.sessions.v1")
            defaults.removeObject(forKey: SelectionMemory.destinationKey)
            if let selection = place.selection {
                defaults.set(selection, forKey: SelectionMemory.key)
            } else {
                defaults.removeObject(forKey: SelectionMemory.key)
            }
            let window = try await TitleBarHarness.window(ContentView(), width: 1360, height: 860)
            try await Task.sleep(for: .seconds(wait))
            try await TitleBarHarness.settle(window)
            for variant in Self.variants {
                window.appearance = NSAppearance(named: variant.appearance)
                try await TitleBarHarness.settle(window)
                let view = try #require(window.contentView?.superview)
                let rep = try #require(view.bitmapImageRepForCachingDisplay(in: view.bounds))
                view.cacheDisplay(in: view.bounds, to: rep)
                let png = try #require(rep.representation(using: .png, properties: [:]))
                try png.write(to: out.appendingPathComponent("\(stage)-\(place.name)-\(variant.name).png"))
            }
            window.close()
        }
    }

    /// Each key back as it was: a value set again, an absent one removed.
    static func restore(_ saved: [String: Any?], in defaults: UserDefaults) {
        for (key, value) in saved {
            if let value { defaults.set(value, forKey: key) } else { defaults.removeObject(forKey: key) }
        }
    }

    @Test func theDefaultsComeBackAsTheyWere() throws {
        let suite = "capture-restore-\(UUID().uuidString)"
        let defaults = try #require(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        defaults.set("|kept|", forKey: "a")
        let saved: [String: Any?] = ["a": defaults.object(forKey: "a"), "b": defaults.object(forKey: "b")]
        defaults.set("|capture|", forKey: "a")
        defaults.set("|capture|", forKey: "b")
        Self.restore(saved, in: defaults)
        #expect(defaults.string(forKey: "a") == "|kept|")
        #expect(defaults.object(forKey: "b") == nil)
    }

    @Test func extraPlacesReadNameAndSelection() {
        #expect(
            Self.extraPlaces("a=|ws|\nneeds=\n=bad\nnoequals") == [
                Place(name: "a", selection: "|ws|"), Place(name: "needs", selection: nil),
            ])
    }
}
