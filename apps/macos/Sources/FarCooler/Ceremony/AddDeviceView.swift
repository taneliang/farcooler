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
    /// The ceremony's one precondition on this Mac. See ``toolsFirst``.
    @StateObject private var tools = CommandLineTools()
    @ObservedObject private var account = Account.shared
    @ObservedObject private var runners = Runners.shared

    /// Every runner this Mac could grant, resolved once when the sheet opens.
    /// Resolving costs an `ssh -G` each, and doing it while somebody is holding
    /// a phone up to a camera is the wrong moment.
    @State private var grantable: [CeremonyStore.RunnerRow] = []
    /// How far each runner's address travels, and what it was swapped for,
    /// keyed by the runner id the ceremony rows carry.
    ///
    /// Held beside the rows rather than folded into `CeremonyRunner`, because
    /// the wire record is what the other device stores forever and this is a
    /// note about how it was chosen — true only on this Mac, at this moment,
    /// and of no use to anybody reading it later.
    @State private var addressing: [String: RunnerFacts.Addressing] = [:]

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            if tools.state == .installed {
                switch store.phase {
                case .scanning: scanning
                case .confirming(let confirmation): confirming(confirmation)
                case .enrolling: working
                case .showingManifest(let manifest): reply(manifest)
                case .refused(let refusal):
                    RefusalView(refusal: refusal, retry: retry, done: dismissed)
                case .done(let sentence): finished(sentence)
                case .showingOffer: EmptyView()
                }
            } else {
                toolsFirst
            }
        }
        .padding(20)
        .frame(width: 460)
        .task { await prepare() }
        .onChange(of: scanner.scanned?.payload) { _, _ in read() }
        // `prepare` returned without asking for the camera while the tools were
        // missing, so installing them is what has to start the ceremony.
        .onChange(of: tools.state) { _, state in
            if state == .installed { Task { await prepare() } }
        }
    }

    // MARK: - The one precondition

    /// Adding a device is refused until this Mac can be reached by the line it
    /// is about to write.
    ///
    /// Key A's line carries a forced command of
    /// `~/.local/bin/farcoolerd-<channel> --stdio …` — `forced_program` in
    /// `crates/fence/src/lib.rs`, which names the binary and deliberately has no
    /// fallback, because an install has one correct answer. Those links are
    /// this app's to create and they are OPT-IN: the first-launch alert fires at
    /// most once ever, sets its flag the moment it decides to show, and a "Not
    /// Now" is therefore permanent.
    ///
    /// Enrolling anyway wrote a line naming a path that did not exist. sshd ran
    /// nothing, the pipe closed, and the new device reported "Far Cooler isn't
    /// installed" — about THIS Mac, the one machine the person holding the phone
    /// cannot see and did not think they were configuring. Refusing here is the
    /// only place that failure can still be named where it lives.
    private var toolsFirst: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Install the command-line tools first")
                .font(.headline)
            Text(
                "A device you add reaches this Mac by running "
                    + "\(CommandLineTools.tools.map(\.link).joined(separator: " and ")) over SSH, "
                    + "so it can’t connect until Far Cooler has added them to ~/.local/bin."
            )
            .font(.callout)
            .foregroundStyle(.secondary)
            .fixedSize(horizontal: false, vertical: true)

            if let obstacle {
                Text(obstacle)
                    .font(.callout)
                    .foregroundStyle(.orange)
                    .fixedSize(horizontal: false, vertical: true)
            }

            HStack {
                Spacer()
                Button("Cancel", action: dismissed).keyboardShortcut(.cancelAction)
                Button("Install") { tools.install() }
                    .keyboardShortcut(.defaultAction)
                    .disabled(obstacle != nil)
            }
        }
    }

    /// What is in the way, when something is — a path this app will not
    /// overwrite, or a build with no bundle to link into. Both are sentences
    /// already; neither is anything this screen can fix by trying harder.
    private var obstacle: String? {
        switch tools.state {
        case .conflict(let sentence), .unavailable(let sentence): sentence
        case .installed, .notInstalled: nil
        }
    }

    // MARK: - Scanning

    private var scanning: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Scan the new device’s code")
                .font(.headline)
            Text(
                "On the new device, open Far Cooler and choose Add This Device. Then hold its "
                    + "code up to this Mac’s camera."
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
                        Text("Use Far Cooler with the runners you select.")
                            .font(.callout)
                        shellAccess(confirmation)
                    }
                } else {
                    Text(
                        "\(name) can run agents and commands as you on the runners you select."
                    )
                    .font(.callout)
                    .fixedSize(horizontal: false, vertical: true)
                }

                fingerprint(confirmation.offer)

                VStack(alignment: .leading, spacing: 10) {
                    ForEach(confirmation.rows) { row in
                        VStack(alignment: .leading, spacing: 2) {
                            Toggle(isOn: grant(row)) {
                                Text(row.note.map { "\(row.runner.label) · \($0)" } ?? row.runner.label)
                            }
                            reachNote(for: row)
                        }
                    }
                }

                Text(
                    "Far Cooler adds this device’s key to ~/.ssh/authorized_keys on each selected "
                        + "runner. You can change its access later in Settings › Devices."
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
                Text("Allow Zed, Git, and Terminal on that Mac to connect to these runners.")
                    .font(.callout)
                Toggle(isOn: .constant(true)) { Text("New key: \(shell.name)") }
                    .disabled(true)
                Toggle(isOn: .constant(shell.addToConfig)) { Text("Add to ~/.ssh/config") }
                    .disabled(true)
                Text("\(confirmation.offer.name) selected this before showing its code.")
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
            Text("Scan this code with the new device").font(.headline)
            Text("This shares the runners you selected.")
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

            // The sentence comes from what the writes DID — see
            // `Enrollment.note(about:outcome:)` — rather than from the
            // transcript merely existing. A Mac that took Key A everywhere and
            // lost Key B on one runner has a transcript and no pending runner,
            // and "some runners don't have the key" would be false about every
            // runner in the code being shown.
            if let note = store.note {
                Text(note)
                    .font(.callout)
                    .foregroundStyle(.orange)
                    .fixedSize(horizontal: false, vertical: true)
            }
            if let transcript = store.transcript {
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

    /// The address this runner will actually be granted under, and whether it
    /// travels.
    ///
    /// On the row rather than in one paragraph at the bottom, because the fact
    /// is per runner: a fleet is routinely a Mac on the desk and a Linux box
    /// with a real name, and one sentence covering both would be false about
    /// one of them.
    ///
    /// Shown for every runner, not only the bad ones. The address the other
    /// device is about to keep forever has never appeared on this screen at
    /// all, and "which machine is `cosmo` actually" is the question somebody
    /// asks a week later when it stops answering.
    @ViewBuilder
    private func reachNote(for row: CeremonyStore.RunnerRow) -> some View {
        let verdict = addressing[row.runner.id]
        HStack(alignment: .firstTextBaseline, spacing: 6) {
            Text(row.runner.address)
                .font(.system(.caption, design: .monospaced))
                .foregroundStyle(.secondary)
            switch verdict?.reach {
            case .anywhere:
                Text("Works anywhere").font(.caption).foregroundStyle(.secondary)
            case .thisNetwork where verdict?.betterAddress != nil:
                // The swap already happened in `prepare`; this says so, because
                // a name somebody recognizes being replaced by one they do not
                // is exactly the moment to explain rather than to be clever.
                Text("Reachable anywhere over Tailscale")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            case .thisNetwork:
                Text("Only on this network")
                    .font(.caption)
                    .foregroundStyle(.orange)
            case .unknown, .none:
                EmptyView()
            }
        }
        .padding(.leading, 20)

        if verdict?.reach == .thisNetwork, verdict?.betterAddress == nil {
            // Named once, under the rows it is about. Tailscale is the answer
            // this product already recommends everywhere else — `runners.md`
            // says an SSH route to a MagicDNS name gets NAT traversal and
            // stable addressing — and it is the only one that does not require
            // the person to own a domain or a static lease.
            Text(
                "This device will only reach it on your current network. "
                    + "Install Tailscale on both, or give the runner an address that "
                    + "resolves anywhere, in Settings › Runners."
            )
            .font(.caption)
            .foregroundStyle(.secondary)
            .fixedSize(horizontal: false, vertical: true)
            .padding(.leading, 20)
        }
    }

    private func grant(_ row: CeremonyStore.RunnerRow) -> Binding<Bool> {
        Binding(
            get: { row.granted },
            set: { store.setGrant($0, for: row.id) })
    }

    private func prepare() async {
        // Nothing is resolved and the camera is never asked for while the
        // ceremony cannot succeed. A permission prompt raised for a scan that
        // was going to be refused anyway is a prompt people deny for good.
        guard tools.state == .installed else { return }
        let prepared = await Task.detached { () -> ([CeremonyStore.RunnerRow], [String: RunnerFacts.Addressing]) in
            // Once, above the loop: it spawns a process and the answer is the
            // same for every runner. Nil is the ordinary case — most people
            // have no Tailscale — and costs nothing beyond the one lookup.
            let tailnet = await Tailnet.current()
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

            // The substitution, and the whole reason this exists. `thisMac()`
            // reports `ProcessInfo.hostName`, which is normally the `.local`
            // mDNS name — so a phone paired at a desk was handed an address
            // that stops resolving the moment it leaves the building, and this
            // Mac has no way to tell it the new one afterwards. The runner's
            // tailnet name is the same machine reachable from anywhere.
            //
            // Applied rather than offered. There is no reading under which the
            // LAN-only name is the better answer when a travelling one exists
            // for the same host, and a dialog asking which of two addresses a
            // phone should use is a question about DNS asked of somebody who
            // wanted to add a phone. What it does instead is SAY so, per row.
            var judged: [String: RunnerFacts.Addressing] = [:]
            for (index, row) in rows.enumerated() {
                let verdict = RunnerFacts.addressing(of: row.runner, in: tailnet)
                judged[row.runner.id] = verdict
                guard let better = verdict.betterAddress else { continue }
                var swapped = row.runner
                swapped.address = better
                rows[index] = .init(runner: swapped, granted: row.granted, note: row.note)
            }
            return (rows, judged)
        }.value
        grantable = prepared.0
        addressing = prepared.1
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
        // Nil for a phone, and nil for a Mac that chose not to have shell access
        // — the choice was made on that Mac, before it showed its code, and its
        // absence from the offer IS the choice. Read from the offer rather than
        // from `confirmation.shell`, which is what the screen shows about the
        // choice; this is the key material the runner will be handed.
        let keyB = confirmation.offer.key_b
        // ONE id for both keys, and it is what the forced command will carry —
        // so it is what closing this device's sessions later will name, and what
        // makes `client revoke` take both of a Mac's lines in one write.
        //
        // **From KEY A, including for Key B's line.** The id names the DEVICE,
        // not the key; deriving each line's id from its own key would split one
        // Mac into two clients, and no single revoke could take both — which is
        // the removal copy ("this takes that Mac's ssh, git and Zed access away
        // too") turning into a lie the day somebody uses it. iOS derives the
        // same id from the same key at `CeremonyStore.clientId(of:)`.
        //
        // Derived rather than made here, which is what this line used to do
        // (`UUID().uuidString`). Not the ceremony's id — that one is a different
        // thing with a different lifetime, a correlator for two codes a minute
        // apart. This one is written into a runner's `authorized_keys` and
        // outlives every session it names, so it has to be the SAME id the next
        // time this device is added: a device re-running the ceremony against a
        // runner it is already on must land on the id already in that fence
        // instead of enrolling a second line for one device, and a fresh UUID
        // could not do that by construction.
        let clientID = DeviceKey.clientID(of: keyA)

        Task {
            await store.confirm(
                // Named, so the system prompt says what is about to happen
                // rather than "Far Cooler is trying to authenticate".
                reason: "Confirm adding \(name) to your runners"
            ) { granted in
                // No id means the code's key did not parse, so there is nothing
                // to enroll UNDER: no runner is written to and the screen says
                // so. Deliberately not a UUID fallback — that is the bug this
                // change removes, and as a fallback it would be the silent
                // version of it, landing lines in somebody's `authorized_keys`
                // under an id nothing can revoke by name while looking like it
                // worked. iOS takes the same branch in `CeremonyStore.confirm()`
                // and leaves every runner pending. So does this: an outcome
                // with nothing written is what makes every runner in the reply
                // travel pending, which is a true statement about those files.
                guard let clientID else {
                    return .nothingWritten(Enrollment.unreadableKey)
                }
                return await Enrollment.enroll(
                    keyA: keyA, keyB: keyB, label: name, clientID: clientID,
                    // `control`, per the design, and it applies to Key A only:
                    // `read` is a narrowing set afterwards in Settings › Devices,
                    // and only for phones. Key B's scope is host_admin and is not
                    // passed from here, because a plain line cannot be held to a
                    // scope at all — `Enrollment` owns that, so no screen can
                    // accidentally ask for a shell at `read`.
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
