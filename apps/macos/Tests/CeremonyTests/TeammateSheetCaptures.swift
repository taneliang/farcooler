import AgentKit
import AppKit
import SwiftUI
import TestWindows
import Testing

@testable import Far_Cooler

/// Opt-in (FARCOOLER_CAPTURE_OUT): the Add Device sheet confirming a
/// teammate's device (ov-525), with long names, and its two refusals, light
/// and dark. The production `AddDeviceView` over a store the real scan filled.
@MainActor
@Suite(.serialized, .enabled(if: ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"] != nil))
struct TeammateSheetCaptures {
    static let email = "alexandra.montgomery-fitzgerald@platform-infrastructure.example-corporation.com"

    static func directory(includingHer: Bool) -> Data {
        let members: [[String: Any]] =
            [["user": "user_bob", "email": "bob@example.com", "role": "admin"]]
            + (includingHer ? [["user": "user_alexandra", "email": email, "role": "member"]] : [])
        let object: [String: Any] = [
            "me": [
                "user": ["id": "user_bob", "email": "bob@example.com"],
                "teams": [[
                    "id": "t", "name": "Platform Infrastructure and Developer Experience", "personal": false,
                    "role": "admin",
                ]],
            ],
            "rosters": [["team": ["id": "t", "personal": false], "members": members]],
        ]
        return try! JSONSerialization.data(withJSONObject: object)
    }

    static func row(_ label: String, granted: Bool, note: String? = nil) -> CeremonyStore.RunnerRow {
        CeremonyStore.RunnerRow(
            runner: CeremonyRunner(
                id: "bob@\(label)", label: label, alias: label, user: "bob",
                host_key: "SHA256:iDqoxaySm9gzxtvLrNXXpM5PimPLeBknaaNj0Rg7vz4",
                reach: .direct(host: "\(label).example-corporation.com", port: 22), pending: true),
            granted: granted, note: note)
    }

    func store(directory: Data?) async throws -> CeremonyStore {
        let code = try #require(
            CeremonyFFI.offer(
                name: "Alexandra’s iPhone 17 Pro Max (Work)", account: "user_alexandra",
                keyA: TeammateCeremonyTests.keyA, keyB: nil))
        let store = CeremonyStore()
        store.teamDirectory = { directory }
        let rows = [
            Self.row("Bob’s MacBook Pro", granted: true, note: "this Mac"),
            Self.row("platform-build-runner-eu-west-1", granted: false),
        ]
        store.scan(code, at: Date(), account: "user_bob", email: "bob@example.com", runners: rows)
        await store.teamCheck?.value
        return store
    }

    static func isConfirming(_ phase: CeremonyStore.Phase) -> Bool { if case .confirming = phase { true } else { false } }
    static func isRefused(_ phase: CeremonyStore.Phase) -> Bool { if case .refused = phase { true } else { false } }

    func shoot(_ store: CeremonyStore, _ name: String, to out: URL, expecting: String, phase: @MainActor (CeremonyStore.Phase) -> Bool) async throws {
        let reached = { @MainActor in phase(store.phase) }
        try await Capture.wait(expecting: expecting, until: reached)
        for scheme in [ColorScheme.light, .dark] {
            let host = NSHostingView(rootView: AddDeviceView(drawing: store).environment(\.colorScheme, scheme)
                // style-exempt: the system window background, as the sheet's window has
                .background(Color(nsColor: .windowBackgroundColor)))
            let size = host.fittingSize
            let window = NSWindow(
                contentRect: NSRect(x: -4000, y: -4000, width: size.width, height: size.height),
                styleMask: [.borderless], backing: .buffered, defer: false)
            window.isReleasedWhenClosed = false
            window.appearance = NSAppearance(named: scheme == .dark ? .darkAqua : .aqua)
            window.contentView = host
            TestWindow.show(window)
            for _ in 0..<15 {
                host.layoutSubtreeIfNeeded()
                try? await Task.sleep(for: .milliseconds(20))
            }
            // Sized again once it has drawn: the key opens on appear.
            window.setContentSize(host.fittingSize)
            for _ in 0..<5 {
                host.layoutSubtreeIfNeeded()
                try? await Task.sleep(for: .milliseconds(20))
            }
            try await Capture.keep(
                host, to: out.appendingPathComponent("mac-\(name)-\(scheme == .dark ? "dark" : "light").png"),
                expecting: expecting, until: reached)
            window.close()
        }
    }

    @Test func theTeammateSheetAndItsRefusals() async throws {
        let out = URL(fileURLWithPath: ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"]!)
        try FileManager.default.createDirectory(at: out, withIntermediateDirectories: true)
        let confirming = try await store(directory: Self.directory(includingHer: true))
        // As an admin leaves it: the shared build runner ticked by hand.
        confirming.setGrant(true, for: "bob@platform-build-runner-eu-west-1")
        try await shoot(confirming, "teammate-confirm", to: out, expecting: "the confirmation, her runners ticked", phase: Self.isConfirming)
        confirming.setTeammate { $0.handle = "Alexandra M" }
        try await shoot(confirming, "teammate-confirm-bad-handle", to: out, expecting: "the confirmation with a handle that was refused", phase: Self.isConfirming)
        try await shoot(try await store(directory: Self.directory(includingHer: false)), "teammate-not-on-team", to: out, expecting: "the refusal for someone not on the team", phase: Self.isRefused)
        try await shoot(try await store(directory: nil), "teammate-directory-unreachable", to: out, expecting: "the refusal with the directory unreachable", phase: Self.isRefused)
    }
}
