import SwiftUI
import UIKit

@main
struct FarCoolerApp: App {
    /// Present only to catch the APNs device token, which arrives nowhere else.
    @UIApplicationDelegateAdaptor(PushDelegate.self) private var pushDelegate
    @ObservedObject private var themes = Themes.shared

    var body: some Scene {
        WindowGroup {
            harnessOrRoot
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

    /// `RootView`, unless this launch asked for the layout harness.
    @ViewBuilder
    private var harnessOrRoot: some View {
        #if DEBUG
        if AgentLayoutHarness.isRequested {
            AgentLayoutHarness()
        } else if ChangesLayoutHarness.isRequested {
            ChangesLayoutHarness()
        } else {
            RootView()
        }
        #else
        RootView()
        #endif
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
                //
                // The `NavigationStack` used to be here, wrapped around this.
                // The reason it was needed has not changed — `FleetView` was
                // previously PUSHED from the host list and inherited that
                // screen's stack, so opening straight onto it left no stack at
                // all: no navigation bar, so no title, no terminal/chat switch,
                // and nothing for a `navigationDestination` to push into. What
                // changed is that the stack now has an explicit path, and that
                // path is a list of ids that only this runner's fleet can
                // resolve. So the stack moved DOWN into `FleetView`, beside the
                // `Connection` it has to be read against; see the comment on
                // `FleetView.body`. This `.id` still rebuilds it, along with
                // everything else, when the runner changes.
                FleetView(host: host, store: hosts)
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
            // Beside the label and for the same reason: AgentKit files this
            // with the device so the relay can honor it while the app is
            // closed, and the key it lives under is this app's, not AgentKit's.
            PushRegistration.shared.notifyOnDone = { NotificationSettings.onDone }
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
/// One statement and ONE action. It carried two — "Authorize This Device" loud
/// and "Add a Runner" quiet — and choosing between them is a question nobody
/// arriving here can answer, because it is really a question about which road
/// they are about to take. Authorizing first is required on the manual road and
/// meaningless on the ceremony, which does both at once; the loud button was
/// therefore right half the time, with nothing on screen to say which half.
///
/// So the ordering decision moves inside `ConnectThisDeviceStep`, where each
/// road states its own, and this screen stops asking. What is left is the only
/// thing a runner-less device can do.
///
/// The gear stays and stops being load-bearing. Sign-in used to live down there
/// and nowhere else, while the ceremony was gated on it — so the shortest road
/// out of this screen ran through an unlabeled toolbar glyph. It is a step of
/// the flow that needs it now.
struct HostOnboardingView: View {
    @ObservedObject var hosts: RunnerStore

    @State private var showAdd = false
    /// Which of the two the sheet opens on, so the loud button can go straight
    /// to the wizard while the quiet one offers both.
    @State private var addStep: AddStep?
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
                    "Far Cooler runs coding agents on machines you reach over SSH. "
                        + "Connect this device to one to get started."
                )
                .font(.callout)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
                .frame(maxWidth: 300)

                Spacer()

                // One road out, and the other way in underneath it rather than
                // beside it: a device with no runners has exactly one thing it
                // can do, and the second button existed only to choose the
                // order in which to do that one thing.
                VStack(spacing: 18) {
                    Button {
                        addStep = .connectThisDevice
                        showAdd = true
                    } label: {
                        Text("Connect This Device").frame(maxWidth: .infinity)
                    }
                    .buttonStyle(.borderedProminent)
                    .controlSize(.large)

                    Button("More Ways to Add…") {
                        addStep = nil
                        showAdd = true
                    }
                    .font(.callout)
                }
                .padding(.horizontal, 40)
                .padding(.bottom, 40)
            }
            .frame(maxWidth: .infinity)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                // No "+" beside these. Adding is what the whole screen is, and a
                // toolbar shortcut would be a third control competing with two
                // that already say it in words.
                ToolbarItem(placement: .topBarLeading) {
                    Button { showSettings = true } label: { Image(systemName: "gearshape") }
                        .accessibilityLabel("Settings")
                }
            }
            .sheet(isPresented: $showAdd) {
                AddView(runners: hosts, initial: addStep)
            }
            .sheet(isPresented: $showSettings) {
                NavigationStack { SettingsView(runners: hosts) }
            }
        }
    }
}

/// Shows the public key to install on a host.
///
/// The manual path, and it stays. There is a ceremony now — two codes and a
/// camera, through `JoinView` — but this is what works when there is no trusted
/// device to scan with, and it is what is left the day every device is lost. Its
/// wording is unchanged apart from "runner".
struct AuthorizeView: View {
    /// Where a granted runner would be written. Nil where the caller has no
    /// store to hand over, which is also where the ceremony has nowhere to put
    /// its answer — so the route to it only appears with one.
    var runners: RunnerStore?

    /// Nil when there is none, and the screen says so rather than standing a
    /// sentence in where a key goes.
    ///
    /// It used to substitute one, two rows above
    /// `echo '<paste>' >> ~/.ssh/authorized_keys` and beside an enabled **Copy
    /// Public Key**. The obvious next move copied that sentence into a
    /// runner’s `authorized_keys`.
    private var publicKey: String? { Identity.publicKey }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                if let runners {
                    // Above the key, because it is the shorter road: someone
                    // holding a device that already has runners never needs to
                    // paste anything.
                    NavigationLink {
                        JoinView(runners: runners)
                    } label: {
                        Text("Add This Device With a Code").frame(maxWidth: .infinity)
                    }
                    .buttonStyle(.borderedProminent)
                    .controlSize(.large)
                    // Both roads off this screen carry the same key — the code
                    // this device shows IS its public key — so with none there
                    // is nothing to enroll, and following this would only push a
                    // refusal. The reason is in the sentence under it, since a
                    // disabled button cannot give one.
                    .disabled(publicKey == nil)

                    Text(
                        publicKey == nil
                            ? "The code carries this \(AddView.deviceKind)’s key, so it can’t be "
                                + "shown without one."
                            : "Use a device you’ve already added to choose which runners this device "
                                + "can access. Or add its public key manually:"
                    )
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                }

                Text(
                    publicKey == nil
                        ? "No key to add to the runner."
                        : "Add this device’s public key to the runner:"
                )
                .font(.callout)

                // All of this is about ONE key, so all of it is behind the key
                // existing: the copy button and the `echo` line are a pair, and
                // offering either without a key to put in it invites the paste
                // this guard is here to prevent.
                if let publicKey {
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
                } else {
                    // No cause and no promised retry, deliberately: this side
                    // knows only that key derivation returned nothing. Plain
                    // text, not the monospaced box — the box is where key
                    // material goes, and prose in it reads as a key.
                    Text("Far Cooler couldn’t make a key for this \(AddView.deviceKind).")
                        .foregroundStyle(.secondary)
                }

                Text(
                    publicKey == nil
                        ? "A runner can only let this \(AddView.deviceKind) in by its key, "
                            + "so there’s nothing to authorize until there is one."
                        : "The private key stays on this device. To revoke access, delete the line "
                            + "from authorized_keys."
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
                        "Far Cooler connects over SSH. Authorize this device on the runner first."
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
