import SwiftUI

/// Adding a device from the one you already trust: scan its code, pick the
/// runners it may reach, confirm, and show the reply back.
///
/// The order of the screens is the security argument. A code from another
/// account reaches the mismatch screen and stops there — the runner list is not
/// behind it, because a list of addresses on screen with only a fingerprint
/// between a stranger and them is the thing this flow exists to avoid.
struct AddDeviceView: View {
    @ObservedObject var runners: RunnerStore

    @StateObject private var store: CeremonyStore
    @StateObject private var scanner = CodeScanner()
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
            .navigationTitle("Add a Device")
            .navigationBarTitleDisplayMode(.inline)
            .onAppear { if store.phase == .idle { store.beginScanning() } }
            .onChange(of: scanner.scanned) { _, payload in
                guard let payload else { return }
                store.read(payload, runners: runners.hosts, grantingFrom: runners.selected)
            }
            // A declined gate stops here and says so. It does not fall back to
            // adding the device, and it does not explain what the gate wanted —
            // the sentence a person needs is that nothing happened.
            .alert("Couldn’t verify your identity", isPresented: $store.declined) {
                Button("OK") { store.declined = false }
            } message: {
                Text("No device was added.")
            }
    }

    @ViewBuilder
    private var content: some View {
        switch store.phase {
        case .idle, .scanning:
            ScanScreen(
                scanner: scanner,
                instruction: "Scan the code shown on the device you’re adding.",
                onCancel: { dismiss() })

        case .mismatch:
            mismatch

        case .confirming:
            confirmation

        case .enrolling:
            waiting

        case .showingManifest:
            reply

        case .refused(let refusal):
            RefusalScreen(refusal: refusal, retry: "Scan Again") {
                scanner.start()
                store.beginScanning()
            } onDone: {
                dismiss()
            }

        case .done, .showingOffer:
            // Nothing on this side of the ceremony produces either, and a
            // screen that cannot be reached is better left empty than filled
            // with something invented for it.
            ProgressView()
        }
    }

    // MARK: - That device is signed into a different account

    private var mismatch: some View {
        VStack(spacing: 16) {
            Spacer()
            Text("This device uses a different account")
                .font(.title3.weight(.semibold))
                .multilineTextAlignment(.center)

            Text(
                "Sign in to \(store.accountEmail) on the new device, then show its code again."
            )
            .font(.callout)
            .foregroundStyle(.secondary)
            .multilineTextAlignment(.center)

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

    // MARK: - The confirmation

    private var confirmation: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 18) {
                Text("Add “\(store.offer?.name ?? "this device")”?")
                    .font(.title3.weight(.semibold))

                Text(
                    "\(store.offer?.name ?? "This device") can run agents and commands as you "
                        + "on the runners you select."
                )
                .font(.callout)
                .foregroundStyle(.secondary)

                if let fingerprint = store.fingerprint {
                    FingerprintRow(fingerprint: fingerprint, key: store.offer?.key_a ?? "")
                }

                VStack(alignment: .leading, spacing: 0) {
                    ForEach($store.rows) { $row in
                        Button {
                            row.picked.toggle()
                        } label: {
                            HStack(alignment: .firstTextBaseline, spacing: 10) {
                                Image(systemName: row.picked ? "checkmark.square.fill" : "square")
                                    .foregroundStyle(row.picked ? Color.accentColor : .secondary)
                                VStack(alignment: .leading, spacing: 2) {
                                    Text(row.label)
                                    Text(row.detail)
                                        .font(.caption)
                                        .foregroundStyle(.secondary)
                                }
                                Spacer()
                            }
                            .padding(.vertical, 8)
                        }
                        .buttonStyle(.plain)
                    }
                }

                // "Will try" rather than "adds", which is what it said while
                // writing nothing at all. It writes now — but only where this
                // phone is connected AND its own key carries `host_admin`, and
                // a phone is normally enrolled at `control` precisely so a
                // device cannot widen its own access. Runners it could not
                // write to still travel, and the next screen names them.
                Text(
                    "Far Cooler will try to add this device’s key to ~/.ssh/authorized_keys on "
                        + "each selected runner. You can change its access later in "
                        + "Settings › Devices."
                )
                .font(.footnote)
                .foregroundStyle(.secondary)

                VStack(spacing: 12) {
                    Button {
                        Task { await store.confirm() }
                    } label: {
                        Text("Add Device").frame(maxWidth: .infinity)
                    }
                    .buttonStyle(.borderedProminent)
                    .controlSize(.large)
                    .disabled(store.rows.allSatisfy { !$0.picked })

                    Button("Cancel") { dismiss() }
                }
                .padding(.top, 4)
            }
            .padding()
        }
    }

    private var waiting: some View {
        VStack(spacing: 14) {
            ProgressView()
            Text("Adding…").font(.callout).foregroundStyle(.secondary)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    // MARK: - The reply

    private var reply: some View {
        VStack(spacing: 18) {
            Text("Scan this code with \(store.offer?.name ?? "the new device")")
                .font(.headline)

            CodeImageView(payload: store.code)
                .frame(maxWidth: 360)

            Text("This shares the runners you selected.")
            .font(.footnote)
            .foregroundStyle(.secondary)
            .multilineTextAlignment(.center)

            // The half that did not happen, named on the screen where it still
            // matters. A runner whose key was not written hands the new device
            // an address it cannot log in to, and finding that out later — as a
            // connection refused, on the other device — is the worst place to
            // find it out. `pending` has always carried this fact on the wire;
            // nothing on this side had ever said it out loud.
            if let note = store.pendingNote {
                Text(note)
                    .font(.footnote)
                    .foregroundStyle(.orange)
                    .multilineTextAlignment(.center)
                    .fixedSize(horizontal: false, vertical: true)
            }

            Button {
                dismiss()
            } label: {
                Text("Done").frame(maxWidth: .infinity)
            }
            .buttonStyle(.borderedProminent)
            .controlSize(.large)
            .padding(.top, 8)
        }
        .padding(24)
    }
}

/// The key being granted, short enough to read out and expandable to the whole
/// thing.
///
/// The short form is what two people compare across a desk; the full key is
/// what somebody pastes into a bug report. Neither is a decision — the target
/// check that matters is made in Rust, against a fingerprint it computes
/// itself.
struct FingerprintRow: View {
    let fingerprint: String
    let key: String

    var body: some View {
        DisclosureGroup {
            Text(key.isEmpty ? fingerprint : key)
                .font(.system(.caption, design: .monospaced))
                .textSelection(.enabled)
                .padding(.top, 6)
        } label: {
            Text(Self.abbreviated(fingerprint))
                .font(.system(.footnote, design: .monospaced))
        }
    }

    /// `SHA256:t7Xq…9Vd` — the ends, which is what a person compares, and none
    /// of the middle, which nobody does.
    static func abbreviated(_ fingerprint: String) -> String {
        let body = fingerprint.hasPrefix("SHA256:")
            ? String(fingerprint.dropFirst("SHA256:".count)) : fingerprint
        guard body.count > 10 else { return fingerprint }
        return "SHA256:\(body.prefix(4))…\(body.suffix(3))"
    }
}

/// A refusal, said in the app's own words.
///
/// Every sentence here comes from `Refusal`, which maps the core's stable word
/// to copy. No Rust error string reaches this screen, and nothing on it
/// suggests loosening anything.
struct RefusalScreen: View {
    let refusal: Refusal
    var retry: String?
    var onRetry: (() -> Void)?
    let onDone: () -> Void

    var body: some View {
        VStack(spacing: 16) {
            Spacer()
            Image(systemName: "exclamationmark.triangle")
                .font(.system(size: 34, weight: .thin))
                .foregroundStyle(.tertiary)

            Text(refusal.title)
                .font(.title3.weight(.semibold))
                .multilineTextAlignment(.center)

            Text(refusal.message)
                .font(.callout)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)

            Spacer()

            VStack(spacing: 12) {
                if let retry, let onRetry {
                    Button {
                        onRetry()
                    } label: {
                        Text(retry).frame(maxWidth: .infinity)
                    }
                    .buttonStyle(.borderedProminent)
                    .controlSize(.large)
                }
                Button("Done", action: onDone)
            }
        }
        .padding(24)
    }
}
