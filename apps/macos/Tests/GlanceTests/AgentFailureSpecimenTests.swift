import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// The chat pane's failure lines, rendered in both appearances so their
/// placement and ink can be looked at (ov-136). Not an assertion beyond
/// "it drew": a rendering, written where `FARCOOLER_GLANCE_OUT` says.
@MainActor
struct AgentFailureSpecimenTests {
    @Test("Write the agent failure sheets")
    func writeSheets() throws {
        let directory = URL(
            fileURLWithPath: ProcessInfo.processInfo.environment["FARCOOLER_GLANCE_OUT"]
                ?? FileManager.default.currentDirectoryPath + "/.build/glance")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        // An `NSHostingView` drawn into a bitmap rather than `ImageRenderer`,
        // which draws AppKit-backed controls (the buttons) as placeholders.
        for dark in [false, true] {
            let host = NSHostingView(
                rootView: AgentFailureSpecimen()
                    .background(dark ? Color(white: 0.12) : Color.white))
            host.appearance = NSAppearance(named: dark ? .darkAqua : .aqua)
            host.frame = CGRect(origin: .zero, size: host.fittingSize)
            host.layoutSubtreeIfNeeded()
            let rep = try #require(host.bitmapImageRepForCachingDisplay(in: host.bounds))
            host.cacheDisplay(in: host.bounds, to: rep)
            let png = try #require(rep.representation(using: .png, properties: [:]))
            try png.write(to: directory.appendingPathComponent("agent-failures-\(dark ? "dark" : "light").png"))
        }
    }
}

/// Each failure state as the pane draws it.
private struct AgentFailureSpecimen: View {
    private let options = [
        PermissionOption(id: "allow", name: "Allow", kind: "allow_once"),
        PermissionOption(id: "deny", name: "Deny", kind: "reject_once"),
        PermissionOption(id: "always", name: "Always Allow", kind: "allow_always"),
    ]

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            caption("A refused send, above the composer field")
            AgentFailureLine(
                sentence: "Your message wasn’t sent. This device can only look at this runner. Changing anything needs control, which is granted from a device that already has it.",
                onRetry: {}, onDismiss: {})
            caption("A send that may have landed: no Try Again")
            AgentFailureLine(sentence: AgentStream.sendMayNotHaveLanded, onRetry: nil, onDismiss: {})
            caption("A refused answer, under the approval buttons")
            ApprovalControls(options: options, onChoose: { _ in })
                .environment(
                    \.approvalState,
                    ApprovalState(
                        sending: false,
                        failure: "Your answer may not have reached the runner. Try again.",
                        onRetry: {}))
            caption("A chat that isn’t updating, in the composer's activity line")
            HStack(spacing: 5) {
                StatusGlyph(status: .lost)
                Text("This chat isn’t updating. Trying again…")
            }
            .font(.caption)
        }
        .padding(20)
        .frame(width: 460, alignment: .leading)
    }

    private func caption(_ text: String) -> some View {
        Text(text.uppercased())
            .font(.system(size: 9, weight: .semibold))
            .foregroundStyle(.tertiary)
    }
}
