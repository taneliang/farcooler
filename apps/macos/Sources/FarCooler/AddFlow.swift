import AgentKit
import SwiftUI

/// Everything add-shaped, behind one door — the Mac's half of what `AddView`
/// does on the phone, and named the same on purpose.
///
/// The two verbs are the same two, because they are properties of the product
/// and not of the platform:
///
/// - **Connect this Mac** — this machine gains access to runners, either by
///   scanning a code from a device that already has them, or by naming one
///   itself;
/// - **Add another device** — a phone or another Mac gains access to the
///   runners this one already reaches.
///
/// They were three controls in two different Settings tabs. "Add Device…" and
/// "Add This Mac to Another Account…" sat under Devices, both greyed out when
/// signed out with the reason in a footer pointing at a THIRD tab; adding a
/// runner was a bare text field under Runners. Nothing said that the first two
/// are opposite directions of one ceremony, and the third is what you do when
/// there is no ceremony to have.
///
/// The Runners tab keeps its list. That screen is about administering runners —
/// probing them, installing onto them, removing them — which is a different
/// activity from acquiring one, and the list is the only place the install
/// button can live.
struct AddView: View {
    @Environment(\.dismiss) private var dismiss
    @ObservedObject private var account = Account.shared

    /// Which road is open, or nil while the choice is still the screen. The
    /// two ceremony views bring their own Cancel and their own `dismiss`, so
    /// choosing one hands the whole sheet over to it.
    @State private var road: Road?

    /// Where to start, for a caller that already knows. The sidebar's
    /// "Add a runner…" knows.
    var initial: Road?

    enum Road: Hashable {
        case scanACode
        case runnerByAddress
        case addAnotherDevice
    }

    var body: some View {
        Group {
            switch road {
            case .none: chooser
            case .scanACode: JoinView()
            case .runnerByAddress: RunnerAddressStep(done: { dismiss() })
            case .addAnotherDevice: AddDeviceView()
            }
        }
        .onAppear { if road == nil { road = initial } }
    }

    private var chooser: some View {
        VStack(alignment: .leading, spacing: 18) {
            Text("Add")
                .font(.title2.weight(.semibold))

            choice(
                icon: "desktopcomputer.and.arrow.down",
                title: "Connect This Mac",
                detail: "Give this Mac access to runners — scan a code from a device that "
                    + "already has them, or name one yourself.",
                blocker: nil
            ) {
                VStack(alignment: .leading, spacing: 8) {
                    Button("Scan a Code…") { road = .scanACode }
                        .disabled(!account.isSignedIn)
                    Button("Add a Runner by Address…") { road = .runnerByAddress }
                }
            }

            Divider()

            choice(
                icon: "plus.rectangle.on.rectangle",
                title: "Add Another Device",
                detail: "Let a phone or another Mac use the runners this one reaches. "
                    + "It can access only the runners you select.",
                blocker: grantBlocker
            ) {
                Button("Scan Its Code…") { road = .addAnotherDevice }
                    .disabled(grantBlocker != nil)
            }

            HStack {
                Spacer()
                Button("Cancel") { dismiss() }
                    .keyboardShortcut(.cancelAction)
            }
        }
        .padding(20)
        .frame(width: 460)
    }

    /// One choice, with its actions under it — and, where it cannot be taken
    /// yet, the reason and the button that fixes it in the same place.
    ///
    /// Not a footer pointing at another tab. "Sign in first in Settings ›
    /// Account" was accurate and it is still an instruction to go somewhere
    /// else and come back, for a step that takes one click and can perfectly
    /// well happen here.
    @ViewBuilder
    private func choice<Actions: View>(
        icon: String,
        title: String,
        detail: String,
        blocker: String?,
        @ViewBuilder actions: () -> Actions
    ) -> some View {
        HStack(alignment: .top, spacing: 14) {
            Image(systemName: icon)
                .font(.system(size: 22))
                .frame(width: 30)
                .foregroundStyle(.tint)
            VStack(alignment: .leading, spacing: 8) {
                Text(title).font(.headline)
                Text(detail)
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                if let blocker {
                    Text(blocker)
                        .font(.callout)
                        .foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
                if !account.isSignedIn {
                    Button(account.signingIn ? "Signing In…" : "Sign In") {
                        Task {
                            await account.signIn()
                            await AccountSection.afterSignIn?()
                        }
                    }
                    .disabled(account.signingIn)
                }
                actions()
            }
            Spacer(minLength: 0)
        }
    }

    private var grantBlocker: String? {
        account.isSignedIn
            ? nil
            : "Both devices sign in to the same account, so each can prove who it is."
    }
}

/// Naming a runner, with what it costs to name it badly.
///
/// The same one field the Runners tab has — `ssh -G` resolves everything else,
/// which is why one field is enough — plus the sentence that field never said:
/// an address that only works on this network is one a phone loses the moment
/// it leaves, and this Mac has no way to tell it the new one later.
private struct RunnerAddressStep: View {
    let done: () -> Void

    @Environment(\.dismiss) private var dismiss
    @ObservedObject private var runners = Runners.shared
    @State private var target = ""

    private var trimmed: String { target.trimmingCharacters(in: .whitespaces) }

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("Add a Runner")
                .font(.title2.weight(.semibold))

            Text(
                "Far Cooler runs `farcooler` over SSH, so anything you can already reach "
                    + "works: a `user@host`, or an alias from your `~/.ssh/config`."
            )
            .font(.callout)
            .foregroundStyle(.secondary)
            .fixedSize(horizontal: false, vertical: true)

            TextField("user@host, or an ssh alias", text: $target)
                .autocorrectionDisabled()
                .onSubmit(add)

            Text(
                "A name that only resolves on your own network — anything ending in "
                    + ".local, or a 192.168 address — will stop working for a phone the "
                    + "moment it leaves. A Tailscale name reaches it from anywhere."
            )
            .font(.footnote)
            .foregroundStyle(.secondary)
            .fixedSize(horizontal: false, vertical: true)

            HStack {
                Spacer()
                Button("Cancel") { dismiss() }
                    .keyboardShortcut(.cancelAction)
                Button("Add", action: add)
                    .keyboardShortcut(.defaultAction)
                    .disabled(trimmed.isEmpty)
            }
        }
        .padding(20)
        .frame(width: 460)
    }

    private func add() {
        guard !trimmed.isEmpty else { return }
        runners.add(trimmed)
        // Probed straight away, exactly as the Runners tab does: the answer is
        // what turns a typed string into a row that says whether Far Cooler is
        // installed, and it changes nothing on the runner.
        let target = trimmed
        Task { await runners.probe(target) }
        done()
    }
}
