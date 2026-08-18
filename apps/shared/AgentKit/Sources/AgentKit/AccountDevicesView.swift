import SwiftUI

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
    @State private var failed = false

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
            } else if failed {
                Section {
                    Text("Couldn’t load devices and runners.")
                        .foregroundStyle(.secondary)
                    Button("Try again") { Task { await load() } }
                }
            } else {
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
        }
        .formStyle(.grouped)
        .task { await load() }
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
        failed = false
        registrations = await account.fetchRegistrations()
        failed = registrations == nil
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
        _ = await account.revoke(row, kind: kind)
        await load()
    }
}
