import SwiftUI
import UIKit

/// Everything add-shaped, behind one door.
///
/// There used to be five. `HostOnboardingView` had two buttons, `AuthorizeView`
/// had a third route hidden under them, `HostEditorView` had a fourth that led
/// back to `AuthorizeView` with its ceremony silently missing, and Settings had
/// a fifth in a section that only existed once you were signed in. They were all
/// called "add", and they meant two unrelated things:
///
/// - **this device gets access** — a local address book entry, plus a key on the
///   runner, which is what the ceremony writes;
/// - **another device gets access** — a line in `~/.ssh/authorized_keys` on
///   runners this device already reaches.
///
/// Those are the two rows here, in the user's own words rather than the
/// protocol's. Everything else is a step inside one of them.
///
/// The hub owns its `NavigationStack` and drives it by path rather than by
/// `NavigationLink`, because the wizard has to be able to SKIP a step: someone
/// already signed in must not be shown a sign-in screen, and someone who signs
/// in must land on the code rather than back where they started.
struct AddView: View {
    @ObservedObject var runners: RunnerStore
    /// Where to start, for a caller that already knows which of the two this
    /// is. Onboarding's primary button knows — there is only one runner-less
    /// answer — and jumping straight there is what makes the first run one tap
    /// rather than a menu.
    var initial: AddStep?

    @Environment(\.dismiss) private var dismiss
    @State private var path: [AddStep] = []

    var body: some View {
        NavigationStack(path: $path) {
            hub
                .navigationTitle("Add")
                .navigationBarTitleDisplayMode(.inline)
                .toolbar {
                    ToolbarItem(placement: .confirmationAction) {
                        Button("Done") { dismiss() }
                    }
                }
                .navigationDestination(for: AddStep.self) { step in
                    destination(step)
                }
        }
        .onAppear {
            // Assigned once. A path seeded on every appearance would push the
            // step again each time the sheet came back from the background.
            if let initial, path.isEmpty { path = [initial] }
        }
    }

    private var hub: some View {
        List {
            Section {
                row(
                    step: .connectThisDevice,
                    icon: "iphone.and.arrow.forward",
                    title: "Connect This Device",
                    detail: "Get this \(Self.deviceKind) onto a runner, by code or by address."
                )
            }

            Section {
                row(
                    step: .addAnotherDevice,
                    icon: "plus.rectangle.on.rectangle",
                    title: "Add Another Device",
                    detail: "Let another phone or Mac use the runners on this \(Self.deviceKind).",
                    blocker: grantBlocker
                )
            } footer: {
                Text("New devices can access only the runners you select.")
            }
        }
    }

    /// One row, and — when the thing it leads to cannot work yet — the reason
    /// and the fix on the row itself.
    ///
    /// Not a disabled row, and above all not a screen you can reach and then
    /// cannot leave. Tapping "Add This Device With a Code" while signed out used
    /// to land on a page reading "Sign in to add this device" whose only button
    /// was **Done**: it named the thing standing in the way and offered no way
    /// to do it, on a device whose owner had no reason to know sign-in lives
    /// three taps deep behind a gear glyph.
    @ViewBuilder
    private func row(
        step: AddStep,
        icon: String,
        title: String,
        detail: String,
        blocker: Blocker? = nil
    ) -> some View {
        if let blocker {
            VStack(alignment: .leading, spacing: 10) {
                label(icon: icon, title: title, detail: detail)
                    .foregroundStyle(.secondary)
                Text(blocker.reason)
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                if let fix = blocker.fix {
                    Button(fix.title) { fix.run() }
                        .buttonStyle(.bordered)
                        .controlSize(.small)
                }
            }
            .padding(.vertical, 4)
        } else {
            NavigationLink(value: step) {
                label(icon: icon, title: title, detail: detail)
            }
        }
    }

    private func label(icon: String, title: String, detail: String) -> some View {
        HStack(alignment: .top, spacing: 14) {
            Image(systemName: icon)
                .font(.system(size: 20))
                .frame(width: 28)
                .foregroundStyle(.tint)
            VStack(alignment: .leading, spacing: 3) {
                Text(title).font(.body)
                Text(detail)
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .padding(.vertical, 2)
    }

    /// What, if anything, stands between this device and granting access to
    /// another one. Order matters: an account is the harder thing to explain,
    /// so a device with neither is told about the runner first.
    private var grantBlocker: Blocker? {
        if runners.hosts.isEmpty {
            return Blocker(
                reason: "Connect this \(Self.deviceKind) to a runner first. "
                    + "There's nothing to share yet.",
                fix: nil
            )
        }
        if !Account.shared.isSignedIn {
            return Blocker(
                reason: "Both devices sign in to the same account, so each can prove who it is.",
                fix: Fix(title: "Sign In") {
                    Task {
                        await Account.shared.signIn()
                        await AccountSection.afterSignIn?()
                    }
                }
            )
        }
        return nil
    }

    @ViewBuilder
    private func destination(_ step: AddStep) -> some View {
        switch step {
        case .connectThisDevice:
            ConnectThisDeviceStep(path: $path)
        case .signIn:
            SignInStep(path: $path)
        case .joinWithCode:
            JoinView(runners: runners)
        case .runnerAddress:
            RunnerAddressStep(runners: runners)
        case .addAnotherDevice:
            AddDeviceView(runners: runners)
        }
    }

    /// "iPhone" or "iPad", because the sentence is about the object in the
    /// reader's hand and "device" is what a protocol calls it.
    static var deviceKind: String {
        UIDevice.current.userInterfaceIdiom == .pad ? "iPad" : "iPhone"
    }
}

/// One screen of the add flow. `Hashable` because the stack is driven by value.
enum AddStep: Hashable {
    case connectThisDevice
    case signIn
    case joinWithCode
    case runnerAddress
    case addAnotherDevice
}

/// Something that has to happen before a row can be used, and the button that
/// does it. `fix` is nil where the answer is somewhere else entirely.
private struct Blocker {
    let reason: String
    let fix: Fix?
}

private struct Fix {
    let title: String
    let run: () -> Void
}

/// Step one of connecting: which road.
///
/// The two are genuinely different in kind, not two spellings of one thing, and
/// the old screen never said so. It offered "Authorize This Device" and "Add a
/// Runner" and encoded their ORDER by which button was loud — because a runner
/// that has never seen this device's key refuses the first connection, so
/// authorizing has to come first. That is true of the manual road and false of
/// the ceremony, which does both at once. So the prominent button was the right
/// one half the time, and nothing on screen said which half you were in.
///
/// Here the order lives inside each road, where it cannot be got wrong.
private struct ConnectThisDeviceStep: View {
    @Binding var path: [AddStep]
    @ObservedObject private var account = Account.shared

    var body: some View {
        List {
            Section {
                Button {
                    // Signed in already? Then there is nothing to ask, and
                    // asking anyway is the step that made this feel long.
                    path.append(account.isSignedIn ? .joinWithCode : .signIn)
                } label: {
                    road(
                        icon: "qrcode.viewfinder",
                        title: "Scan a Code",
                        detail: "Use a device that already has your runners. "
                            + "Nothing to type, and it picks up their addresses too.",
                        recommended: true
                    )
                }
            }

            Section {
                Button { path.append(.runnerAddress) } label: {
                    road(
                        icon: "network",
                        title: "Enter an Address",
                        detail: "Type a runner's SSH address and put this "
                            + "\(AddView.deviceKind)'s key on it yourself. No account needed.",
                        recommended: false
                    )
                }
            }
        }
        .navigationTitle("Connect This Device")
        .navigationBarTitleDisplayMode(.inline)
    }

    private func road(
        icon: String,
        title: String,
        detail: String,
        recommended: Bool
    ) -> some View {
        HStack(alignment: .top, spacing: 14) {
            Image(systemName: icon)
                .font(.system(size: 22))
                .frame(width: 30)
                .foregroundStyle(.tint)
            VStack(alignment: .leading, spacing: 4) {
                HStack(spacing: 8) {
                    Text(title).font(.body.weight(.medium))
                    if recommended {
                        Text("Recommended")
                            .font(.caption2.weight(.semibold))
                            .padding(.horizontal, 6)
                            .padding(.vertical, 2)
                            .background(Color.accentColor.opacity(0.15))
                            .clipShape(Capsule())
                    }
                }
                Text(detail)
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            Spacer(minLength: 0)
            Image(systemName: "chevron.right")
                .font(.footnote.weight(.semibold))
                .foregroundStyle(.tertiary)
        }
        .padding(.vertical, 6)
        .contentShape(Rectangle())
    }
}

/// Sign-in, as a step of the thing that needs it.
///
/// The same `Account` the Settings row drives — there is one sign-in in this
/// app and this is not a second one. What is new is that it says WHY, at the
/// moment the answer matters, and that finishing it carries on to the code
/// instead of returning you to the screen you were already on.
///
/// The escape matters as much as the button. Someone who does not want an
/// account is not stuck here: the other road needs none, and this says so.
private struct SignInStep: View {
    @Binding var path: [AddStep]
    @ObservedObject private var account = Account.shared

    var body: some View {
        VStack(spacing: 0) {
            Spacer()

            Image(systemName: "person.badge.key")
                .font(.system(size: 40, weight: .thin))
                .foregroundStyle(.tertiary)
                .padding(.bottom, 20)

            Text("Sign In to Scan a Code")
                .font(.title3.weight(.semibold))
                .padding(.bottom, 8)

            Text(
                "Both devices sign in to the same account, so each can prove who it is "
                    + "before any key is written. Far Cooler still reaches your runners "
                    + "over SSH — the account isn't a way in."
            )
            .font(.callout)
            .foregroundStyle(.secondary)
            .multilineTextAlignment(.center)
            .frame(maxWidth: 320)

            if let error = account.lastError {
                Text(error)
                    .font(.footnote)
                    .foregroundStyle(.orange)
                    .multilineTextAlignment(.center)
                    .padding(.top, 14)
                    .frame(maxWidth: 320)
            }

            Spacer()

            VStack(spacing: 16) {
                Button {
                    Task {
                        await account.signIn()
                        await AccountSection.afterSignIn?()
                    }
                } label: {
                    Group {
                        if account.signingIn {
                            HStack(spacing: 8) {
                                ProgressView().controlSize(.small)
                                Text("Signing In…")
                            }
                        } else {
                            Text("Sign In")
                        }
                    }
                    .frame(maxWidth: .infinity)
                }
                .buttonStyle(.borderedProminent)
                .controlSize(.large)
                .disabled(account.signingIn)

                Button("Enter an Address Instead") {
                    // Replaces this step rather than pushing past it: going
                    // back from the address screen should reach the choice, not
                    // a sign-in nobody chose.
                    path.removeLast()
                    path.append(.runnerAddress)
                }
                .font(.callout)
            }
            .padding(.horizontal, 40)
            .padding(.bottom, 40)
        }
        .frame(maxWidth: .infinity)
        .navigationTitle("Sign In")
        .navigationBarTitleDisplayMode(.inline)
        // Carries on by itself. Signing in is not the thing anyone came here to
        // do, and making them tap Next afterwards is the shape of a form rather
        // than of an errand that is over.
        .onChange(of: account.isSignedIn) { _, signedIn in
            guard signedIn else { return }
            path.removeLast()
            path.append(.joinWithCode)
        }
    }
}

/// The manual road, on ONE screen.
///
/// Its two halves used to be two, and in the wrong order: `HostEditorView` took
/// the address and carried a link to `AuthorizeView` for the key, while the
/// runner cannot accept a connection until the key is already on it. So the
/// honest sequence was key first, address second, and the screens went the
/// other way — leaving people to add a runner, fail to connect, and go looking
/// for what they missed.
///
/// Worse, the `AuthorizeView` reachable from there was constructed with no
/// `runners`, which is the argument that decides whether the ceremony is
/// offered at all. The shorter road was invisible from the longer one.
///
/// Both halves are here now, key first, with the runner's own command to paste.
private struct RunnerAddressStep: View {
    @ObservedObject var runners: RunnerStore

    @State private var label = ""
    @State private var address = ""
    @State private var user = ""
    @State private var port = "22"
    @State private var copied = false

    private var publicKey: String { Identity.publicKey ?? "could not generate a key" }

    private var isValid: Bool {
        !address.trimmingCharacters(in: .whitespaces).isEmpty
            && !user.trimmingCharacters(in: .whitespaces).isEmpty
    }

    var body: some View {
        Form {
            Section {
                Text(publicKey)
                    .font(.system(.footnote, design: .monospaced))
                    .textSelection(.enabled)
                Button {
                    UIPasteboard.general.string = publicKey
                    copied = true
                } label: {
                    Label(copied ? "Copied" : "Copy Public Key", systemImage: copied ? "checkmark" : "doc.on.doc")
                }
                Text("echo '<paste>' >> ~/.ssh/authorized_keys")
                    .font(.system(.footnote, design: .monospaced))
                    .textSelection(.enabled)
            } header: {
                Text("1. Put this key on the runner")
            } footer: {
                Text(
                    "The private key never leaves this \(AddView.deviceKind). "
                        + "To revoke access later, delete the line from authorized_keys."
                )
            }

            Section {
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
            } header: {
                Text("2. Where the runner is")
            } footer: {
                Text(
                    "A name that only works on your own network — anything ending in .local, "
                        + "or a 192.168 address — stops working the moment you leave it. "
                        + "A Tailscale name reaches it from anywhere."
                )
            }

            Section {
                Button("Add Runner") { runners.add(typed()) }
                    .disabled(!isValid)
            }
        }
        .navigationTitle("Enter an Address")
        .navigationBarTitleDisplayMode(.inline)
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

    /// The runner as typed. No `existing` to preserve — correcting one is
    /// `HostEditorView`'s job, and this screen only ever adds.
    private func typed() -> Runner {
        let trimmed = address.trimmingCharacters(in: .whitespaces)
        return Runner(
            id: UUID(),
            label: label.trimmingCharacters(in: .whitespaces).isEmpty ? trimmed : label,
            address: trimmed,
            port: Int(port) ?? 22,
            user: user.trimmingCharacters(in: .whitespaces),
            fingerprint: nil)
    }
}
