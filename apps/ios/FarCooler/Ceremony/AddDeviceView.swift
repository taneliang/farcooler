import SwiftUI

/// Adding a device from the one you already trust: scan its code, pick the
/// runners it may reach, confirm, and show the reply back.
///
/// The order of the screens is the security argument. A code from another
/// account is held while the team directory says whether it is a teammate's
/// (ov-854), and stops at a refusal if it isn't — the runner list is not behind
/// it, because a list of addresses on screen with only a fingerprint between a
/// stranger and them is the thing this flow exists to avoid. A teammate's has
/// its own confirmation: who they are, what they may do, and the runners.
struct AddDeviceView: View {
    @ObservedObject var runners: RunnerStore

    @StateObject private var store: CeremonyStore
    @StateObject private var scanner = CodeScanner()
    @Environment(\.dismiss) private var dismiss

    init(runners: RunnerStore) {
        self.init(
            runners: runners,
            store: CeremonyStore(
                account: Account.shared.userId,
                accountEmail: Account.shared.email,
                deviceName: UIDevice.current.name))
    }

    /// With a store of the caller's: the harness answers the directory, the
    /// gate and the runners (`TeammateAddHarness`).
    init(runners: RunnerStore, store: CeremonyStore) {
        self.runners = runners
        _store = StateObject(wrappedValue: store)
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

        case .checkingTeam:
            checkingTeam

        case .confirming:
            if let teammate = store.teammate {
                teammateConfirmation(teammate)
            } else {
                confirmation
            }

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

        case .done, .showingOffer, .confirmingJoin:
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

    // MARK: - Is it a teammate's?

    /// A code from another account is held here while the team directory is
    /// asked (ov-854). Nothing about the code is shown until the core says yes.
    private var checkingTeam: some View {
        VStack(spacing: 14) {
            ProgressView()
            Text("Checking your teams…").font(.callout).foregroundStyle(.secondary)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("teammate-checking")
    }

    // MARK: - A teammate's device (ov-854)

    /// The runners being added to, or every runner this phone may add to
    /// while none is ticked yet.
    private var addingTo: [RunnerRow] {
        let picked = store.rows.filter(\.picked)
        return picked.isEmpty ? store.rows.filter { $0.blocked == nil } : picked
    }

    /// The people this device could be added to, from the runners being added
    /// to: the one under this account and any added by hand with none yet.
    private var candidates: [(person: ExistingPerson, runner: String)] {
        var seen = Set<String>()
        return addingTo.flatMap { row in
            (store.facts[row.runner.id.uuidString]?.candidates ?? []).map { ($0, row.label) }
        }.filter { seen.insert($0.0.handle).inserted }
    }

    /// What the Access menu offers: Admin only when this phone is the Owner
    /// on every runner being added to.
    private var accessChoices: [PersonAccess] {
        TeammateAccessRules.choices(callers: addingTo.map { store.facts[$0.runner.id.uuidString]?.caller })
    }

    /// The confirmation for a teammate's device, in the Mac's words: who the
    /// directory says they are, the key to compare in person, what they'll be
    /// called and may do, and the runners.
    ///
    /// The email is the directory's, never the code's: the code only named an
    /// account, and anyone can show a code naming any account. What stops a
    /// stranger is the person adding them checking it's really them, and that
    /// the key matches the one under their code.
    private func teammateConfirmation(_ teammate: TeammateChoice) -> some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 18) {
                Text("Add Your Teammate’s Device?")
                    .font(.title3.weight(.semibold))
                    .accessibilityIdentifier("teammate-title")

                Text(
                    "“\(store.offer?.name ?? "This device")” is signed in as \(teammate.who.email), "
                        + "who’s on your team “\(teammate.who.team)”. Add it only if they’re with you and "
                        + "the key below matches the one under their code."
                )
                .font(.callout)
                .foregroundStyle(.secondary)
                .accessibilityIdentifier("teammate-subtitle")

                if let fingerprint = store.fingerprint {
                    FingerprintRow(fingerprint: fingerprint, key: store.offer?.key_a ?? "", opened: true)
                }

                TeammateFields(
                    teammate: teammate, candidates: candidates, choices: accessChoices
                ) { change in store.setTeammate(change) }

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
                                    Text(row.blocked ?? row.detail)
                                        .font(.caption)
                                        .foregroundStyle(.secondary)
                                        .fixedSize(horizontal: false, vertical: true)
                                }
                                Spacer()
                            }
                            .padding(.vertical, 8)
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                        .disabled(row.blocked != nil)
                        .opacity(row.blocked == nil ? 1 : 0.55)
                        .accessibilityIdentifier("teammate-runner-\(row.label)")
                    }
                }

                Text(
                    "Far Cooler adds this device’s key to ~/.ssh/authorized_keys on each selected runner and "
                        + "records it as your teammate’s. Work there runs on the Claude account signed in on "
                        + "that runner, so its usage and limits are shared."
                )
                .font(.footnote)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)

                VStack(spacing: 12) {
                    Button {
                        Task { await store.confirmTeammate() }
                    } label: {
                        Text("Add Teammate").frame(maxWidth: .infinity)
                    }
                    .buttonStyle(.borderedProminent)
                    .controlSize(.large)
                    .disabled(!store.rows.contains(where: \.picked) || !teammate.isComplete)
                    .accessibilityIdentifier("teammate-add")

                    Button("Cancel") { dismiss() }
                        .accessibilityIdentifier("teammate-cancel")
                }
                .padding(.top, 4)
            }
            .padding()
        }
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
                        + "Settings > Devices."
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

            // A browser's offer: the reply went to it sealed, and this is the
            // code to match against its screen (ov-770). Nothing otherwise.
            SealedReplySection(reply: store.sealedReply, deviceName: store.offer?.name ?? "the new device")

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
            // Runners the code had no room to give Tailcat (ov-485 N1).
            if let note = store.addressOnlyNote {
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
    /// Open to start with: a teammate's key is the one thing to compare, so
    /// comparing it is not left behind a tap (ov-854).
    var opened = false

    @State private var expanded = false

    var body: some View {
        DisclosureGroup(isExpanded: $expanded) {
            Text(key.isEmpty ? fingerprint : key)
                .font(.system(.caption, design: .monospaced))
                .textSelection(.enabled)
                .padding(.top, 6)
        } label: {
            Text(Self.abbreviated(fingerprint))
                .font(.system(.footnote, design: .monospaced))
        }
        .onAppear { expanded = opened }
    }

    /// The ends, which is what a person compares, and none of the middle,
    /// which nobody does: the form every platform shows (`KeyFingerprint`).
    static func abbreviated(_ fingerprint: String) -> String {
        KeyFingerprint.short(fingerprint)
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

/// Who the teammate is and what they may do (ov-854, ov-856): the person (one
/// a runner already has, or a New Person), then the Access, as the Mac's
/// `TeammateFields` has it.
struct TeammateFields: View {
    let teammate: TeammateChoice
    let candidates: [(person: ExistingPerson, runner: String)]
    let choices: [PersonAccess]
    let change: ((inout TeammateChoice) -> Void) -> Void

    private var chosen: (person: ExistingPerson, runner: String)? {
        candidates.first { $0.person.handle == teammate.person }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            if !candidates.isEmpty {
                Picker("Person", selection: bind(\.person)) {
                    ForEach(candidates, id: \.person.handle) { Text($0.person.name).tag(String?.some($0.person.handle)) }
                    Text("New Person…").tag(String?.none)
                }
                .pickerStyle(.menu)
                .accessibilityIdentifier("teammate-person")
            }
            if chosen == nil {
                LabeledContent("Name") {
                    TextField("Name", text: bind(\.name))
                        .multilineTextAlignment(.trailing)
                        .accessibilityIdentifier("teammate-name")
                }
                LabeledContent("Handle") {
                    TextField("Handle", text: bind(\.handle))
                        .multilineTextAlignment(.trailing)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .font(.system(.body, design: .monospaced))
                        .accessibilityIdentifier("teammate-handle")
                }
                if !TeammateNames.isValidHandle(teammate.handle) {
                    Text("Use lowercase letters, numbers, and hyphens, starting with a letter.")
                        .font(.caption)
                        .foregroundStyle(.orange)
                        .fixedSize(horizontal: false, vertical: true)
                        .accessibilityIdentifier("teammate-handle-note")
                }
                accessPicker
            } else if let chosen {
                Text(TeammateAccessRules.keeps(chosen.person, runner: chosen.runner))
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                    .accessibilityIdentifier("teammate-access-detail")
            }
        }
    }

    @ViewBuilder private var accessPicker: some View {
        Picker("Access", selection: bind(\.access)) {
            ForEach(choices) { access in Text(access.title).tag(access) }
        }
        .pickerStyle(.segmented)
        .accessibilityIdentifier("teammate-access")
        Text(teammate.access.summary)
            .font(.caption)
            .foregroundStyle(.secondary)
            .fixedSize(horizontal: false, vertical: true)
            .accessibilityIdentifier("teammate-access-detail")
    }

    private func bind<Value>(_ path: WritableKeyPath<TeammateChoice, Value>) -> Binding<Value> {
        Binding(get: { teammate[keyPath: path] }, set: { value in change { $0[keyPath: path] = value } })
    }
}
