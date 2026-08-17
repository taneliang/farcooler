import AgentKit
import SwiftUI

/// This Mac adding another device: scan its code, pick runners, confirm, show
/// the reply.
///
/// The four moments of the ceremony from the trusted end. Every decision about
/// whether a scanned code is acceptable was made in Rust before this view saw
/// it — what is here is a camera, a list of checkboxes, a fingerprint at the
/// tap, and a code shown back.
struct AddDeviceView: View {
    @Environment(\.dismiss) private var dismiss
    @StateObject private var store = CeremonyStore()
    @StateObject private var scanner = CodeScanner()
    @ObservedObject private var account = Account.shared
    @ObservedObject private var runners = Runners.shared

    /// Every runner this Mac could grant, resolved once when the sheet opens.
    /// Resolving costs an `ssh -G` each, and doing it while somebody is holding
    /// a phone up to a camera is the wrong moment.
    @State private var grantable: [CeremonyStore.RunnerRow] = []

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            switch store.phase {
            case .scanning: scanning
            case .confirming(let confirmation): confirming(confirmation)
            case .enrolling: working
            case .showingManifest(let manifest): reply(manifest)
            case .refused(let refusal): RefusalView(refusal: refusal, retry: retry, done: dismissed)
            case .done(let sentence): finished(sentence)
            case .showingOffer: EmptyView()
            }
        }
        .padding(20)
        .frame(width: 460)
        .task { await prepare() }
        .onChange(of: scanner.scanned?.payload) { _, _ in read() }
    }

    // MARK: - Scanning

    private var scanning: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Scan the code on the device you're adding")
                .font(.headline)
            Text(
                "Open Far Cooler on the new device and choose Add This Device. "
                    + "Point this Mac's camera at the code it shows."
            )
            .font(.callout)
            .foregroundStyle(.secondary)
            .fixedSize(horizontal: false, vertical: true)

            if let problem = scanner.problem {
                Text(problem)
                    .font(.callout)
                    .foregroundStyle(.orange)
                    .fixedSize(horizontal: false, vertical: true)
            } else {
                CameraPreview(session: scanner.session)
                    .frame(height: 260)
                    .clipShape(RoundedRectangle(cornerRadius: 10))
            }

            HStack {
                Spacer()
                Button("Cancel", action: dismissed).keyboardShortcut(.cancelAction)
            }
        }
    }

    // MARK: - The confirmation

    /// The copy here is the spec's, verbatim. It was written and reviewed
    /// there; it is not to be paraphrased into something that reads more
    /// smoothly and promises something else.
    @ViewBuilder
    private func confirming(_ confirmation: CeremonyStore.Confirmation) -> some View {
        let name = confirmation.offer.name
        let isMac = confirmation.shell != nil

        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                Text(
                    isMac
                        ? "Add \"\(name)\"?"
                        : "Add \"\(name)\" to \(account.email)?"
                )
                .font(.title3.weight(.semibold))

                if isMac {
                    VStack(alignment: .leading, spacing: 10) {
                        // Markdown in a `Text` literal, not `Text + Text`: the
                        // latter is deprecated as of macOS 26 and this renders
                        // the same two weights.
                        Text("**Far Cooler access** — run agents and terminals on the runners you pick.")
                            .font(.callout)
                        shellAccess(confirmation)
                    }
                } else {
                    Text(
                        "\(name) will be able to run agents and commands on the runners you pick, "
                            + "as you. Each enrollment is recorded under \(account.email)."
                    )
                    .font(.callout)
                    .fixedSize(horizontal: false, vertical: true)
                }

                fingerprint(confirmation.offer)

                VStack(alignment: .leading, spacing: 6) {
                    ForEach(confirmation.rows) { row in
                        Toggle(isOn: grant(row)) {
                            Text(row.note.map { "\(row.runner.label) · \($0)" } ?? row.runner.label)
                        }
                    }
                }

                Text(
                    "Far Cooler adds this key to ~/.ssh/authorized_keys on each runner you pick, "
                        + "and changes nothing else. "
                        + "You can add or remove runners later in Settings › Devices."
                )
                .font(.caption)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)

                HStack {
                    Spacer()
                    Button("Cancel", action: dismissed).keyboardShortcut(.cancelAction)
                    Button(isMac ? "Add Mac" : "Add Device") { confirm(name: name) }
                        .keyboardShortcut(.defaultAction)
                        .disabled(!confirmation.rows.contains(where: \.granted))
                }
            }
        }
        .frame(maxHeight: 460)
    }

    /// The Key B rows.
    ///
    /// **Shown here, chosen there.** The private half of Key B never leaves the
    /// Mac being added, and neither does its `~/.ssh` — so that Mac makes this
    /// choice before it shows its code, and this end reports what it chose.
    /// Presenting these as editable would be offering to pick a key out of
    /// another machine's home directory.
    @ViewBuilder
    private func shellAccess(_ confirmation: CeremonyStore.Confirmation) -> some View {
        if let shell = confirmation.shell {
            VStack(alignment: .leading, spacing: 4) {
                Text("**Shell access** — Zed, git and Terminal on that Mac reach them too.")
                    .font(.callout)
                Toggle(isOn: .constant(true)) { Text("New key — \(shell.name)") }
                    .disabled(true)
                Toggle(isOn: .constant(shell.addToConfig)) { Text("Add to ~/.ssh/config") }
                    .disabled(true)
                Text("Chosen on \(confirmation.offer.name) before it showed its code.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
    }

    private func fingerprint(_ offer: CeremonyOffer) -> some View {
        DisclosureGroup {
            Text(offer.fingerprint ?? offer.key_a)
                .font(.system(.caption, design: .monospaced))
                .textSelection(.enabled)
                .frame(maxWidth: .infinity, alignment: .leading)
        } label: {
            Text(abbreviated(offer.fingerprint))
                .font(.system(.callout, design: .monospaced).weight(.semibold))
        }
    }

    /// `SHA256:t7Xq…9Vd`. Head and tail, because a fingerprint is compared from
    /// both ends and a middle-truncated one is the only form anybody checks.
    private func abbreviated(_ fingerprint: String?) -> String {
        guard let fingerprint, fingerprint.count > 20 else { return fingerprint ?? "" }
        return "\(fingerprint.prefix(11))…\(fingerprint.suffix(3))"
    }

    // MARK: - After the tap

    private var working: some View {
        VStack(spacing: 12) {
            ProgressView()
            Text("Adding…").font(.callout).foregroundStyle(.secondary)
        }
        .frame(maxWidth: .infinity, minHeight: 200)
    }

    @ViewBuilder
    private func reply(_ manifest: String) -> some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Now scan this with the new device").font(.headline)
            Text("It needs the addresses of the runners you just granted.")
                .font(.callout)
                .foregroundStyle(.secondary)

            if let image = qrImage(manifest) {
                Image(nsImage: image)
                    .resizable()
                    .interpolation(.none)
                    .scaledToFit()
                    .frame(width: 300, height: 300)
                    .frame(maxWidth: .infinity)
            }

            if let transcript = store.transcript {
                Text(Enrollment.couldNotReachAll)
                    .font(.callout)
                    .foregroundStyle(.orange)
                    .fixedSize(horizontal: false, vertical: true)
                DetailBox(text: transcript)
            }

            HStack {
                Spacer()
                Button("Done", action: dismissed).keyboardShortcut(.defaultAction)
            }
        }
    }

    private func finished(_ sentence: String) -> some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(sentence).font(.callout)
            HStack {
                Spacer()
                Button("Done", action: dismissed).keyboardShortcut(.defaultAction)
            }
        }
    }

    // MARK: - Wiring

    private func grant(_ row: CeremonyStore.RunnerRow) -> Binding<Bool> {
        Binding(
            get: { row.granted },
            set: { store.setGrant($0, for: row.id) })
    }

    private func prepare() async {
        grantable = await Task.detached {
            var rows: [CeremonyStore.RunnerRow] = [
                // Only the runner being granted from is checked by default.
                // Everything else is a decision somebody makes on purpose.
                .init(runner: RunnerFacts.thisMac(), granted: true, note: "this Mac")
            ]
            for runner in await Runners.shared.all {
                guard
                    let facts = await RunnerFacts.facts(for: runner.target, label: runner.target)
                else { continue }
                rows.append(.init(runner: facts, granted: false, note: nil))
            }
            return rows
        }.value
        await scanner.start()
    }

    private func read() {
        guard let scanned = scanner.scanned else { return }
        store.scan(
            scanned.payload, at: scanned.at, account: account.userId, email: account.email,
            runners: grantable)
    }

    private func retry() {
        store.scanAgain()
        scanner.rescan()
    }

    private func confirm(name: String) {
        // Read out of the phase HERE, not inside the closure. `store.confirm`
        // moves to `.enrolling` before it calls back, so a closure that went
        // looking for the offer then would find none and enroll an empty key.
        guard case .confirming(let confirmation) = store.phase else { return }
        let keyA = confirmation.offer.key_a
        // One id per enrollment, and it is what the forced command will carry —
        // so it is what closing this device's sessions later will name. Made
        // here because nothing else has made one: the ceremony correlates by
        // its own id, which is a different thing with a different lifetime.
        let clientID = UUID().uuidString

        Task {
            await store.confirm(
                // Named, so the system prompt says what is about to happen
                // rather than "Far Cooler is trying to authenticate".
                reason: "Confirm adding \(name) to your runners"
            ) { granted in
                await Enrollment.enroll(
                    key: keyA, label: name, clientID: clientID,
                    // `control`, per the design: `read` is a narrowing set
                    // afterwards in Settings › Devices, and only for phones —
                    // a shell key cannot be held to a scope at all.
                    scope: "control", on: granted)
            }
        }
    }

    private func dismissed() {
        scanner.stop()
        dismiss()
    }
}

/// A refused code, in this app's words.
struct RefusalView: View {
    let refusal: Refusal
    let retry: () -> Void
    let done: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(refusal.title).font(.headline)
            Text(refusal.detail)
                .font(.callout)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            HStack {
                Spacer()
                if refusal.retryable {
                    Button("Try Again", action: retry)
                }
                Button("Done", action: done).keyboardShortcut(.defaultAction)
            }
        }
    }
}
