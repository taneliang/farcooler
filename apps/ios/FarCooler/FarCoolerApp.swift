import SwiftUI
import UIKit

@main
struct FarCoolerApp: App {
    /// Present only to catch the APNs device token, which arrives nowhere else.
    @UIApplicationDelegateAdaptor(PushDelegate.self) private var pushDelegate

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
    return "farcooler-\(name)"
}

/// The app opens onto TERMINALS, not onto a list of machines.
///
/// It used to open on the host list, which meant the common case — one machine,
/// already added, agents running on it — cost a tap on every launch to get past
/// a screen with one row on it. A list of machines is onboarding, and onboarding
/// is not a home screen.
///
/// So the host list appears exactly when it is the thing to do: when there are
/// no hosts. Once there is one, the app lands on it, and switching machines
/// moves to where you already go to switch terminals — see
/// `WorkspaceListView`, which is the phone's equivalent of the Mac's sidebar.
struct RootView: View {
    @StateObject private var hosts = HostStore()

    var body: some View {
        Group {
            if let host = hosts.selected {
                // The stack `FleetView` and everything under it assume.
                //
                // `FleetView` was previously PUSHED from the host list, so it
                // inherited that screen's `NavigationStack`. Opening straight
                // onto it left no stack at all: no navigation bar, so no title,
                // no terminal/chat switch, and `navigationDestination` had
                // nothing to push into.
                //
                // Keyed on the host, so switching machines rebuilds everything
                // below rather than handing one host's screen the other's
                // connection. The same rule the Mac follows for a pane whose
                // terminal changes underneath it.
                NavigationStack {
                    FleetView(host: host, store: hosts)
                }
                .id(host.id)
            } else {
                HostOnboardingView(hosts: hosts)
            }
        }
        // Generate the device key at launch rather than the first time
        // something asks for it.
        //
        // It used to appear only when the "Authorise" screen was opened, which
        // put a several-hundred-millisecond keygen behind a tap and, worse,
        // meant a host could be added and connected to before this device had
        // an identity to offer. Doing it here costs one keygen on first run and
        // nothing on every run after — `privateKey()` returns the stored one.
        .task {
            _ = Identity.publicKey
            // Asked for at launch, alongside the device key.
            //
            // Not on the first notification: the point of this feature is being
            // told about an agent while you are not looking at the app, and a
            // permission prompt that only appears once you ARE looking has
            // already missed it.
            Notifier.shared.requestAuthorisation()
            PushRegistration.shared.label = { UIDevice.current.name }
            AccountSection.afterSignIn = { await PushRegistration.shared.sendIfPossible() }
        }
    }
}

/// The first-run screen: no machines yet, so there is nothing else to show.
struct HostOnboardingView: View {
    @ObservedObject var hosts: HostStore

    @State private var showAddHost = false
    @State private var showSettings = false

    var body: some View {
        NavigationStack {
            List {
                Section {
                    VStack(alignment: .leading, spacing: 10) {
                        Text("No hosts yet").font(.headline)
                        Text(
                            "Far Cooler drives machines you already have SSH access to. "
                                + "Add one, then authorise this device on it."
                        )
                        .font(.callout)
                        .foregroundStyle(.secondary)
                    }
                    .padding(.vertical, 6)
                }

                Section {
                    Button("Add a host") { showAddHost = true }
                }

                Section("This device") {
                    NavigationLink("Authorise on a host") { AuthoriseView() }
                }
            }
            .navigationTitle("Far Cooler")
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    Button { showSettings = true } label: { Image(systemName: "gearshape") }
                        .accessibilityLabel("Settings")
                }
                ToolbarItem(placement: .topBarTrailing) {
                    Button { showAddHost = true } label: { Image(systemName: "plus") }
                }
            }
            .sheet(isPresented: $showAddHost) {
                AddHostView { hosts.add($0) }
            }
            .sheet(isPresented: $showSettings) {
                NavigationStack { SettingsView() }
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
                        "Far Cooler connects over SSH. This device must be authorised "
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
