import AgentKit
import AppKit
import SwiftUI
import TestWindows
import Testing

@testable import Far_Cooler

/// Who added each device on a runner, and Revoke's two choices (ov-855): the
/// Mac reads `client list --json` and sends `client revoke`, with
/// `--with-added` only for "Revoke This and the N It Added".
@MainActor
@Suite(.serialized)
struct EnrolledDevicesSettingsTests {
    static let list = """
        {"clients":[
          {"clientId":"mac-1","label":"Studio Mac","scope":"host_admin","foreign":false,"addedBy":""},
          {"clientId":"phone-1","label":"Alexandra’s iPhone","scope":"control","foreign":false,"addedBy":"mac-1"},
          {"clientId":"web-1","label":"Alexandra’s Browser","scope":"read","foreign":false,"addedBy":"phone-1"}
        ]}
        """

    /// A store whose `client list` is `list` and whose revokes are recorded.
    final class Sent { var revokes: [[String]] = [] }

    static func store(sent: Sent, revokeAnswer: String? = nil) -> RunnerSettingsStore {
        let store = RunnerSettingsStore(target: "box")
        store.cliForTesting = { args in
            if args.contains("revoke") {
                sent.revokes.append(args)
                guard let revokeAnswer else { return CLI.Result(ok: false, output: "", errors: "refused") }
                return CLI.Result(ok: true, output: revokeAnswer, errors: "")
            }
            if args.suffix(2) == ["client", "list"] { return CLI.Result(ok: true, output: list, errors: "") }
            if args.contains("settings") { return CLI.Result(ok: true, output: #"{"branchPrefix":""}"#, errors: "") }
            return CLI.Result(ok: true, output: #"{"themes":[]}"#, errors: "")
        }
        return store
    }

    @Test("The list is read and each device names who added it")
    func listed() async throws {
        let store = Self.store(sent: Sent())
        await store.loadEnrolled()
        let enrolled = try #require(store.enrolled)
        let web = try #require(enrolled.devices.first { $0.clientId == "web-1" })
        #expect(enrolled.addedByLine(for: web) == "Added by Alexandra’s iPhone")
    }

    @Test("Revoke This sends no --with-added; the second choice does")
    func flag() async throws {
        let sent = Sent()
        let store = Self.store(sent: sent, revokeAnswer: Self.list)
        await store.loadEnrolled()
        let phone = try #require(store.enrolled?.devices.first { $0.clientId == "phone-1" })
        await store.revoke(phone, withAdded: false)
        await store.revoke(phone, withAdded: true)
        #expect(sent.revokes.count == 2)
        #expect(!sent.revokes[0].contains("--with-added"), "\(sent.revokes[0])")
        #expect(sent.revokes[1].contains("--with-added"), "\(sent.revokes[1])")
        #expect(sent.revokes[1].contains("phone-1"))
        #expect(sent.revokes[1].contains("--host") && sent.revokes[1].contains("box"))
    }

    @Test("A refused revoke keeps the list and says so in plain words")
    func refused() async throws {
        let store = Self.store(sent: Sent())
        await store.loadEnrolled()
        let before = store.enrolled
        await store.revoke(try #require(before?.devices.first), withAdded: false)
        #expect(store.enrolled == before)
        #expect(store.revokeFailure == EnrolledDevices.notRevoked)
        #expect(store.revokeFailure?.contains("refused") == false)
    }

    /// The real sheet, drawn with the list read.
    static func drawn(scheme: ColorScheme = .light) async -> (Set<String>, NSWindow) {
        let asked = RunnerPeopleSettingsTests.Answered()
        let store = store(sent: Sent())
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
            return store.enrolled != nil && !store.loading
        }
        asked.memberList = true
        await Settle.still(host, reading: { seen.views.keys.sorted().joined(separator: ",") + "\(seen.views.values)" })
        return (Set(seen.views.keys), window)
    }

    @Test("Runner Settings draws the devices section when the runner lists some")
    func drawnSection() async {
        let (ids, window) = await Self.drawn()
        window.close()
        #expect(ids.contains("runner-enrolled-devices"), "\(ids)")
    }

    @Test("Captures of the devices section", .enabled(if: ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"] != nil))
    func captures() async throws {
        let out = URL(fileURLWithPath: ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"]!)
        try FileManager.default.createDirectory(at: out, withIntermediateDirectories: true)
        for scheme in [ColorScheme.light, .dark] {
            let (ids, window) = await Self.drawn(scheme: scheme)
            try await Capture.keep(
                window, to: out.appendingPathComponent("mac-enrolled-devices-\(scheme == .dark ? "dark" : "light").png"),
                expecting: "the devices section", until: { ids.contains("runner-enrolled-devices") })
            window.close()
        }
    }
}
