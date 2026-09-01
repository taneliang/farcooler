import SwiftUI

#if os(macOS)
    import AppKit
#elseif os(iOS)
    import UIKit
#endif

/// What can reach you, and what can reach out.
///
/// Two lists, because they are two different powers and conflating them is how
/// people end up unable to answer the only question that matters after losing a
/// laptop: what can that machine still do? A DEVICE receives notifications. A
/// RUNNER sends them. Revoking either is one swipe, and takes effect at the
/// relay — which is the point, since the runner you want to revoke is usually
/// the one you can no longer run a command on.
///
/// In the apps rather than a web dashboard on purpose: two lists and two delete
/// buttons do not justify a second product with its own login, and everyone who
/// needs this already has the app open.
public struct AccountDevicesView: View {
    @ObservedObject private var account = Account.shared

    @State private var registrations: Registrations?
    @State private var loading = true
    /// Why the last load failed, not merely that it did.
    ///
    /// This screen used to say "Couldn’t load devices and runners." to somebody
    /// signed out, somebody offline, and somebody whose relay was returning
    /// 500 — one sentence for causes with three different fixes.
    @State private var failure: AccountError?
    /// Why the last Remove didn't take, if it didn't.
    ///
    /// Its own state rather than the account's `lastRelayFailure`: the reload
    /// that follows a Remove is a different call and usually works, so the row
    /// would come back with nothing said about why it came back.
    @State private var removalFailure: AccountError?
    @State private var copied = false

    public init() {}

    public var body: some View {
        Form {
            if !account.isSignedIn {
                Section {
                    Text("Sign in to view the devices and runners on your account.")
                        .foregroundStyle(.secondary)
                }
            } else if loading {
                Section {
                    HStack {
                        Text("Loading…")
                        Spacer()
                        ProgressView().controlSize(.small)
                    }
                }
            } else if let failure {
                Section {
                    Text("Couldn’t load devices and runners. \(failure.message)")
                        .foregroundStyle(.secondary)
                    Button("Try Again") { Task { await load() } }
                    details
                }
            } else {
                if let removalFailure {
                    Section {
                        Text("Couldn’t remove that. \(removalFailure.message)")
                            .font(.callout)
                            .foregroundStyle(.secondary)
                    }
                }
                list(
                    "Devices", kind: .device, rows: registrations?.devices ?? [],
                    empty: "No devices. Allow notifications on a device to add it.",
                    footer: "Devices receive notifications. Removing a device stops notifications "
                        + "until it opens Far Cooler again."
                )
                list(
                    "Runners", kind: .runner, rows: registrations?.runners ?? [],
                    empty: "No paired runners. Pair one in Runners.",
                    footer: "Runners can send notifications. Removing a runner stops it "
                        + "immediately, even when the runner is offline."
                )
            }

            // Only while something is still wrong, and only for a failure that
            // happened somewhere else — pairing a runner, filing a push token.
            // A load that just failed already says its piece above. Cleared by
            // the next call that works, so this is never a screen nagging about
            // an outage that ended an hour ago.
            if failure == nil, account.lastRelayFailure != nil {
                Section {
                    Text(lastProblem)
                        .font(.callout)
                        .foregroundStyle(.secondary)
                    details
                }
            }
        }
        .formStyle(.grouped)
        .task { await load() }
    }

    private var lastProblem: String {
        guard let diagnostic = account.lastRelayFailure else { return "" }
        let when = diagnostic.at.formatted(.relative(presentation: .named))
        return "Something else this app asked the relay didn’t work \(when). "
            + diagnostic.failure.message
    }

    /// The one affordance for somebody who is actually debugging.
    ///
    /// A button rather than a line of text, because the detail is a status code
    /// and a path — the kind of thing that belongs in a bug report and never on
    /// a screen. Nothing appears here until a call has failed, and it goes away
    /// again when one works.
    @ViewBuilder
    private var details: some View {
        #if os(macOS) || os(iOS)
            if let diagnostic = account.lastRelayFailure {
                Button(copied ? "Copied" : "Copy Details") { copy(diagnostic.line) }
                    .font(.callout)
            }
        #endif
    }

    private func copy(_ line: String) {
        #if os(macOS)
            NSPasteboard.general.clearContents()
            NSPasteboard.general.setString(line, forType: .string)
        #elseif os(iOS)
            UIPasteboard.general.string = line
        #endif
        copied = true
    }

    @ViewBuilder
    private func list(
        _ title: String, kind: RegistrationKind, rows: [Registration],
        empty: String, footer: String
    ) -> some View {
        Section {
            if rows.isEmpty {
                Text(empty).foregroundStyle(.secondary).font(.callout)
            } else {
                ForEach(rows) { row in
                    HStack {
                        VStack(alignment: .leading, spacing: 2) {
                            Text(row.label)
                            Text(row.subtitle)
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        }
                        Spacer()
                        Button("Remove", role: .destructive) {
                            Task { await revoke(row, kind: kind) }
                        }
                        // Plain, not the tinted default: a destructive button
                        // per row turns a list into a wall of red, and the one
                        // thing here worth emphasising is the label.
                        .buttonStyle(.plain)
                        .foregroundStyle(.secondary)
                    }
                }
            }
        } header: {
            Text(title)
        } footer: {
            Text(footer)
        }
    }

    private func load() async {
        guard account.isSignedIn else {
            loading = false
            return
        }
        loading = true
        failure = nil
        copied = false
        removalFailure = nil
        switch await account.fetchRegistrations() {
        case .success(let loaded):
            registrations = loaded
            failure = nil
        case .failure(let why):
            failure = why
        }
        loading = false
    }

    private func revoke(_ row: Registration, kind: RegistrationKind) async {
        // Removed from the list before the round trip, and put back by the
        // reload if the relay disagreed. A delete button that does nothing for a
        // second reads as broken.
        if kind == .device {
            registrations = Registrations(
                devices: (registrations?.devices ?? []).filter { $0.id != row.id },
                runners: registrations?.runners ?? [])
        } else {
            registrations = Registrations(
                devices: registrations?.devices ?? [],
                runners: (registrations?.runners ?? []).filter { $0.id != row.id })
        }
        let outcome = await account.revoke(row, kind: kind)
        // The reload is what puts a refused row back; this is what says why.
        // Removing is the one action on this screen with real consequences, and
        // a Remove that quietly did nothing is the failure worth naming.
        await load()
        if case .failure(let why) = outcome { removalFailure = why }
    }
}
