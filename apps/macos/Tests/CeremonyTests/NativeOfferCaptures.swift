import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// Opt-in (FARCOOLER_CAPTURE_OUT): a Claude pane whose conversation isn't
/// offered (ov-443), its dimmed switch over the terminal and what its popover
/// says, in the four appearances. The production `NativeSwitch` and
/// `ConversationUnavailableNote`, in a real titled window off every screen,
/// taken with `screencapture -l` of that window. Sends no input.
@MainActor
@Suite(.serialized, .enabled(if: ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"] != nil))
struct NativeOfferCaptures {
    @Test func theDimmedSwitchAndItsReason() async throws {
        let out = try #require(ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"])
        try FileManager.default.createDirectory(atPath: out, withIntermediateDirectories: true)
        let claude = try NativeOfferTests.terminal(#""preset":"Fix the login bug","program":"shell","runningAgent":"claude""#)
        let reasons: [(String, AgentConversation.Unavailable)] = [
            ("update", .runnerNeedsUpdate), ("pairing", .pairingNeeded), ("setting", .settingOff),
        ]
        for (name, appearance) in RealWindowCaptures.variants {
            let agents = NativeOfferTests.agents(offered: ["agent_compose"])
            let life = NativeAgentTests.SurfaceLife()
            let pane = NativeAgentTests.window(NativeSwitch(terminal: claude, target: "", isFocused: true, agents: agents) { focused in
                NativeAgentTests.StandInSurface(life: life, focused: focused)
            })
            pane.appearance = NSAppearance(named: appearance)
            await NativeAgentTests.settle(pane, 400)
            let image = try #require(RealWindowCaptures.windowImage(pane), "screencapture -l isn't allowed here")
            try #require(image.representation(using: .png, properties: [:]))
                .write(to: URL(fileURLWithPath: out).appendingPathComponent("offer-switch-\(name).png"))
            pane.close()
            for (reason, unavailable) in reasons {
                let note = NativeAgentTests.window(
                    ConversationUnavailableNote(reason: unavailable) { _ in }
                        .surface(.content, in: .floating)
                        .frame(maxWidth: .infinity, maxHeight: .infinity))
                note.appearance = NSAppearance(named: appearance)
                await NativeAgentTests.settle(note, 300)
                let shot = try #require(RealWindowCaptures.windowImage(note))
                try #require(shot.representation(using: .png, properties: [:]))
                    .write(to: URL(fileURLWithPath: out).appendingPathComponent("offer-note-\(reason)-\(name).png"))
                note.close()
            }
        }
    }
}
