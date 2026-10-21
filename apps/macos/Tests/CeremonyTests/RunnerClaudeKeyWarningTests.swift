import AgentKit
import AppKit
import SwiftUI
import TestWindows
import Testing

@testable import Far_Cooler

/// The owner's warning that their shell's startup files export a Claude key
/// (ov-651): shown at the top of Runner Settings when `status --json` carries
/// `claudeKeyWarning`, absent when it doesn't.
@MainActor
@Suite(.serialized)
struct RunnerClaudeKeyWarningTests {
    static let sentence =
        "Your shell’s startup files set ANTHROPIC_API_KEY, CLAUDE_CODE_OAUTH_TOKEN and ANTHROPIC_AUTH_TOKEN, so a member typing claude in their own shell would use your account."

    /// A store whose `status --json` is `status`; everything else is empty.
    static func store(status: String, asked: RunnerPeopleSettingsTests.Answered) -> RunnerSettingsStore {
        let store = RunnerSettingsStore(target: "box")
        store.cliForTesting = { args in
            if args.suffix(1) == ["status"] {
                asked.memberList = true
                return CLI.Result(ok: true, output: status, errors: "")
            }
            if args.contains("settings") { return CLI.Result(ok: true, output: #"{"branchPrefix":""}"#, errors: "") }
            return CLI.Result(ok: true, output: #"{"themes":[]}"#, errors: "")
        }
        return store
    }

    /// What the sheet drew once `status` was read, and its window.
    static func drawn(status: String, scheme: ColorScheme = .light) async -> (Set<String>, NSWindow) {
        let asked = RunnerPeopleSettingsTests.Answered()
        let store = store(status: status, asked: asked)
        let seen = RunnerPeopleSettingsTests.Seen()
        let host = NSHostingView(
            rootView: RunnerPeopleSettingsTests.Probed(store: store, seen: seen).environment(\.colorScheme, scheme))
        let size = SettingsSheetSize.runner
        let window = NSWindow(
            contentRect: NSRect(x: -4000, y: -4000, width: size.width, height: size.height),
            styleMask: [.borderless], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.appearance = NSAppearance(named: scheme == .dark ? .darkAqua : .aqua)
        window.contentView = host
        TestWindow.show(window)
        await Settle.until {
            host.layoutSubtreeIfNeeded()
            return asked.memberList && !store.loading
        }
        await Settle.still(host, reading: { seen.views.keys.sorted().joined(separator: ",") + "\(seen.views.values)" })
        return (Set(seen.views.keys), window)
    }

    static var warned: String { #"{"claudeKeyWarning":"\#(sentence)"}"# }

    @Test("A runner whose status carries the warning shows it")
    func shown() async {
        let (drawn, window) = await Self.drawn(status: Self.warned)
        window.close()
        #expect(drawn.contains("runner-claude-key-warning"), "\(drawn)")
    }

    @Test("A runner whose status carries none, or null, shows no warning")
    func hidden() async {
        for status in [#"{"claudeKeyWarning":null}"#, #"{"runtimeHealthy":true}"#] {
            let (drawn, window) = await Self.drawn(status: status)
            window.close()
            #expect(!drawn.contains("runner-claude-key-warning"), "\(status): \(drawn)")
        }
    }

    /// The sheet with the long three-name sentence, light and dark, into
    /// `FARCOOLER_CAPTURE_OUT`. Opt-in: a capture tool, never run in CI.
    @Test("Captures of the warning", .enabled(if: ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"] != nil))
    func captures() async throws {
        let out = URL(fileURLWithPath: ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"]!)
        try FileManager.default.createDirectory(at: out, withIntermediateDirectories: true)
        for scheme in [ColorScheme.light, .dark] {
            let (_, window) = await Self.drawn(status: Self.warned, scheme: scheme)
            let view = try #require(window.contentView)
            let rep = try #require(view.bitmapImageRepForCachingDisplay(in: view.bounds))
            view.cacheDisplay(in: view.bounds, to: rep)
            let png = try #require(rep.representation(using: .png, properties: [:]))
            try png.write(to: out.appendingPathComponent("mac-claude-key-warning-\(scheme == .dark ? "dark" : "light").png"))
            window.close()
        }
    }
}
