import SwiftUI

/// Where tunneled runners and this device meet, and a way to change it.
///
/// **This is a recovery valve, not a feature.** A runner reached through the
/// tunnel has no address; a device and a runner find each other through a
/// rendezvous service, and every tunneled connection goes through it rather
/// than only the ones that could not go direct. The service the app ships with
/// is documented as best-effort and revocable at any time. Without this field,
/// the day it stops answering costs an App Store review, a Mac release, a Play
/// release and a visit to every runner in the fleet; with it, it costs a
/// setting.
///
/// So the copy here says what to do — leave it alone — rather than what could
/// be done. **Nothing in this product says anyone should run their own
/// rendezvous, and this screen must not become the place that implies it.**
/// It sits beside ``RelaySection`` because it answers the same shape of
/// question and carries the same warning: someone talked through changing this
/// by a caller claiming to be support has been phished, not helped.
///
/// Both ends have their own copy of the setting and both have to agree, which
/// is the one thing a person changing this has to know. A runner's is an
/// environment variable its installer sets (`FARCOOLER_DERP_MAP`), because a
/// runner must not be moved onto a rendezvous by whoever is dialing it.
public struct RendezvousSection: View {
    @ObservedObject private var account = Account.shared
    @State private var expanded = false
    @State private var draft = ""
    @State private var saved = false

    public init() {}

    private var isDefault: Bool { account.derpMap.isEmpty }

    public var body: some View {
        Section {
            // Closed by default, like the relay's, and for the stronger
            // version of the same reason: a person who has never heard of a
            // rendezvous has no business being shown a field for one.
            DisclosureGroup("Rendezvous", isExpanded: $expanded) {
                content
            }
        } footer: {
            Text(
                "Leave this empty unless Far Cooler has told you to change it. "
                    + "Far Cooler Support will never ask you to change it."
            )
        }
        .onAppear { draft = account.derpMap }
    }

    @ViewBuilder
    private var content: some View {
        VStack(alignment: .leading, spacing: 10) {
            // The mechanism, in one sentence, because the alternative is a
            // field whose effect nobody can predict. It says what a rendezvous
            // does and why this is here — not how to stand one up.
            Text(
                "Runners reached through the tunnel have no address. They and this "
                    + "device meet at a rendezvous service, and this is which one. "
                    + "It's here so a service that goes away can be replaced without "
                    + "an app update."
            )
            .font(.caption)
            .foregroundStyle(.secondary)

            if !isDefault {
                VStack(alignment: .leading, spacing: 6) {
                    Text(account.derpMap)
                        .font(.callout.monospaced())
                        .textSelection(.enabled)
                    // Said out loud, because a runner still on the default
                    // meets nothing this device is looking for, and no other
                    // screen would explain the silence. Tunneled runners fail
                    // by timing out rather than by refusing.
                    Text(
                        "Using a custom rendezvous. Runners have to be set to the same "
                            + "one or they won't be reachable."
                    )
                    .font(.caption)
                    .foregroundStyle(.orange)
                }
            }

            TextField("https://example.com/derpmap.json", text: $draft)
                .font(.callout.monospaced())
                .autocorrectionDisabled()
                #if os(iOS)
                    .textInputAutocapitalization(.never)
                    .keyboardType(.URL)
                #endif

            HStack {
                Button("Use Rendezvous") { apply(draft) }
                    .disabled(!isUsable(draft))
                Spacer()
                Button("Reset") { apply("") }
                    .disabled(isDefault)
            }

            if saved {
                // The setting is read when a connection is opened, so a
                // session already running is still meeting at the old place.
                Text("Rendezvous changed. Reconnect your runners.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
    }

    /// A rendezvous this app could actually use.
    ///
    /// The same answer ``Account/derpMapSetting(_:)`` gives, asked before the
    /// button is enabled rather than after it is pressed — so a value that
    /// would be dropped cannot be typed, saved, and then silently ignored.
    /// That failure would be invisible: a tunnel that meets nowhere times out
    /// rather than refusing.
    private func isUsable(_ text: String) -> Bool {
        let usable = Account.derpMapSetting(text)
        return !usable.isEmpty && usable != account.derpMap
    }

    private func apply(_ value: String) {
        account.derpMap = value
        draft = account.derpMap
        saved = true
    }
}
