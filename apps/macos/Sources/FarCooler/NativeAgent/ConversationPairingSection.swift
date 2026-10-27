import AgentKit
import SwiftUI

/// Settings > Devices > Conversation View (ov-408): each remote runner this
/// Mac's conversation key is paired with, or why it isn't, with the one
/// action that changes it. Pair writes the key to a runner never paired
/// here, and is the consent to (ov-830); Unpair takes it off the runner
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
        if !targets.isEmpty {
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
                PairButton(target: target, pairing: pairing) { native.pairAgain(target) }
            }
        } label: {
            Text(target)
            Text(Self.status(state)).fixedSize(horizontal: false, vertical: true)
        }
    }

    /// What the key can do, said plainly, beside the Pair that consents to
    /// it: `control` runs commands.
    static let footer =
        "Pair adds a key of this Mac’s own, kept in the Keychain, to that runner. With it, anyone using Far Cooler "
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

/// Pair, for a runner never paired here, and while it pairs a spinner that
/// says so (ov-830): in Settings > Devices and in a pane's note.
struct PairButton: View {
    let target: String
    @ObservedObject var pairing: RemotePairing
    let pair: () -> Void

    var body: some View {
        if pairing.pairing.contains(target) {
            HStack(spacing: Spacing.group) {
                ProgressView().controlSize(.small)
                Text("Pairing…")
            }
            .identified("native-pairing")
        } else {
            Button("Pair", action: pair)
                .identified("native-pair")
        }
    }
}
