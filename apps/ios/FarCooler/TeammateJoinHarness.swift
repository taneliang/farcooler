#if DEBUG
import SwiftUI

/// A teammate's phone asking before it joins their runner admin's runners
/// (ov-525), for the UI suite (`-teammate-join`).
///
/// The store is the app's own, and so is everything it decides: it shows a
/// fixture key's offer (minting no node key), takes a reply the core built
/// for it, and that reply names who granted it. What's canned is the camera:
/// the reply is handed to ``CeremonyStore/takeReply(_:)`` rather than read off
/// a screen, and Join records what it would add rather than adding it, so a
/// run leaves no runner behind.
///
///   -teammate-join        one runner, from alice@example.com
///   -teammate-join-many   three runners
///   -teammate-join-long   long names: the email, and a runner's label
struct TeammateJoinHarness: View {
    static var isRequested: Bool { CommandLine.arguments.contains("-teammate-join") }
    private static func has(_ flag: String) -> Bool { CommandLine.arguments.contains(flag) }

    static let key = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIB1iLbeqDzK4CDeUC3t+ffVPDI9Gk+sBwIZqJZW1NfS5 bob"

    @StateObject private var store = CeremonyStore(
        account: "user_bob", accountEmail: "bob@example.com", deviceName: "Bob’s iPhone")
    @State private var joined: [String]?
    @State private var cancelled = false

    var body: some View {
        NavigationStack {
            Group {
                if let joined {
                    Text("Joined \(joined.joined(separator: ", "))")
                        .accessibilityIdentifier("teammate-joined")
                } else if cancelled {
                    Text("Nothing joined").accessibilityIdentifier("teammate-cancelled")
                } else if store.phase == .confirmingJoin {
                    TeammateJoinPrompt(store: store) {
                        store.join()
                        joined = store.phase == .done ? store.granted.map(\.label) : []
                    } cancel: {
                        cancelled = true
                    }
                } else {
                    Text("Not asking: \(String(describing: store.phase))")
                        .accessibilityIdentifier("teammate-not-asking")
                }
            }
            .navigationTitle("Add This Device")
            .navigationBarTitleDisplayMode(.inline)
        }
        .task { prepare() }
    }

    private func prepare() {
        store.offeredNodeKey = { nil }
        store.showOffer(publicKey: Self.key)
        let long = Self.has("-teammate-join-long")
        let labels =
            Self.has("-teammate-join-many") || long
            ? (long
                ? ["Platform Builds, EU West (Shared)", "build-box", "gpu-workstation"]
                : ["build-box", "staging", "gpu-workstation"])
            : ["build-box"]
        let runners = labels.enumerated().map { index, label in
            CeremonyRunner(
                id: "runner-\(index)", label: label, alias: label, user: "alice",
                host_key: "SHA256:iDqoxaySm9gzxtvLrNXXpM5PimPLeBknaaNj0Rg7vz4",
                reach: .direct(host: "\(label).example.com", port: 22), pending: false)
        }
        guard case .payload(let reply) = CeremonyCore.reply(offer: Data(store.code.utf8), runners: runners),
            var object = try? JSONSerialization.jsonObject(with: reply) as? [String: Any]
        else { return }
        object["granted_by"] =
            long ? "alexandra.montgomery-fitzgerald@platform-infrastructure.example-corporation.com" : "alice@example.com"
        guard let named = try? JSONSerialization.data(withJSONObject: object) else { return }
        store.takeReply(String(decoding: named, as: UTF8.self))
    }
}
#endif
