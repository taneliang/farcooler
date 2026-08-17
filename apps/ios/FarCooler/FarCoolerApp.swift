import SwiftUI
import UIKit

@main
struct FarCoolerApp: App {
    /// Present only to catch the APNs device token, which arrives nowhere else.
    @UIApplicationDelegateAdaptor(PushDelegate.self) private var pushDelegate
    @ObservedObject private var themes = Themes.shared

    var body: some Scene {
        WindowGroup {
            RootView()
                // Whichever way the theme goes, everywhere.
                //
                // This was an unconditional `.dark`, and the reasoning was
                // sound as far as it went: half the app is a terminal, a
                // terminal is dark whatever the phone is set to, and a white
                // list handing off to a black screen looked like two
                // applications. Choosing one beat reconciling two.
                //
                // What it could not survive is a light TERMINAL. Now that the
                // palette is a choice, the same argument points at following
                // it: the chrome and the grid go the same way because they are
                // one theme, which is exactly the join that had to be
                // protected. Still one decision, just no longer a constant.
                .preferredColorScheme(themes.current.colorScheme)
                // The theme's own ground, not just its light/dark leaning.
                //
                // The scheme alone gives the SYSTEM's black, which is the
                // colour the complaint that started this was about — picking
                // Nord and still getting `#000000` chrome around a `#2E3440`
                // terminal is the theme applying to half the screen.
                .background(themes.current.backgroundColor)
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

/// The app opens onto TERMINALS, not onto a list of runners.
///
/// It used to open on the host list, which meant the common case — one runner,
/// already added, agents running on it — cost a tap on every launch to get past
/// a screen with one row on it. A list of runners is onboarding, and onboarding
/// is not a home screen.
///
/// So the host list appears exactly when it is the thing to do: when there are
/// no hosts. Once there is one, the app lands on it, and switching runners
/// moves to where you already go to switch terminals — see
/// `WorkspaceListView`, which is the phone's equivalent of the Mac's sidebar.
struct RootView: View {
    @StateObject private var hosts = RunnerStore()

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
                // Keyed on the host, so switching runners rebuilds everything
                // below rather than handing one host's screen the other's
                // connection. The same rule the Mac follows for a pane whose
                // terminal changes underneath it.
                //
                // On the whole VALUE, not just its id: correcting a mistyped
                // address is as much a change of runner as picking a different
                // one from the list, and keying on the id alone left the old
                // connection running while the screen showed the new details.
                // `RunnerStore.trust` deliberately does not write through to
                // `selected`, so approving a host key is not mistaken for one.
                NavigationStack {
                    FleetView(host: host, store: hosts)
                }
                .id(host)
            } else {
                HostOnboardingView(hosts: hosts)
            }
        }
        // Generate the device key at launch rather than the first time
        // something asks for it.
        //
        // It used to appear only when the "Authorize" screen was opened, which
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
            Notifier.shared.requestAuthorization()
            PushRegistration.shared.label = { UIDevice.current.name }
            AccountSection.afterSignIn = { await PushRegistration.shared.sendIfPossible() }
            // Alongside the push token and for the same reason: the tokens that
            // let the relay raise and dismiss a lock screen card can only be
            // collected while the app is running, and the case the card exists
            // for is the app NOT running.
            LiveActivities.shared.start()
        }
    }
}

/// The first-run screen: no runners yet, so there is nothing else to show.
///
/// One statement and two actions, the more important one loud. It used to be a
/// list — a prose card that looked like a row but did nothing, then two rows
/// under "First" and "Then" headers — which was the same idea said three times
/// in three shapes, crammed into the top third of the screen.
///
/// The order those steps go in is not a preference. The screen used to say "Add
/// one, then authorize this device on it", a sequence that cannot work: a
/// runner that has never seen this device's key refuses the very first
/// connection, so anyone who followed the instructions ended onboarding on a
/// failure screen. Authorizing first costs nothing and makes the first
/// connection the one that succeeds — which is why it is the prominent button
/// and adding a runner is the quiet one.
struct HostOnboardingView: View {
    @ObservedObject var hosts: RunnerStore

    @State private var showAddRunner = false
    @State private var showSettings = false

    var body: some View {
        NavigationStack {
            VStack(spacing: 0) {
                Spacer()

                Image(systemName: "server.rack")
                    .font(.system(size: 42, weight: .thin))
                    .foregroundStyle(.tertiary)
                    .padding(.bottom, 22)

                // One statement of what to do, not a status line ("No runners
                // yet") followed by a paragraph restating it followed by two
                // rows restating it again.
                Text("Connect a Runner")
                    .font(.title2.weight(.semibold))
                    .padding(.bottom, 8)

                Text(
                    "Far Cooler runs coding agents on runners you already reach "
                        + "over SSH. Put this device’s key on one, then add its address."
                )
                .font(.callout)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
                .frame(maxWidth: 300)

                Spacer()

                // The order is carried by which button is the loud one, which
                // is the only place it needs to be carried. Authorizing first
                // is not a preference: a runner that has never seen this
                // phone's key refuses the very first connection, so the other
                // order ends onboarding on a failure screen.
                VStack(spacing: 18) {
                    NavigationLink {
                        AuthorizeView()
                    } label: {
                        Text("Authorize This Device").frame(maxWidth: .infinity)
                    }
                    .buttonStyle(.borderedProminent)
                    .controlSize(.large)

                    Button("Add a Runner") { showAddRunner = true }
                }
                .padding(.horizontal, 40)
                .padding(.bottom, 40)
            }
            .frame(maxWidth: .infinity)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                // No "+" beside these. A third way to add a runner on a screen
                // whose whole purpose is two ordered steps is one more thing to
                // weigh up, and it is the step that must not come first.
                ToolbarItem(placement: .topBarLeading) {
                    Button { showSettings = true } label: { Image(systemName: "gearshape") }
                        .accessibilityLabel("Settings")
                }
            }
            .sheet(isPresented: $showAddRunner) {
                HostEditorView { hosts.add($0) }
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
/// account: a host authorizes this device exactly the way it authorizes any
/// other SSH client, and revokes it by deleting one line.
struct AuthorizeView: View {
    private var publicKey: String { Identity.publicKey ?? "could not generate a key" }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                Text("Add this device’s public key to the runner:")
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
                    Label("Copy Public Key", systemImage: "doc.on.doc")
                }
                .buttonStyle(.borderedProminent)

                Text("On the runner, run:")
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
        .navigationTitle("Authorize")
        .navigationBarTitleDisplayMode(.inline)
    }
}

/// Add a runner, or correct one that was typed in wrong.
///
/// One screen for both, because they are the same four fields and because the
/// second was missing entirely: the app opens onto the selected host, so a
/// mistyped address was a screen you could never get past and never fix. That
/// made "Remove" the other thing this had to grow — an unreachable host with no
/// delete is a permanently broken app.
struct HostEditorView: View {
    /// The host being corrected, or nil when adding a new one. Everything that
    /// differs between the two modes — the title, the confirm button's word,
    /// whether Remove exists at all — comes from this and nothing else.
    var existing: Runner?
    let onSave: (Runner) -> Void
    /// Only supplied when removing is something the caller can survive.
    var onRemove: ((Runner) -> Void)?

    @Environment(\.dismiss) private var dismiss

    @State private var label = ""
    @State private var address = ""
    @State private var user = ""
    @State private var port = "22"
    @State private var confirmingRemove = false
    /// The fields start empty and are filled from `existing` once, on appear.
    /// `@State` initial values are captured when the view is first built and
    /// would not survive the struct being recreated with different arguments.
    @State private var loaded = false

    private var isEditing: Bool { existing != nil }

    private var isValid: Bool {
        !address.trimmingCharacters(in: .whitespaces).isEmpty
            && !user.trimmingCharacters(in: .whitespaces).isEmpty
    }

    var body: some View {
        NavigationStack {
            Form {
                // Labeled rows rather than bare placeholder text.
                //
                // A placeholder names a field only while the field is empty,
                // which is fine for the add case and useless for the edit case
                // this screen also serves: four filled-in rows reading "Demo
                // host / 10.0.0.4 / me / 22" leave you to work out which is
                // which, and a port of "1" or a user of "me" gives no clue at
                // all. The label is the only thing that makes an edit screen
                // legible without emptying a field to find out what it was.
                Section("Runner") {
                    field("Name", text: $label, hint: "Optional")
                    field("Address", text: $address, hint: "Required")
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .keyboardType(.URL)
                    field("User", text: $user, hint: "Required")
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                    field("Port", text: $port, hint: "22")
                        .keyboardType(.numberPad)
                }

                Section {
                    Text(
                        "Far Cooler connects over SSH. This device must be authorized "
                        + "on the runner first."
                    )
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                    NavigationLink("Authorize This Device") { AuthorizeView() }
                }

                if isEditing, let existing {
                    // Only when editing, and only when the caller can handle it:
                    // removing the runner you are currently connected to is
                    // fine (the app falls back to another, or to onboarding),
                    // but it is the caller that knows that, not this screen.
                    if let onRemove {
                        Section {
                            Button("Remove This Runner", role: .destructive) {
                                confirmingRemove = true
                            }
                        } footer: {
                            Text("Removes it from this device only. Nothing on the runner changes.")
                        }
                        .confirmationDialog(
                            "Remove \(existing.label)?",
                            isPresented: $confirmingRemove,
                            titleVisibility: .visible
                        ) {
                            Button("Remove", role: .destructive) {
                                onRemove(existing)
                                dismiss()
                            }
                            Button("Cancel", role: .cancel) {}
                        }
                    }
                }
            }
            .navigationTitle(isEditing ? "Edit Runner" : "Add Runner")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button(isEditing ? "Save" : "Add") {
                        onSave(edited())
                        dismiss()
                    }
                    .disabled(!isValid)
                }
            }
            .onAppear {
                guard !loaded, let existing else { loaded = true; return }
                label = existing.label
                address = existing.address
                user = existing.user
                port = String(existing.port)
                loaded = true
            }
        }
    }

    private func field(_ name: String, text: Binding<String>, hint: String) -> some View {
        HStack {
            Text(name)
            Spacer(minLength: 16)
            TextField(hint, text: text)
                .multilineTextAlignment(.trailing)
                .foregroundStyle(.secondary)
        }
    }

    /// The host as typed. Keeps `existing`'s identity so an edit updates the
    /// host in place rather than adding a second one alongside it.
    private func edited() -> Runner {
        let trimmed = address.trimmingCharacters(in: .whitespaces)
        return Runner(
            id: existing?.id ?? UUID(),
            label: label.trimmingCharacters(in: .whitespaces).isEmpty ? trimmed : label,
            address: trimmed,
            port: Int(port) ?? 22,
            user: user.trimmingCharacters(in: .whitespaces),
            fingerprint: existing?.fingerprint)
    }
}
