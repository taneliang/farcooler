import AgentKit
import AppKit
import SwiftUI
import TestWindows
import Testing

@testable import Far_Cooler

/// Whether members share the runner's logins, per agent (ov-652): the sheet's
/// switches go through the CLI, and only an admin's turn.
@MainActor
@Suite(.serialized)
struct MemberLoginsSettingsTests {
    /// `farcooler --json member list` on a shared runner, its switches as given.
    static func list(you: String, codex: Bool = false, mayChange: Bool, long: Bool = false) -> String {
        let alice = long ? "Alexandra Montgomery-Fitzgerald of the Platform Team" : "Alice Chen"
        return #"{"members":[{"handle":"alice","name":"\#(alice)","devices":[],"reps":0,"rep_panes":[]},"#
            + #"{"handle":"bob","name":"Bob","devices":[],"reps":0,"rep_panes":[]}],"#
            + #""claude_account":"platform-team-shared-account@example-corporation.com","you":"\#(you)","#
            + #""own_logins":{"claude":false,"codex":\#(codex),"cursor":false,"may_change":\#(mayChange)}}"#
    }

    final class Calls {
        var made: [[String]] = []
    }

    static func store(you: String, mayChange: Bool, calls: Calls = Calls(), long: Bool = false) -> RunnerSettingsStore {
        let store = RunnerSettingsStore(target: "box")
        store.cliForTesting = { args in
            calls.made.append(args)
            if args.suffix(2) == ["member", "list"] {
                return CLI.Result(ok: true, output: list(you: you, mayChange: mayChange, long: long), errors: "")
            }
            if let at = args.firstIndex(of: "own-login") {
                if !mayChange {
                    return CLI.Result(ok: false, output: "", errors: "error: only a runner admin or the owner can do that\ncode: scope-denied")
                }
                return CLI.Result(ok: true, output: list(you: you, codex: args[at + 2] == "on", mayChange: true), errors: "")
            }
            if args.contains("settings") { return CLI.Result(ok: true, output: #"{"branchPrefix":""}"#, errors: "") }
            return CLI.Result(ok: true, output: #"{"themes":[]}"#, errors: "")
        }
        return store
    }

    @Test("A switch goes through the CLI and the sheet reads the switches it answers")
    func sets() async throws {
        let calls = Calls()
        let store = Self.store(you: "", mayChange: true, calls: calls)
        await store.load()
        #expect(store.people?.ownLogins == MemberOwnLogins(mayChange: true))
        await store.setOwnLogin(.codex, true)
        #expect(calls.made.last == ["--host", "box", "--json", "member", "own-login", "codex", "on"])
        #expect(store.people?.ownLogins?.codex == true)
        #expect(store.failure == nil)
    }

    @Test("A refusal is said, and the switches stay as they were")
    func refused() async throws {
        let store = Self.store(you: "bob", mayChange: false)
        await store.load()
        await store.setOwnLogin(.codex, true)
        #expect(store.people?.ownLogins?.codex == false)
        #expect(store.failure == "only a runner admin or the owner can do that")
    }

    /// The section in a grouped form `width` points wide.
    static func section(_ logins: MemberOwnLogins, owner: Bool, width: CGFloat, scheme: ColorScheme) -> NSHostingView<some View> {
        let view = Form {
            MemberLoginsSection(logins: logins, owner: owner, set: logins.mayChange ? { _, _ in } : nil)
        }
        .formStyle(.grouped)
        .frame(width: width, height: 440)
        // style-exempt: the system window background, as the sheet's window has
        .background(Color(nsColor: .windowBackgroundColor))
        .environment(\.colorScheme, scheme)
        let host = NSHostingView(rootView: view)
        host.appearance = NSAppearance(named: scheme == .dark ? .darkAqua : .aqua)
        host.frame = CGRect(x: 0, y: 0, width: width, height: 440)
        return host
    }

    /// The section, as an admin and read-only, narrow and wide, light and
    /// dark, and the whole sheet on a runner with a long name, into
    /// `FARCOOLER_CAPTURE_OUT`. Opt-in.
    @Test("Captures of Member Logins", .enabled(if: ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"] != nil))
    func captures() async throws {
        let out = URL(fileURLWithPath: ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"]!)
        try FileManager.default.createDirectory(at: out, withIntermediateDirectories: true)
        let cases: [(String, MemberOwnLogins, Bool)] = [
            ("admin", MemberOwnLogins(claude: false, codex: true, cursor: false, mayChange: true), true),
            ("readonly", MemberOwnLogins(claude: true, codex: false, cursor: false, mayChange: false), false),
        ]
        for (who, logins, owner) in cases {
            for (size, width) in [("narrow", CGFloat(360)), ("wide", CGFloat(720))] {
                for scheme in [ColorScheme.light, .dark] {
                    let host = Self.section(logins, owner: owner, width: width, scheme: scheme)
                    let window = NSWindow(
                        contentRect: NSRect(x: -4000, y: -4000, width: width, height: 440), styleMask: [.borderless],
                        backing: .buffered, defer: false)
                    window.isReleasedWhenClosed = false
                    window.contentView = host
                    TestWindow.show(window)
                    await Settle.still(host, reading: { "\(host.fittingSize)" })
                    let rep = try #require(host.lookBitmap(scale: 2))
                    try #require(rep.representation(using: .png, properties: [:]))
                        .write(to: out.appendingPathComponent("mac-member-logins-\(who)-\(size)-\(scheme == .dark ? "dark" : "light").png"))
                    window.close()
                }
            }
        }
        // The whole sheet, as the owner, on a runner with a long name.
        let store = Self.store(you: "", mayChange: true, long: true)
        let sheet = RunnerSettingsSheet(name: "Platform Team’s Shared Build Runner in the Basement Rack", target: "box", store: store)
            // style-exempt: the system window background, as the sheet's window has
            .background(Color(nsColor: .windowBackgroundColor))
        let host = NSHostingView(rootView: sheet)
        let size = SettingsSheetSize.runner
        let window = NSWindow(
            contentRect: NSRect(x: -4000, y: -4000, width: size.width, height: size.height), styleMask: [.borderless],
            backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = host
        TestWindow.show(window)
        await Settle.until { host.layoutSubtreeIfNeeded(); return store.people != nil && !store.loading }
        await Settle.still(host, reading: { "\(store.people?.ownLogins != nil)" })
        let rep = try #require(host.lookBitmap(scale: 2))
        try #require(rep.representation(using: .png, properties: [:])).write(to: out.appendingPathComponent("mac-member-logins-sheet-light.png"))
        window.close()
    }
}
