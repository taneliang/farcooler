#if DEBUG
import SwiftUI

/// The owner's phone adding a browser to a runner it reaches only through
/// Tailcat (ov-828), for the UI suite (`-grant-browser`).
///
/// The store is the app's own, and so is everything it decides: it reads a
/// browser's offer built by the core, lists Cosmo as the Add a Device screen
/// does, and confirms. What's canned is the Face ID tap, the directory
/// certification, and the runner's answer to `client.enroll`:
///
///   -grant-browser               Cosmo refuses the grant (`scope-denied`),
///                                as a runner from before ov-828 does
///   -grant-browser-unreached     this phone holds no session to Cosmo
struct GrantBrowserHarness: View {
    static var isRequested: Bool { CommandLine.arguments.contains("-grant-browser") }

    static let browserKey =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIFJhEUXBcOjzsos5aFyw6hfDrERWHei9akIudYJGG2gR browser"
    static let nodeKey = "3q2-7wAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
    static let cosmo = Runner(
        label: "Cosmo", reach: .tailcat(token: String(repeating: "t", count: 184)), user: "e-liang",
        fingerprint: "SHA256:iDqoxaySm9gzxtvLrNXXpM5PimPLeBknaaNj0Rg7vz4")

    @StateObject private var store = CeremonyStore(
        account: "user_owner", accountEmail: "owner@example.com", deviceName: "Owner’s iPhone")
    @State private var reply = ""

    var body: some View {
        NavigationStack {
            List {
                ForEach(store.rows) { row in
                    VStack(alignment: .leading, spacing: 2) {
                        Text(row.label)
                        Text(row.detail).font(.caption).foregroundStyle(.secondary)
                            .accessibilityIdentifier("grant-row-detail")
                    }
                }
                if let note = store.pendingNote {
                    Text(note).accessibilityIdentifier("grant-note")
                }
                if !reply.isEmpty {
                    Text(reply).font(.caption2).accessibilityIdentifier("grant-reply")
                }
            }
            .navigationTitle("Add a Device")
            .navigationBarTitleDisplayMode(.inline)
        }
        .task { await run() }
    }

    private func run() async {
        guard
            case .payload(let offer) = CeremonyCore.offer(
                name: "Chrome on Mac", account: "user_owner", keyA: Self.browserKey, nodeKey: Self.nodeKey)
        else { return }
        store.read(String(decoding: offer, as: UTF8.self), runners: [Self.cosmo], grantingFrom: Self.cosmo)
        store.confirmTap = { _ in .confirmed }
        store.certify = { _, _ in }
        let unreached = CommandLine.arguments.contains("-grant-browser-unreached")
        store.enroller = { _, _, _, _, runners in
            unreached ? [:] : Dictionary(uniqueKeysWithValues: runners.map { ($0.id, Connection.Enrollment.notAllowed) })
        }
        await store.confirm()
        reply = store.code
    }
}
#endif
