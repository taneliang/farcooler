import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// Opt-in (FARCOOLER_CAPTURE_OUT): the conversation's quiet text on a light
/// and a dark theme (ov-686), for judging that the hierarchy still reads. The
/// rows are `NativePolishTests`'s: a long message, a run of calls, a queued
/// message, a subagent, a task list and the answers. For the before, set
/// `Ink.quiet` to `Color.secondary` and run it again.
@MainActor
@Suite(.serialized, .enabled(if: ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"] != nil))
struct QuietInkCaptures {
    typealias T = NativeAgentTests

    @Test(arguments: [
        ("solarized-light", UInt32(0xFD_F6_E3), NSAppearance.Name.aqua),
        ("nord", UInt32(0x2E_34_40), NSAppearance.Name.darkAqua),
    ])
    func conversation(name: String, ground: UInt32, appearance: NSAppearance.Name) async throws {
        let out = try #require(ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"])
        try FileManager.default.createDirectory(atPath: out, withIntermediateDirectories: true)
        let saved = Themes.shared.current.backgroundColor
        defer { ThemeBackdrop.set(saved) }
        ThemeBackdrop.set(Theme.color(ground))
        let model = T.model(try T.terminal())
        let window = NativePolishTests.window(width: 640, height: 1_300, NativeAgentView(model: model, isFocused: true, showTerminal: {}))
        defer { window.close() }
        window.appearance = NSAppearance(named: appearance)
        model.store.apply(try await model.store.ledger.page(T.page(NativePolishTests.rows())))
        try await Capture.keep(
            window, to: URL(fileURLWithPath: out).appendingPathComponent("quiet-\(name).png"), expecting: "the conversation, to its last answer"
        ) { model.store.ids.contains("prose:2") }
    }
}
