import AgentKit
import AppKit
import SwiftUI
import TestWindows
import Testing

@testable import Far_Cooler

/// A shared runner's settings sheet says whose Claude account the work runs
/// on and lists its people with their reps (ov-508); a private one says
/// neither.
@MainActor
@Suite(.serialized)
struct RunnerPeopleSettingsTests {
    /// `farcooler --json member list`, as the CLI prints it.
    static let shared =
        #"{"members":[{"handle":"alice","name":"Alice Chen","devices":["phone-a"],"reps":1,"rep_panes":[{"terminal":"t1","workspace":"w-billing"}]},"#
        + #"{"handle":"bob","name":"Bob","devices":[],"reps":0,"rep_panes":[]}],"claude_account":"team@example.com","you":"bob"}"#
    static let alone = #"{"members":[],"claude_account":"team@example.com","you":""}"#

    /// A store whose CLI answers `member list` with `members`, and every
    /// other read with nothing.
    static func store(members: String) -> RunnerSettingsStore {
        let store = RunnerSettingsStore(target: "box")
        store.cliForTesting = { args in
            if args.suffix(2) == ["member", "list"] { return CLI.Result(ok: true, output: members, errors: "") }
            if args.contains("settings") { return CLI.Result(ok: true, output: #"{"branchPrefix":""}"#, errors: "") }
            return CLI.Result(ok: true, output: #"{"themes":[]}"#, errors: "")
        }
        return store
    }

    final class Seen {
        var views: [String: CGRect] = [:]
    }

    /// The real sheet, probed, drawn in a window off every screen.
    struct Probed: View {
        let store: RunnerSettingsStore
        let seen: Seen

        var body: some View {
            RunnerSettingsSheet(name: "box", target: "box", workspaceNames: ["w-billing": "Billing", "w-main": "Main", "w-long": "Customer Onboarding Experiments"], store: store)
                // A sheet's window draws this behind it; a borderless test
                // window doesn't. style-exempt: the system window background, as the sheet's window has
                .background(Color(nsColor: .windowBackgroundColor))
                .environment(\.gridProbing, true)
                .overlayPreferenceValue(ProbedViewsKey.self) { probed in
                    GeometryReader { proxy in
                        let _ = seen.views = Dictionary(
                            probed.map { ($0.id, proxy[$0.bounds]) }, uniquingKeysWith: { first, _ in first })
                        Color.clear
                    }
                }
        }
    }

    /// What the sheet drew once `members` was read, and the window, for a
    /// capture; the caller closes it.
    static func drawn(members: String, scheme: ColorScheme = .light) async -> (Set<String>, NSWindow) {
        let store = store(members: members)
        let seen = Seen()
        let host = NSHostingView(rootView: Probed(store: store, seen: seen).environment(\.colorScheme, scheme))
        let size = SettingsSheetSize.runner
        let window = NSWindow(
            contentRect: NSRect(x: -4000, y: -4000, width: size.width, height: size.height),
            styleMask: [.borderless], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.appearance = NSAppearance(named: scheme == .dark ? .darkAqua : .aqua)
        window.contentView = host
        TestWindow.show(window)
        // Polled, with an early return: the sheet reads on `.task`.
        for _ in 0..<300 {
            host.layoutSubtreeIfNeeded()
            if store.people != nil, !store.loading, !seen.views.isEmpty { break }
            try? await Task.sleep(for: .milliseconds(20))
        }
        for _ in 0..<5 {
            host.layoutSubtreeIfNeeded()
            try? await Task.sleep(for: .milliseconds(20))
        }
        return (Set(seen.views.keys), window)
    }

    @Test("A shared runner's sheet says whose account and lists each member")
    func sharedRunner() async {
        let (drawn, window) = await Self.drawn(members: Self.shared)
        window.close()
        #expect(drawn.contains("runner-people-account"), "\(drawn)")
        #expect(drawn.contains("runner-person-alice") && drawn.contains("runner-person-bob"), "\(drawn)")
    }

    /// Long names and several reps, for the captures.
    static let long =
        #"{"members":[{"handle":"alexandra-montgomery","name":"Alexandra Montgomery-Fitzgerald of the Platform Team","devices":["phone-a"],"reps":3,"#
        + #""rep_panes":[{"terminal":"t1","workspace":"w-billing"},{"terminal":"t2","workspace":"w-main"},{"terminal":"t3","workspace":"w-long"}]},"#
        + #"{"handle":"bob","name":"Bob","devices":[],"reps":0,"rep_panes":[]}],"claude_account":"platform-team-shared-account@example-corporation.com","you":"bob"}"#

    /// The sheet as drawn, light and dark, into `FARCOOLER_CAPTURE_OUT`.
    /// Opt-in: a capture tool, never run in CI or a normal run.
    @Test("Captures of the People section", .enabled(if: ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"] != nil))
    func captures() async throws {
        let out = URL(fileURLWithPath: ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"]!)
        try FileManager.default.createDirectory(at: out, withIntermediateDirectories: true)
        for (name, members) in [("short", Self.shared), ("long", Self.long)] {
            for scheme in [ColorScheme.light, .dark] {
                let (_, window) = await Self.drawn(members: members, scheme: scheme)
                let view = try #require(window.contentView)
                let rep = try #require(view.bitmapImageRepForCachingDisplay(in: view.bounds))
                view.cacheDisplay(in: view.bounds, to: rep)
                let png = try #require(rep.representation(using: .png, properties: [:]))
                try png.write(to: out.appendingPathComponent("mac-people-\(name)-\(scheme == .dark ? "dark" : "light").png"))
                window.close()
            }
        }
    }

    @Test("A runner nobody shares shows no People section")
    func privateRunner() async {
        let (drawn, window) = await Self.drawn(members: Self.alone)
        window.close()
        #expect(!drawn.contains("runner-people-account"), "\(drawn)")
    }
}
