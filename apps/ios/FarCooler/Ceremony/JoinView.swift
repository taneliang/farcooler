import SwiftUI

/// This device asking to be added: show a code, then scan the reply.
///
/// The code carries this device's public key, its name, an opaque account id,
/// the channel and a random ceremony id. Nothing in it is a secret — a
/// photograph of it enrolls keys belonging to a device the photographer does
/// not hold — which is why it can simply sit on a screen.
struct JoinView: View {
    /// Where the runners a reply grants are written. The whole point of the
    /// exchange, so this screen is only offered where there is one.
    @ObservedObject var runners: RunnerStore

    @StateObject private var store: CeremonyStore
    @StateObject private var scanner = CodeScanner()
    @ObservedObject private var account = Account.shared
    @Environment(\.dismiss) private var dismiss

    init(runners: RunnerStore) {
        self.runners = runners
        _store = StateObject(
            wrappedValue: CeremonyStore(
                account: Account.shared.userId,
                accountEmail: Account.shared.email,
                deviceName: UIDevice.current.name))
    }

    var body: some View {
        content
            .navigationTitle("Add This Device")
            .navigationBarTitleDisplayMode(.inline)
            .onAppear {
                if store.phase == .idle, account.isSignedIn {
                    store.showOffer(publicKey: Identity.publicKey)
                }
            }
            .onChange(of: scanner.scanned) { _, payload in
                guard let payload else { return }
                store.takeReply(payload)
                // Written the moment the reply is taken, not when someone taps
                // Done: a reply is consumed once, so a back-swipe off the next
                // screen would otherwise throw away the only copy of it.
                if store.phase == .done { adopt() }
            }
    }

    @ViewBuilder
    private var content: some View {
        if !account.isSignedIn {
            signInFirst
        } else {
            switch store.phase {
            case .idle, .showingOffer:
                offer

            case .scanning:
                ScanScreen(
                    scanner: scanner,
                    instruction: "Scan the code on the other device.",
                    onCancel: { store.showCodeAgain() })

            case .done:
                added

            case .refused(let refusal):
                RefusalScreen(refusal: refusal, retry: "Show a New Code") {
                    store.showOffer(publicKey: Identity.publicKey)
                } onDone: {
                    dismiss()
                }

            case .mismatch, .confirming, .enrolling, .showingManifest:
                // The other side of the ceremony. Unreachable here.
                ProgressView()
            }
        }
    }

    // MARK: - Signed out

    private var signInFirst: some View {
        VStack(spacing: 16) {
            Spacer()
            Text("Sign in to add this device")
                .font(.title3.weight(.semibold))
            Text(
                "Both devices must be signed in to the same account. You can also add this "
                    + "device by copying its public key."
            )
            .font(.callout)
            .foregroundStyle(.secondary)
            .multilineTextAlignment(.center)
            Spacer()
            Button("Done") { dismiss() }
        }
        .padding(24)
    }

    // MARK: - This device's code

    private var offer: some View {
        VStack(spacing: 18) {
            Text("Scan this code with another device")
                .font(.headline)
                .multilineTextAlignment(.center)

            CodeImageView(payload: store.code)
                .frame(maxWidth: 360)

            Text(
                "On a device you’ve already added, open Far Cooler and choose Add Device. "
                    + "Then scan this code."
            )
            .font(.footnote)
            .foregroundStyle(.secondary)
            .multilineTextAlignment(.center)

            Button {
                scanner.start()
                store.beginScanning()
            } label: {
                Text("Scan Code").frame(maxWidth: .infinity)
            }
            .buttonStyle(.borderedProminent)
            .controlSize(.large)
            .padding(.top, 8)
        }
        .padding(24)
    }

    // MARK: - Added

    private var added: some View {
        let joined = Joined(store.granted)
        return VStack(alignment: .leading, spacing: 16) {
            Text(joined.headline)
                .font(.title3.weight(.semibold))

            if let summary = joined.summary {
                Text(summary)
                    .font(.callout)
                    .foregroundStyle(.secondary)
            }

            ForEach(joined.reachable) { runner in
                VStack(alignment: .leading, spacing: 2) {
                    Text(runner.label)
                    Text(runner.reach.detail(user: runner.user))
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                .padding(.vertical, 4)
            }

            // The runners this device was NOT given a key for, named. The
            // mirror of what the granting screen says in `AddDeviceView`, and
            // orange for the same reason: it is the half somebody has to act
            // on, and the alternative to reading it here is meeting it later as
            // a connection that fails for no stated reason.
            if let note = joined.note {
                Text(note)
                    .font(.callout)
                    .foregroundStyle(.orange)
                    .fixedSize(horizontal: false, vertical: true)
            }

            Spacer()

            Button {
                dismiss()
            } label: {
                Text("Done").frame(maxWidth: .infinity)
            }
            .buttonStyle(.borderedProminent)
            .controlSize(.large)
        }
        .padding(24)
    }

    /// Write the granted runners into this device's own list.
    ///
    /// Matched by where they are reached and as whom, rather than by the id in
    /// the manifest: two devices generate their own ids for the same runner, so
    /// adopting by id would leave someone with the same box listed twice.
    ///
    /// A pending runner is not written into it at all — see ``Joined``. It is
    /// also not taken back OUT of it: this reply says nothing about a key some
    /// earlier ceremony wrote, and dropping a runner the person is already
    /// using would be a second wrong answer rather than a correction.
    ///
    /// A tunneled runner is not written in either, and for a different reason:
    /// ``Runner`` records an address and a port. ``Joined/note`` names it, so
    /// nobody has to notice a runner that quietly did not arrive.
    private func adopt() {
        for entry in store.granted {
            guard let arriving = entry.asRunner else { continue }
            let existing = runners.hosts.first {
                $0.address == arriving.address && $0.user == arriving.user
                    && $0.port == arriving.port
            }
            if let existing {
                // Already known. The pin is the one thing worth taking from the
                // manifest, and only when this device has none of its own: a
                // fingerprint someone approved here outranks one that arrived
                // in a code. Taken whatever `pending` says, because a host key
                // is a fact about the box and the flag is a fact about a file.
                if existing.fingerprint == nil, !entry.host_key.isEmpty {
                    var updated = existing
                    updated.fingerprint = entry.host_key
                    runners.update(updated)
                }
            } else if !entry.pending {
                runners.add(arriving)
            }
        }
    }
}

/// A reply, as the device RECEIVING it has to read it.
///
/// `CeremonyRunner.pending` means "this runner does NOT have this device's
/// key". The granting side decides it from what its writes actually did — a
/// Mac in `Enrollment.Outcome.granting(_:)`, a phone in
/// ``CeremonyStore/confirm()`` — so it arrives here as a fact about a file on
/// another machine, and a failed write, an unreachable runner and one never
/// attempted all arrive the same way, because the file is in the same state in
/// all three.
///
/// **Nothing here fixes one, and nothing anywhere else does either.** There is
/// no retry queue, and a phone may never enroll a device's key: granting is a
/// Mac-and-CLI capability. A runner a granting device could not write to
/// traveling pending is the intended end state, so the only move this device
/// has is to be honest about it — which is what this split is.
///
/// ## What a pending runner does NOT get
///
/// It is not added to the runner list, and it is not shown as one this device
/// can access. Both used to happen to every runner in the reply, and both are
/// claims of access: an entry that can only fail to connect, in a list where it
/// looks identical to a working one. Selecting it is the first thing this app
/// does with a new runner, so it would be the first thing to fail.
///
/// Leaving it out loses nothing that lasts. The remedy the note names is this
/// same ceremony, run again from a device that CAN reach that runner, and the
/// runner then arrives with `pending` false: added at the moment it works
/// rather than before it does. An entry marked "might not work" would be worse
/// than no entry, because nothing in this app could ever clear the mark.
///
/// The Mac reads a reply the same way, in its own `Joined`. The two differ only
/// in what the screen does with the answer — the phone lists runners, the Mac
/// counts them in a sentence — and in the one extra thing a Mac writes for a
/// runner it can reach, which is the `~/.ssh/config` block.
struct Joined {
    /// The runners whose `authorized_keys` has this device's key, and that this
    /// app can record.
    let reachable: [CeremonyRunner]
    /// The runners that do not have the key.
    let pending: [CeremonyRunner]
    /// The runners this app cannot record at all, whatever the key says: a
    /// tunneled runner has no address, and ``Runner`` is an address and a port.
    /// Separate from ``pending`` because the two have different remedies and
    /// neither is the other's fault.
    let unstorable: [CeremonyRunner]

    init(_ runners: [CeremonyRunner]) {
        unstorable = runners.filter { !$0.isStorable }
        let storable = runners.filter(\.isStorable)
        reachable = storable.filter { !$0.pending }
        pending = storable.filter(\.pending)
    }

    static let ready = "This device is ready"

    /// Not ``ready`` when nothing landed, which is not a corner: a phone grants
    /// the one runner its live connection reaches, so a single failed write
    /// there is a reply where every runner is pending.
    var headline: String {
        reachable.isEmpty ? "No runners were added" : Self.ready
    }

    /// The line above the list, or nil when there is no list and ``note`` is the
    /// whole message.
    var summary: String? {
        if !reachable.isEmpty { return "This device can access these runners:" }
        return pending.isEmpty ? "The other device didn’t share any runners." : nil
    }

    /// What to say about the runners that did not take the key.
    ///
    /// It NAMES them, which the granting side deliberately does not: over there
    /// the sentence sits beside the list the person just ticked, and here that
    /// list is exactly what is missing — "which ones" is the question. What it
    /// does not do is guess WHY, for the reason
    /// ``CeremonyStore/note(about:outcomes:)`` gives: a runner asleep, a daemon
    /// never installed and a fence that could not be rewritten look identical
    /// from here, and a screen that guesses is how an app ends up telling
    /// somebody to loosen an sshd setting that was never the problem.
    ///
    /// It promises nothing later, either. Nothing retries, so "yet" is as far as
    /// this goes, and the sentence after it is an instruction rather than a wait.
    var note: String? {
        let parts = [tunnelNote, pendingNote].compactMap { $0 }
        return parts.isEmpty ? nil : parts.joined(separator: "\n\n")
    }

    private var pendingNote: String? {
        guard !pending.isEmpty else { return nil }
        let names = ListFormatter.localizedString(byJoining: pending.map(name(of:)))
        return pending.count == 1
            ? "\(names) doesn’t have this device’s key yet. To use it, add this device again "
                + "from one that can reach it."
            : "\(names) don’t have this device’s key yet. To use them, add this device again "
                + "from one that can reach them."
    }

    /// A granted runner this app cannot record is stated, never dropped
    /// silently. It is the same rule as ``pendingNote``: the alternative to
    /// reading it here is meeting it later as a runner that is simply absent
    /// from the list, with nothing on any screen having said so.
    private var tunnelNote: String? {
        guard !unstorable.isEmpty else { return nil }
        let names = ListFormatter.localizedString(byJoining: unstorable.map(name(of:)))
        return unstorable.count == 1
            ? "\(names) is reachable only through a tunnel, which this device can’t use yet."
            : "\(names) are reachable only through a tunnel, which this device can’t use yet."
    }

    /// A label comes from the granting device and is what somebody ticked there,
    /// so it is the name to use. How the runner is reached is the fallback for a
    /// reply that carried none, because a sentence with a blank in it names
    /// nothing.
    private func name(of runner: CeremonyRunner) -> String {
        runner.label.isEmpty ? runner.reach.name(user: runner.user) : runner.label
    }
}
