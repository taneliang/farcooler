import AgentKit
import SwiftUI

/// Settings › Devices › Conversation View (ov-408): each remote runner this
/// Mac's conversation key is paired with, or why it isn't, with the one
/// action that changes it. Unpair takes the key off the runner
/// (`client revoke`); Pair Again writes it back. A key removed on the runner
/// is never put back without this.
struct ConversationPairingSection: View {
    @ObservedObject private var native: NativeAgents
    @ObservedObject private var pairing: RemotePairing
    /// The runner an Unpair is on its way to.
    @State private var working: String?
    @State private var trouble: String?

    init(agents: NativeAgents = .shared) {
        _native = ObservedObject(wrappedValue: agents)
        _pairing = ObservedObject(wrappedValue: agents.pairing)
    }

    var body: some View {
        let targets = Set(native.remotes).union(pairing.states.keys).sorted()
        if native.enabled || !pairing.states.isEmpty, !targets.isEmpty {
            Section {
                ForEach(targets, id: \.self) { target in
                    row(target, pairing.states[target])
                }
                if let trouble {
                    Text(trouble).font(.callout).foregroundStyle(.secondary)
                }
            } header: {
                Text("Conversation View")
            } footer: {
                Text(Self.footer)
                .fixedSize(horizontal: false, vertical: true)
            }
        }
    }

    private func row(_ target: String, _ state: RemotePairing.State?) -> some View {
        LabeledContent {
            switch state {
            case .paired?:
                Button("Unpair") { unpair(target) }.disabled(working != nil)
            case .removed?, .unpaired?:
                Button("Pair Again") { native.pairAgain(target) }
            case .unavailable?:
                Button("Try Again") { native.retry(target) }
            case nil:
                EmptyView()
            }
        } label: {
            Text(target)
            Text(Self.status(state)).fixedSize(horizontal: false, vertical: true)
        }
    }

    /// What the key can do, said plainly: turning the setting on is the
    /// consent for every runner, and `control` runs commands.
    static let footer =
        "This Mac adds a key of its own, kept in the Keychain, to each runner. With it, anyone using Far Cooler "
        + "on this Mac can read and reply to Claude there, and create terminals and run commands, as with a paired phone."

    static func status(_ state: RemotePairing.State?) -> String {
        switch state {
        case .paired?: "Paired"
        case .removed?: "This Mac’s key was removed on the runner."
        case .unpaired?: "Unpaired"
        case .unavailable(let sentence)?: sentence
        case nil: "Not paired yet"
        }
    }

    private func unpair(_ target: String) {
        working = target
        trouble = nil
        Task {
            trouble = await native.unpair(target)
            working = nil
        }
    }
}
