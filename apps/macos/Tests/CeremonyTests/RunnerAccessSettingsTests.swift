import AgentKit
import AppKit
import SwiftUI
import TestWindows
import Testing

@testable import Far_Cooler

/// The People section's Access (ov-856, slice 4) on the Mac: each person with
/// their Access and devices, the two sections that appear when they have
/// something, and the changes the runner's own socket makes through the CLI.
@MainActor
@Suite(.serialized)
struct RunnerAccessSettingsTests {
    /// `farcooler --json member list` from a runner that records access:
    /// Cosmo (Member) with an iPhone that added an iPad held at Viewer; Dan
    /// (Admin) whose phone waits for an approval and whose Mac has a shell; a
    /// legacy line to check.
    nonisolated static let members =
        #"{"members":[{"handle":"cosmo","name":"Cosmo","devices":["phone-c","pad-c"],"reps":0,"rep_panes":[],"access":"member"},"#
        + #"{"handle":"dan","name":"Dan","devices":["phone-d","mac-d"],"reps":0,"rep_panes":[],"access":"admin"}],"#
        + #""claude_account":"team@example.com","you":"","device_access":["#
        + #"{"client_id":"mac-o","person":"","access":"owner","cap":"","check":false,"step_up":"trusted","step_up_code":"","step_up_at":5},"#
        + #"{"client_id":"legacy-t","person":"","access":"owner","cap":"member","check":true,"step_up":"","step_up_code":"","step_up_at":0},"#
        + #"{"client_id":"phone-c","person":"cosmo","access":"member","cap":"","check":false,"step_up":"trusted","step_up_code":"","step_up_at":5},"#
        + #"{"client_id":"pad-c","person":"cosmo","access":"member","cap":"viewer","check":false,"step_up":"","step_up_code":"","step_up_at":0},"#
        + #"{"client_id":"phone-d","person":"dan","access":"admin","cap":"","check":false,"step_up":"proposed","step_up_code":"418207","step_up_at":9},"#
        + #"{"client_id":"mac-d","person":"dan","access":"admin","cap":"","check":false,"step_up":"trusted","step_up_code":"","step_up_at":5}]}"#

    nonisolated static let clients =
        #"{"clients":[{"clientId":"phone-c","label":"farcooler-Cosmos-iPhone-dddd4444","shellAccess":false,"foreign":false,"addedBy":""},"#
        + #"{"clientId":"pad-c","label":"farcooler-Cosmos-iPad-eeee5555","shellAccess":false,"foreign":false,"addedBy":"phone-c"},"#
        + #"{"clientId":"phone-d","label":"farcooler-Dans-iPhone-ffff6666","shellAccess":false,"foreign":false,"addedBy":""},"#
        + #"{"clientId":"mac-d","label":"farcooler-Dans-Mac-gggg7777","shellAccess":false,"foreign":false,"addedBy":""},"#
        + #"{"clientId":"mac-d","label":"farcooler-shell-Dans-Mac-gggg7777.mac-d","shellAccess":true,"foreign":false,"addedBy":""},"#
        + #"{"clientId":"legacy-t","label":"farcooler-Old-iPad-cccc3333","shellAccess":false,"foreign":false,"addedBy":""}]}"#

    /// Every call the store made to the CLI, in order, and how it answers.
    final class Calls {
        var made: [[String]] = []
        var refuse: String?
    }

    static func store(members: String = members, calls: Calls = Calls()) -> RunnerSettingsStore {
        let store = RunnerSettingsStore(target: "box")
        store.cliForTesting = { args in
            calls.made.append(args)
            if args.suffix(2) == ["member", "list"] { return CLI.Result(ok: true, output: members, errors: "") }
            if args.suffix(2) == ["client", "list"] { return CLI.Result(ok: true, output: clients, errors: "") }
            if args.contains("people") || args.contains("revoke") {
                if let refuse = calls.refuse { return CLI.Result(ok: false, output: "", errors: refuse) }
                return CLI.Result(ok: true, output: "{}", errors: "")
            }
            if args.contains("settings") { return CLI.Result(ok: true, output: #"{"branchPrefix":""}"#, errors: "") }
            return CLI.Result(ok: true, output: #"{"themes":[]}"#, errors: "")
        }
        return store
    }

    @Test("The store reads each person's Access and the devices under them")
    func reads() async throws {
        let store = Self.store()
        await store.loadPeople()
        let access = try #require(store.access)
        #expect(access.people.map(\.access) == [.owner, .member, .admin])
        let cosmo = try #require(access.people.first { $0.handle == "cosmo" })
        #expect(cosmo.devices.map(\.name) == ["Cosmos-iPhone", "Cosmos-iPad"])
        #expect(cosmo.devices[1].addedBy == "Added by Cosmos-iPhone" && cosmo.devices[1].viewerOnThisDevice)
        #expect(access.toCheck.map(\.id) == ["legacy-t"] && access.waiting.map(\.id) == ["phone-d"])
        #expect(access.people.first { $0.handle == "dan" }?.devices.map(\.hasShell) == [false, true])
    }

    @Test("A runner that hasn't recorded access shows none")
    func oldRunner() async {
        let store = Self.store(members: #"{"members":[{"handle":"a","name":"A","devices":[]}],"claude_account":"","you":""}"#)
        await store.loadPeople()
        #expect(store.access == nil)
    }

    @Test("Every change is the CLI's command on the runner's socket, and the list is read again")
    func changes() async throws {
        let calls = Calls()
        let store = Self.store(calls: calls)
        await store.loadPeople()
        let access = try #require(store.access)
        let actions = store.accessActions()
        let cosmo = try #require(access.people.first { $0.handle == "cosmo" })
        await actions.removeDevice?(cosmo.devices[0], true)
        await actions.limitToViewer?(cosmo.devices[0])
        await actions.setAccess?(cosmo, .viewer)
        await actions.approve?(access.waiting[0])
        let sent = calls.made.filter { $0.contains("people") || $0.contains("revoke") }.map { Array($0.dropFirst(3)) }
        #expect(sent == [
            ["client", "revoke", "phone-c", "--with-added"],
            ["people", "limit", "phone-c", "viewer"],
            ["people", "access", "cosmo", "viewer"],
            ["people", "approve", "phone-d", "418207"],
        ], "\(sent)")
        #expect(calls.made.filter { $0.suffix(2) == ["member", "list"] }.count == 5, "read again after each change")
    }

    @Test("A refusal is a sentence, not the runner's word")
    func refusal() async throws {
        let calls = Calls()
        let store = Self.store(calls: calls)
        await store.loadPeople()
        calls.refuse = "error: socket_only"
        let access = try #require(store.access)
        await store.accessActions().approve?(access.waiting[0])
        #expect(store.failure == "You can’t change that from this Mac.")
    }

    // MARK: - A Mac that isn't the runner (ov-856)

    /// The client core, as a paired Mac's phone approval calls it: it answers the
    /// lists, an approval that is pending for `pendingLooks` looks and then
    /// granted, and the approved action, and remembers every call.
    final class PairedCalls: StepUpCalling, @unchecked Sendable {
        var made: [(method: String, args: [String: Any])] = []
        var refuse: String?
        var pendingLooks = 2
        /// What the store's `waitingOnPhone` read at each look.
        var waitingAtLook: [Bool] = []
        var waiting: (() async -> Bool)?
        /// Answers every look with this state instead of waiting for the grant.
        var always: String?
        struct Refusal: Error { let word: String }
        func call(_ method: String, _ args: [String: Any]) async throws -> Data {
            made.append((method, args))
            if let refuse, method == "approval.ask" { throw Refusal(word: refuse) }
            switch method {
            case "member.list":
                return Data(RunnerAccessSettingsTests.members.replacingOccurrences(of: "\"you\":\"\"", with: "\"you\":\"dan\"").utf8)
            case "client.list": return Data(RunnerAccessSettingsTests.clients.utf8)
            case "approval.ask": return Data(#"{"id":"a-1","state":"pending","expiresAt":9}"#.utf8)
            case "approval.status":
                waitingAtLook.append(await waiting?() ?? false)
                let granted = made.filter { $0.method == "approval.status" }.count > pendingLooks
                let state = always ?? (granted ? "granted" : "pending")
                return Data(#"{"id":"a-1","state":"\#(state)","expiresAt":9}"#.utf8)
            default: return Data("{}".utf8)
            }
        }
        func refusalWord(_ error: Error) -> String? { (error as? Refusal)?.word }
    }

    /// A store for a runner the owner's ssh can't reach, but this Mac is paired with.
    static func pairedStore(_ paired: PairedCalls) -> RunnerSettingsStore {
        let store = RunnerSettingsStore(target: "box")
        store.cliForTesting = { _ in CLI.Result(ok: false, output: "", errors: "Permission denied (publickey).") }
        store.stepUpOpener = { _ in
            RemoteStepUp(approval: PhoneApproval(core: paired, pause: 1_000_000, looks: 20), core: paired, clientID: "mac-d")
        }
        paired.waiting = { [weak store] in await MainActor.run { store?.waitingOnPhone ?? false } }
        return store
    }

    @Test("A Mac with no shell on the runner reads the people as its paired device")
    func pairedRead() async throws {
        let store = Self.pairedStore(PairedCalls())
        await store.loadPeople()
        let access = try #require(store.access)
        #expect(access.yourAccess == .admin)
    }

    @Test("The Mac waits for the iPhone, then sends the very request it asked about")
    func pairedActionWaitsThenCompletes() async throws {
        let paired = PairedCalls()
        let store = Self.pairedStore(paired)
        await store.loadPeople()
        let access = try #require(store.access)
        let cosmo = try #require(access.people.first { $0.handle == "cosmo" })
        #expect(store.waitingOnPhone == false)
        await store.accessActions().limitToViewer?(cosmo.devices[0])
        let methods = paired.made.map(\.method).filter { $0.hasPrefix("approval") }
        #expect(methods == ["approval.ask", "approval.status", "approval.status", "approval.status", "approval.finish"], "\(methods)")
        let asked = try #require(paired.made.first { $0.method == "approval.ask" })
        #expect(asked.args["method"] as? String == "client.set_cap")
        #expect((asked.args["args"] as? [String: Any])?["clientId"] as? String == "phone-c")
        #expect((asked.args["args"] as? [String: Any])?["cap"] as? String == "viewer")
        #expect(paired.waitingAtLook.allSatisfy { $0 } && !paired.waitingAtLook.isEmpty, "the Mac said it was waiting while it waited")
        #expect(paired.made.last?.method == "member.list" || paired.made.last?.method == "client.list", "and read the people again after")
        try await Task.sleep(nanoseconds: 50_000_000)
        #expect(store.waitingOnPhone == false && store.failure == nil)
    }

    @Test("This Mac makes no key of its own, and asks no one to approve one")
    func pairedMacKeepsNoKey() async throws {
        let store = Self.pairedStore(PairedCalls())
        await store.loadPeople()
        let actions = store.accessActions()
        #expect(actions.asksPhone && actions.allowAdminActions == nil)
    }

    @Test("An iPhone with no key, or one that said no, is a sentence on the Mac")
    func pairedRefusal() async throws {
        let paired = PairedCalls()
        let store = Self.pairedStore(paired)
        await store.loadPeople()
        paired.refuse = "no_approver"
        let cosmo = try #require(store.access?.people.first { $0.handle == "cosmo" })
        await store.accessActions().setAccess?(cosmo, .member)
        #expect(store.failure == "Open Far Cooler on your iPhone and allow admin actions there first.", "\(store.failure ?? "")")
        #expect(paired.made.contains { $0.method == "approval.finish" } == false, "nothing was sent")
    }

    @Test("An iPhone that says no sends nothing, and the Mac says so")
    func pairedDenied() async throws {
        let paired = PairedCalls()
        paired.always = "denied"
        let store = Self.pairedStore(paired)
        await store.loadPeople()
        let cosmo = try #require(store.access?.people.first { $0.handle == "cosmo" })
        await store.accessActions().setAccess?(cosmo, .viewer)
        #expect(store.failure == "Your iPhone didn’t approve it.", "\(store.failure ?? "")")
        #expect(paired.made.contains { $0.method == "approval.finish" } == false)
        #expect(store.waitingOnPhone == false)
    }

    // MARK: - Drawn

    final class Seen {
        var views: [String: CGRect] = [:]
    }

    struct Probed: View {
        let store: RunnerSettingsStore
        let seen: Seen
        var body: some View {
            RunnerSettingsSheet(name: "box", target: "box", store: store)
                .background(Color(nsColor: .windowBackgroundColor))  // style-exempt: the system window background, as the sheet's window has
                .environment(\.gridProbing, true)
                .overlayPreferenceValue(ProbedViewsKey.self) { probed in
                    GeometryReader { proxy in
                        let _ = seen.views = Dictionary(probed.map { ($0.id, proxy[$0.bounds]) }, uniquingKeysWith: { first, _ in first })
                        Color.clear
                    }
                }
        }
    }

    static func drawn(scheme: ColorScheme = .light) async -> (RunnerSettingsStore, NSWindow) {
        let store = Self.store()
        let seen = Seen()
        let host = NSHostingView(rootView: Probed(store: store, seen: seen).environment(\.colorScheme, scheme))
        let size = SettingsSheetSize.runner
        let window = NSWindow(
            contentRect: NSRect(x: -4000, y: -4000, width: size.width, height: size.height), styleMask: [.borderless],
            backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.appearance = NSAppearance(named: scheme == .dark ? .darkAqua : .aqua)
        window.contentView = host
        TestWindow.show(window)
        await Settle.until {
            host.layoutSubtreeIfNeeded()
            return store.access != nil && !store.loading
        }
        await Settle.still(host, reading: { "\(store.access?.people.count ?? 0)" })
        return (store, window)
    }

    /// The sheet as drawn, light and dark, into `FARCOOLER_CAPTURE_OUT`.
    /// Opt-in: a capture tool, never run in CI or a normal run.
    @Test("Captures of People with Access", .enabled(if: ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"] != nil))
    func captures() async throws {
        let out = URL(fileURLWithPath: ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"]!)
        try FileManager.default.createDirectory(at: out, withIntermediateDirectories: true)
        for scheme in [ColorScheme.light, .dark] {
            let (store, window) = await Self.drawn(scheme: scheme)
            let view = try #require(window.contentView)
            try await Capture.keep(
                view, to: out.appendingPathComponent("mac-people-access-\(scheme == .dark ? "dark" : "light").png"),
                expecting: "People with Access, devices and approvals", until: { store.access != nil })
            window.close()
        }
    }
}
