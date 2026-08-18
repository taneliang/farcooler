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
        VStack(alignment: .leading, spacing: 16) {
            Text("This device is ready")
                .font(.title3.weight(.semibold))

            Text("This device can access these runners:")
            .font(.callout)
            .foregroundStyle(.secondary)

            ForEach(store.granted) { runner in
                VStack(alignment: .leading, spacing: 2) {
                    Text(runner.label)
                    Text("\(runner.user)@\(runner.address)")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                .padding(.vertical, 4)
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
    /// Matched by address, user and port rather than by the id in the manifest:
    /// two devices generate their own ids for the same runner, so adopting by
    /// id would leave someone with the same box listed twice.
    private func adopt() {
        for entry in store.granted {
            let existing = runners.hosts.first {
                $0.address == entry.address && $0.user == entry.user && $0.port == entry.port
            }
            if let existing {
                // Already known. The pin is the one thing worth taking from the
                // manifest, and only when this device has none of its own: a
                // fingerprint someone approved here outranks one that arrived
                // in a code.
                if existing.fingerprint == nil, !entry.host_key.isEmpty {
                    var updated = existing
                    updated.fingerprint = entry.host_key
                    runners.update(updated)
                }
            } else {
                runners.add(entry.asRunner)
            }
        }
    }
}
