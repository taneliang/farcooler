import SwiftUI

/// Which relay this app talks to, and a way to change it.
///
/// There is one relay per channel — separate deployments, separate databases,
/// separate WorkOS environments — so a build normally talks to its own and this
/// screen is a read-only fact. See
/// `docs/superpowers/specs/2026-08-12-relay-channels-and-staging-design.md`.
///
/// The override exists on EVERY channel rather than debug builds only, because
/// the thing worth testing is an old app against a new relay, and the app most
/// worth testing that way is a release build.
///
/// It is behind a disclosure, closed by default, and it says what it does.
/// Where a person's notifications go is worth changing on purpose: someone
/// talked through changing it by a caller claiming to be support has been
/// phished, not helped, and a screen that made it a one-tap field would be
/// helping that along.
public struct RelaySection: View {
    @ObservedObject private var account = Account.shared
    @State private var expanded = false
    @State private var draft = ""
    @State private var saved = false

    public init() {}

    private var isDefault: Bool { account.relay == Account.defaultRelay }

    public var body: some View {
        Section {
            // A disclosure rather than a bare field, and closed by default.
            // `Section(isExpanded:)` cannot carry a footer, and the footer is
            // the part that says who never asks you to change this.
            DisclosureGroup("Relay", isExpanded: $expanded) {
                content
            }
        } header: {
            Text("Advanced")
        } footer: {
            Text(
                "Change the relay only for testing. Far Cooler Support will never ask you "
                    + "to change it."
            )
            .font(.caption)
            .foregroundStyle(.secondary)
        }
        .onAppear { draft = account.relay }
    }

    @ViewBuilder
    private var content: some View {
        VStack(alignment: .leading, spacing: 10) {
            VStack(alignment: .leading, spacing: 6) {
                Text(account.relay)
                    .font(.callout.monospaced())
                    .textSelection(.enabled)
                if !isDefault {
                    // Said out loud, because a relay that is not this build's
                    // own is the reason notifications would be missing, and
                    // nothing else on this screen would explain it.
                    Text("Using a custom relay. Notifications will be sent there.")
                        .font(.caption)
                        .foregroundStyle(.orange)
                }
            }

            TextField("https://relay.example.com", text: $draft)
                .font(.callout.monospaced())
                .autocorrectionDisabled()
                #if os(iOS)
                    .textInputAutocapitalization(.never)
                    .keyboardType(.URL)
                #endif

            HStack {
                Button("Use Relay") { apply(draft) }
                    .disabled(!isUsable(draft))
                Spacer()
                Button("Reset") { apply(Account.defaultRelay) }
                    .disabled(isDefault)
            }

            if saved {
                // Changing where a runner reports invalidates what it was
                // given: a daemon token is issued BY a relay and means nothing
                // to another one, so the pairing has to be redone rather than
                // silently failing the next time something tries to notify.
                Text("Relay changed. Pair your runners again.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
    }

    /// A URL this app could actually reach.
    ///
    /// `https` only, and not merely on principle: a relay carries a session
    /// token on every request, and both platforms would refuse a cleartext
    /// request at the transport layer anyway — so accepting one here would
    /// produce a setting that saves and then silently never works.
    private func isUsable(_ text: String) -> Bool {
        let trimmed = text.trimmingCharacters(in: .whitespaces)
        guard let url = URL(string: trimmed), url.scheme == "https", url.host != nil else {
            return false
        }
        return trimmed != account.relay
    }

    private func apply(_ value: String) {
        account.relay = value.trimmingCharacters(in: .whitespaces)
        draft = account.relay
        saved = true
    }
}
