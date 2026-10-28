#if DEBUG
import SwiftUI

/// The owner's phone adding a teammate's device (ov-854), for the UI suite
/// (`-teammate-add`).
///
/// The screen is the app's own `AddDeviceView` over its own `CeremonyStore`,
/// and so is everything the store decides: it reads a teammate's code built by
/// the core, asks the directory's answers, has the core vouch for the code,
/// and builds the reply. What's canned is the camera (the code is handed to the
/// store), the team directory, the Face ID tap, which runners this phone
/// administers, and the runner's answer to `teammate.enroll`. The enroller
/// records what it was asked, so a run leaves no key behind.
///
///   -teammate-add            Bob, of Platform, on a phone that administers
///                            build-box and only controls gpu-workstation
///   -teammate-add-long       a long email and long runner and device names
///   -teammate-add-stranger   the directory doesn't list Bob
///   -teammate-add-offline    the directory can't be asked
///   -teammate-add-refused    build-box's Far Cooler is too old to bind
struct TeammateAddHarness: View {
    static var isRequested: Bool { CommandLine.arguments.contains("-teammate-add") }
    private static func has(_ flag: String) -> Bool { CommandLine.arguments.contains(flag) }

    static let key = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIB1iLbeqDzK4CDeUC3t+ffVPDI9Gk+sBwIZqJZW1NfS5 bob"

    private static var email: String {
        has("-teammate-add-long") ? "bartholomew.montgomery-fitzgerald@platform-infrastructure.example.com" : "bob.smith@example.com"
    }

    static let buildBox = Runner(
        label: "build-box", reach: .direct(host: "build-box.example.com", port: 22), user: "alice",
        fingerprint: "SHA256:iDqoxaySm9gzxtvLrNXXpM5PimPLeBknaaNj0Rg7vz4")
    static let gpu = Runner(
        label: "gpu-workstation", reach: .direct(host: "gpu.example.com", port: 22), user: "alice",
        fingerprint: "SHA256:iDqoxaySm9gzxtvLrNXXpM5PimPLeBknaaNj0Rg7vz4")
    static let platform = Runner(
        label: "Platform Builds, EU West (Shared)", reach: .direct(host: "eu-west.platform.example.com", port: 22),
        user: "alice", fingerprint: "SHA256:iDqoxaySm9gzxtvLrNXXpM5PimPLeBknaaNj0Rg7vz4")

    @StateObject private var store = CeremonyStore(
        account: "user_alice", accountEmail: "alice@example.com", deviceName: "Alice’s iPhone")
    @StateObject private var runners = RunnerStore()
    /// What the enroller was asked, in one line, for the suite to read.
    @State private var asked = ""

    var body: some View {
        NavigationStack {
            AddDeviceView(runners: runners, store: store)
        }
        .overlay(alignment: .bottom) {
            Color.clear
                .frame(width: 1, height: 1)
                .accessibilityElement()
                .accessibilityIdentifier("teammate-asked")
                .accessibilityValue(asked)
        }
        .task { await prepare() }
    }

    private func prepare() async {
        let offerer = Self.has("-teammate-add-stranger") ? "user_carol" : "user_bob"
        guard
            case .payload(let offer) = CeremonyCore.offer(
                name: Self.has("-teammate-add-long") ? "Bartholomew’s Work iPhone (the one with the cracked screen)" : "Bob’s iPhone",
                account: offerer, keyA: Self.key, nodeKey: nil)
        else { return }
        store.teamDirectory = { Self.has("-teammate-add-offline") ? nil : Self.directory }
        store.confirmTap = { _ in .confirmed }
        store.mayAddTeammate = { $0.id != Self.gpu.id }
        store.teammateEnroller = { _, _, _, _, teammate, wanted in
            asked =
                "scope=\(teammate.scope.rawValue) handle=\(teammate.handle) name=\(teammate.name) "
                + "account=\(teammate.who.account) runners=\(wanted.map(\.label).joined(separator: ","))"
            let outcome: TeammateAddOutcome = Self.has("-teammate-add-refused") ? .tooOld : .written(tunnel: "")
            return Dictionary(uniqueKeysWithValues: wanted.map { ($0.id, outcome) })
        }
        let runnersOnPhone = Self.has("-teammate-add-long") ? [Self.platform, Self.gpu] : [Self.buildBox, Self.gpu]
        store.mayAddTeammate = { $0.id != Self.gpu.id }
        store.read(String(decoding: offer, as: UTF8.self), runners: runnersOnPhone, grantingFrom: nil)
    }

    /// The relay's answers: Alice is an admin of Platform, and Bob is in it
    /// unless this launch says he isn't.
    private static var directory: Data {
        let members: [[String: Any]] =
            [["user": "user_alice", "email": "alice@example.com", "role": "admin"]]
            + (has("-teammate-add-stranger") ? [] : [["user": "user_bob", "email": email, "role": "member"]])
        let object: [String: Any] = [
            "me": [
                "user": ["id": "user_alice", "email": "alice@example.com"],
                "teams": [["id": "t-platform", "name": "Platform", "personal": false, "role": "admin"]],
            ],
            "rosters": [["team": ["id": "t-platform", "name": "Platform", "personal": false], "members": members]],
        ]
        return (try? JSONSerialization.data(withJSONObject: object)) ?? Data()
    }
}
#endif
