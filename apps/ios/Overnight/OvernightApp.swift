import SwiftUI
import UIKit

@main
struct OvernightApp: App {
    var body: some Scene {
        WindowGroup {
            RootView()
                // Dark everywhere, not just over the terminal.
                //
                // Half the app was already forced dark because a terminal is
                // dark whatever the phone is set to, and the light chrome around
                // it did not survive the join: a white list handing off to a
                // black screen, and back, looked like two applications. Choosing
                // one is better than reconciling two, and for a tool whose
                // content is terminals the choice makes itself.
                .preferredColorScheme(.dark)
        }
    }
}

/// The device name, used as the SSH key comment so a host's authorized_keys
/// says which phone each line belongs to — which is what makes revoking one
/// possible without guessing.
func UIDeviceName() -> String {
    let name = UIDevice.current.name.replacingOccurrences(of: " ", with: "-")
    return "overnight-\(name)"
}

struct RootView: View {
    @StateObject private var hosts = HostStore()
    @State private var showAddHost = false

    var body: some View {
        NavigationStack {
            List {
                if hosts.hosts.isEmpty {
                    Section {
                        VStack(alignment: .leading, spacing: 10) {
                            Text("No hosts yet").font(.headline)
                            Text(
                                "Overnight drives machines you already have SSH access to. "
                                + "Add one, then authorise this device on it."
                            )
                            .font(.callout)
                            .foregroundStyle(.secondary)
                        }
                        .padding(.vertical, 6)
                    }
                }

                ForEach(hosts.hosts) { host in
                    NavigationLink(value: host) {
                        VStack(alignment: .leading, spacing: 2) {
                            Text(host.label).font(.body.weight(.medium))
                            Text("\(host.user)@\(host.address)")
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        }
                    }
                }
                .onDelete { offsets in
                    offsets.map { hosts.hosts[$0] }.forEach(hosts.remove)
                }

                Section("This device") {
                    NavigationLink("Authorise on a host") { AuthoriseView() }
                }
            }
            .navigationTitle("Overnight")
            // Generate the device key at launch rather than the first time
            // something asks for it.
            //
            // It used to appear only when the "Authorise" screen was opened,
            // which put a several-hundred-millisecond keygen behind a tap and,
            // worse, meant a host could be added and connected to before this
            // device had an identity to offer. Doing it here costs one keygen on
            // first run and nothing on every run after — `privateKey()` returns
            // the stored one.
            .task { _ = Identity.publicKey }
            .navigationDestination(for: Host.self) { host in
                FleetView(host: host, store: hosts)
            }
            .toolbar {
                Button { showAddHost = true } label: { Image(systemName: "plus") }
            }
            .sheet(isPresented: $showAddHost) {
                AddHostView { hosts.add($0) }
            }
        }
    }
}

/// Shows the public key to install on a host.
///
/// Deliberately the whole of enrollment. There is no pairing code and no
/// account: a host authorises this device exactly the way it authorises any
/// other SSH client, and revokes it by deleting one line.
struct AuthoriseView: View {
    private var publicKey: String { Identity.publicKey ?? "could not generate a key" }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                Text("Add this device's public key to the host:")
                    .font(.callout)

                Text(publicKey)
                    .font(.system(.footnote, design: .monospaced))
                    .textSelection(.enabled)
                    .padding(12)
                    .background(Color(.secondarySystemBackground))
                    .clipShape(RoundedRectangle(cornerRadius: 10))

                Button {
                    UIPasteboard.general.string = publicKey
                } label: {
                    Label("Copy public key", systemImage: "doc.on.doc")
                }
                .buttonStyle(.borderedProminent)

                Text("On the host, run:")
                    .font(.callout)
                    .padding(.top, 8)
                Text("echo '<paste>' >> ~/.ssh/authorized_keys")
                    .font(.system(.footnote, design: .monospaced))
                    .textSelection(.enabled)
                    .padding(12)
                    .background(Color(.secondarySystemBackground))
                    .clipShape(RoundedRectangle(cornerRadius: 10))

                Text(
                    "The private key never leaves this device. Revoke it by deleting "
                    + "that line from authorized_keys."
                )
                .font(.footnote)
                .foregroundStyle(.secondary)
                .padding(.top, 4)
            }
            .padding()
        }
        .navigationTitle("Authorise")
        .navigationBarTitleDisplayMode(.inline)
    }
}

struct AddHostView: View {
    let onAdd: (Host) -> Void
    @Environment(\.dismiss) private var dismiss

    @State private var label = ""
    @State private var address = ""
    @State private var user = ""
    @State private var port = "22"

    private var isValid: Bool {
        !address.trimmingCharacters(in: .whitespaces).isEmpty
            && !user.trimmingCharacters(in: .whitespaces).isEmpty
    }

    var body: some View {
        NavigationStack {
            Form {
                Section("Host") {
                    TextField("Name", text: $label)
                    TextField("Address", text: $address)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                    TextField("User", text: $user)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                    TextField("Port", text: $port).keyboardType(.numberPad)
                }
                Section {
                    Text(
                        "Overnight connects over SSH. This device must be authorised "
                        + "on the host first."
                    )
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                }
            }
            .navigationTitle("Add host")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Add") {
                        let trimmed = address.trimmingCharacters(in: .whitespaces)
                        onAdd(
                            Host(
                                label: label.isEmpty ? trimmed : label,
                                address: trimmed,
                                port: Int(port) ?? 22,
                                user: user.trimmingCharacters(in: .whitespaces)
                            ))
                        dismiss()
                    }
                    .disabled(!isValid)
                }
            }
        }
    }
}
