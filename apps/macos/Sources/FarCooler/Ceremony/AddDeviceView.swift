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
    @StateObject private var store: CeremonyStore
    @StateObject private var scanner = CodeScanner()
    /// The ceremony's one precondition on this Mac. See ``toolsFirst``.
    @StateObject private var tools = CommandLineTools()
    @ObservedObject private var account = Account.shared
    @ObservedObject private var runners = Runners.shared

    /// Every runner this Mac could grant, resolved once when the sheet opens.
    /// Resolving costs an `ssh -G` each, and doing it while somebody is holding
    /// a phone up to a camera is the wrong moment.
    @State private var grantable: [CeremonyStore.RunnerRow] = []
    /// Whether the offer's fingerprint is shown in full.
    @State private var fingerprintOpen = false
    /// How far each runner's address travels, and what it was swapped for,
    /// keyed by the runner id the ceremony rows carry.
    ///
    /// Held beside the rows rather than folded into `CeremonyRunner`, because
    /// the wire record is what the other device stores forever and this is a
    /// note about how it was chosen — true only on this Mac, at this moment,
    /// and of no use to anybody reading it later.
    @State private var addressing: [String: RunnerFacts.Addressing] = [:]
    /// A capture drawing a store it filled itself: no camera, no runner
    /// lookups, and the tools taken as installed.
    private let drawnForCapture: Bool

    init() {
        _store = StateObject(wrappedValue: CeremonyStore())
        drawnForCapture = false
    }

    /// The sheet over `store` as it stands, for a capture (ov-525).
    init(drawing store: CeremonyStore) {
        _store = StateObject(wrappedValue: store)
        drawnForCapture = true
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            if drawnForCapture || tools.state == .installed {
                switch store.phase {
                case .scanning: scanning
                case .confirming(let confirmation):
                    if let teammate = confirmation.teammate {
                        confirmingTeammate(confirmation, teammate)
                    } else {
                        confirming(confirmation)
                    }
                case .checkingTeam: checkingTeam
                case .enrolling: working
                case .showingManifest(let manifest): reply(manifest)
                case .refused(let refusal):
                    RefusalView(refusal: refusal, retry: retry, done: dismissed)
                case .done(let sentence): finished(sentence)
                case .showingOffer, .confirmingJoin: EmptyView()
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
            SheetHeader(
                title: "Install the Command-Line Tools First",
                subtitle: "A device you add reaches this Mac by running "
                    + "\(CommandLineTools.tools.map(\.link).joined(separator: " and ")) over SSH, "
                    + "so it can’t connect until Far Cooler has added them to ~/.local/bin.")

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
            SheetHeader(
                title: "Scan the New Device’s Code",
                subtitle: "On the new device, open Far Cooler and choose Add This Device. Then hold its "
                    + "code up to this Mac’s camera.")

            if let problem = scanner.problem {
                Text(problem)
                    .font(.callout)
                    .foregroundStyle(.orange)
                    .fixedSize(horizontal: false, vertical: true)
            } else {
                CameraPreview(session: scanner.session)
                    .frame(height: 260)
                    .clipShape(.card)
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
        // Whether the new device can use Tailcat at all: an offer with no node
        // key is an older device, granted at addresses only.
        let tunnel = !(confirmation.offer.node_key ?? "").isEmpty
        // The runners one code has no room to give Tailcat (ov-485 N1), warned
        // about here, before anything is written.
        let addressOnly =
            tunnel
            ? CeremonyStore.predictedAddressOnly(
                offer: confirmation.offerJSON, rows: confirmation.rows, budget: codeBudgetBytes)
            : []

        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                SheetHeader(
                    title: isMac
                        ? "Add “\(name)”?"
                        : "Add “\(name)” to \(account.email)?")

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
                            reachNote(
                                for: row, tunnel: tunnel && !addressOnly.contains(row.runner.id),
                                addressOnly: addressOnly.contains(row.runner.id))
                        }
                    }
                }

                // The one picture, once, under the rows it is about: the route
                // is Tailcat, and the address on each row is kept for the day
                // Tailcat can't connect (ov-485).
                if tunnel { TailcatPairingNote(name: name) }

                Text(
                    "Far Cooler adds this device’s key to ~/.ssh/authorized_keys on each selected "
                        + "runner. You can change its access later in Settings > Devices."
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

    // MARK: - A teammate's device (ov-525)

    /// A code from another account, while the team directory is asked about
    /// it. Nothing about the code is shown until the core has said yes.
    private var checkingTeam: some View {
        VStack(spacing: 12) {
            ProgressView()
            Text("Checking your teams…").font(.callout).foregroundStyle(.secondary)
        }
        .frame(maxWidth: .infinity, minHeight: 200)
    }

    /// The confirmation for a teammate's device: who the directory says they
    /// are, the key to compare in person, what they'll be called and may do,
    /// and the runners (ruling R-65).
    ///
    /// The email is the directory's, never the code's: the code only named an
    /// account, and anyone can show a code naming any account. What stops a
    /// stranger is the person adding them checking it's really them, and that
    /// the key matches the one under their code.
    @ViewBuilder
    private func confirmingTeammate(
        _ confirmation: CeremonyStore.Confirmation, _ teammate: CeremonyStore.TeammateChoice
    ) -> some View {
        let name = confirmation.offer.name
        let tunnel = !(confirmation.offer.node_key ?? "").isEmpty
        let addressOnly =
            tunnel
            ? CeremonyStore.predictedAddressOnly(
                offer: confirmation.offerJSON, rows: confirmation.rows, budget: codeBudgetBytes,
                grantedBy: teammate.grantedBy)
            : []

        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                SheetHeader(
                    title: "Add Your Teammate’s Device?",
                    subtitle: "“\(name)” is signed in as \(teammate.who.email), who’s on your team "
                        + "“\(teammate.who.team)”. Add it only if they’re with you and the key below matches "
                        + "the one under their code.")

                fingerprint(confirmation.offer)
                    // Open: comparing it is what the sentence above asks for.
                    .onAppear { fingerprintOpen = true }

                // Held after their account's trust reset (R-76): in orange,
                // since adding it is the admin vouching for it.
                if !teammate.held.isEmpty {
                    Text(CeremonyHold.sentence(teammate.held, email: teammate.who.email))
                        .font(.callout)
                        .foregroundStyle(.orange)
                        .fixedSize(horizontal: false, vertical: true)
                }

                TeammateFields(teammate: teammate) { change in store.setTeammate(change) }

                VStack(alignment: .leading, spacing: 10) {
                    ForEach(confirmation.rows) { row in
                        VStack(alignment: .leading, spacing: 2) {
                            Toggle(isOn: grant(row)) {
                                Text(row.note.map { "\(row.runner.label) · \($0)" } ?? row.runner.label)
                            }
                            reachNote(
                                for: row, tunnel: tunnel && !addressOnly.contains(row.runner.id),
                                addressOnly: addressOnly.contains(row.runner.id))
                        }
                    }
                }

                if tunnel { TailcatPairingNote(name: name) }

                Text(
                    "Far Cooler adds this device’s key to ~/.ssh/authorized_keys on each selected runner and "
                        + "records it as your teammate’s. Work there runs on the Claude account signed in on "
                        + "that runner, so its usage and limits are shared."
                )
                .font(.caption)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            }
        }
        .frame(maxHeight: 520)

        // Outside the scroll view, so Add Teammate is never scrolled away.
        HStack {
            Spacer()
            Button("Cancel", action: dismissed).keyboardShortcut(.cancelAction)
            Button("Add Teammate") { confirm(name: name) }
                .keyboardShortcut(.defaultAction)
                .disabled(!confirmation.rows.contains(where: \.granted) || !teammate.isComplete)
        }
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
                // Plain statements, not disabled switches: this end reports what
                // the other Mac chose and cannot change it, and a switch that
                // does nothing when clicked reads as broken.
                Label("New key: \(shell.name)", systemImage: "checkmark.circle.fill")
                    .font(.callout)
                    .foregroundStyle(.secondary)
                if shell.addToConfig {
                    Label("Adds to ~/.ssh/config", systemImage: "checkmark.circle.fill")
                        .font(.callout)
                        .foregroundStyle(.secondary)
                }
                Text("\(confirmation.offer.name) selected this before showing its code.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
    }

    private func fingerprint(_ offer: CeremonyOffer) -> some View {
        // The shared section (ov-101), where a `DisclosureGroup` was.
        CollapsibleSection(
            id: "fingerprint", metrics: .inline, isExpanded: $fingerprintOpen,
            accessibilityLabel: "Fingerprint \(abbreviated(offer.fingerprint))",
            label: { _ in
                Text(abbreviated(offer.fingerprint))
                    .font(.system(.callout, design: .monospaced).weight(.semibold))
            },
            accessory: { EmptyView() }
        ) {
            Text(offer.fingerprint ?? offer.key_a)
                .font(.system(.caption, design: .monospaced))
                .textSelection(.enabled)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    /// Head and tail, because a fingerprint is compared from both ends and a
    /// middle-truncated one is the only form anybody checks: the same form
    /// the device being added shows under its code (`KeyFingerprint`).
    private func abbreviated(_ fingerprint: String?) -> String {
        fingerprint.map(KeyFingerprint.short) ?? ""
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
            SheetHeader(
                title: "Scan This Code with the New Device",
                subtitle: "This shares the runners you selected.")

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
            // Under the warning and above the transcript, in the ordinary
            // secondary color: nothing went wrong here. It says which route
            // the new device will take, now that the writes have answered.
            if let addressOnlyNote = store.addressOnlyNote {
                Text(addressOnlyNote)
                    .font(.callout)
                    .foregroundStyle(.orange)
                    .fixedSize(horizontal: false, vertical: true)
            }
            if let tunnelNote = store.tunnelNote {
                Text(tunnelNote)
                    .font(.callout)
                    .foregroundStyle(.secondary)
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

    /// The address this runner is granted under, and how far it reaches.
    ///
    /// On the row rather than in one paragraph at the bottom, because the fact
    /// is per runner, and shown for every runner: "which machine is `cosmo`
    /// actually" is the question somebody asks a week later.
    ///
    /// **With Tailcat, the address is the alternative**, so how far it travels
    /// matters only on the day the tunnel fails, and the row says nothing about
    /// it (ov-485). Without Tailcat, an older device, the address is the only
    /// route, and one that stops at this network is said in orange with what
    /// fixes it: updating that device, not installing a VPN.
    @ViewBuilder
    private func reachNote(
        for row: CeremonyStore.RunnerRow, tunnel: Bool, addressOnly: Bool = false
    ) -> some View {
        let verdict = tunnel ? nil : addressing[row.runner.id]
        HStack(alignment: .firstTextBaseline, spacing: 6) {
            Text(row.runner.reach.display)
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

        if addressOnly {
            Text(Self.noRoomForTailcat)
                .font(.caption)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
                .padding(.leading, 20)
        } else if verdict?.reach == .thisNetwork, verdict?.betterAddress == nil {
            Text(Self.updateForTailcat)
                .font(.caption)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
                .padding(.leading, 20)
        }
    }

    /// Under the rows, for a device that can use Tailcat.
    static func tunnelFirst(_ name: String) -> String {
        "\(name) will connect through Tailcat, Far Cooler’s tunnel, which works from any network. "
            + "If Tailcat can’t connect, it’ll offer the address shown instead."
    }

    /// Under a row one code has no room to give Tailcat (ov-485 N1).
    static let noRoomForTailcat =
        "There isn’t room in one code for this runner’s Tailcat route, so it’ll connect at this "
        + "address. Add it again on its own to use Tailcat."

    /// Under a LAN-only row, for a device too old to use Tailcat.
    static let updateForTailcat =
        "This device will only reach it on your current network. Update Far Cooler on it and "
        + "add it again to connect from anywhere."

    private func grant(_ row: CeremonyStore.RunnerRow) -> Binding<Bool> {
        Binding(
            get: { row.granted },
            set: { store.setGrant($0, for: row.id) })
    }

    private func prepare() async {
        // Nothing is resolved and the camera is never asked for while the
        // ceremony cannot succeed. A permission prompt raised for a scan that
        // was going to be refused anyway is a prompt people deny for good.
        guard tools.state == .installed, !drawnForCapture else { return }
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
                swapped.reach = swapped.reach.replacingHost(better)
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
        // The new device's tailcat node public key, read out of the offer it
        // showed. Empty for a v=1 device, for a phone whose own mint failed and
        // for a Mac that could not make one (``MacNodeIdentity``) — all of
        // which enroll exactly as they always have.
        //
        // Read HERE with `keyA` and `keyB`, for the same reason they are:
        // `store.confirm` moves to `.enrolling` before it calls back, so a
        // closure that went looking for the offer then would find none.
        let nodeKey = confirmation.offer.node_key ?? ""
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

        // A teammate's device (ov-525): Key A only, at the scope chosen, and
        // recorded as theirs on every runner that took it. Never Key B: the
        // core refused a teammate's code that asked for one, and nothing here
        // passes one on.
        if let teammate = confirmation.teammate {
            let member = TeammateEnrollment.Member(
                handle: teammate.handle, name: teammate.name.trimmingCharacters(in: .whitespaces),
                account: teammate.who.account, email: teammate.who.email)
            let scope = teammate.scope
            Task {
                await store.confirm(reason: "Confirm adding \(name) for \(teammate.who.email)") { granted in
                    guard let clientID else { return .nothingWritten(Enrollment.unreadableKey) }
                    return await TeammateEnrollment.enroll(
                        keyA: keyA, label: name, clientID: clientID, scope: scope, nodeKey: nodeKey,
                        member: member, on: granted)
                }
            }
            return
        }

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
                    // `read` is a narrowing set afterwards in Settings > Devices,
                    // and only for phones. Key B's scope is host_admin and is not
                    // passed from here, because a plain line cannot be held to a
                    // scope at all — `Enrollment` owns that, so no screen can
                    // accidentally ask for a shell at `read`.
                    scope: "control", nodeKey: nodeKey, on: granted)
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

/// The line under the runners on Add Device for a device that can use
/// Tailcat (ov-485). Its own view so a capture draws the real one.
struct TailcatPairingNote: View {
    let name: String

    var body: some View {
        Text(AddDeviceView.tunnelFirst(name))
            .font(.caption)
            .foregroundStyle(.secondary)
            .fixedSize(horizontal: false, vertical: true)
    }
}

/// What a teammate will be called on the runners, and what they may do there
/// (ruling R-65): a name and a handle suggested from their email, both
/// editable, and Control or Read, never a shell.
struct TeammateFields: View {
    let teammate: CeremonyStore.TeammateChoice
    let change: ((inout CeremonyStore.TeammateChoice) -> Void) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            LabeledContent("Name") {
                TextField("Name", text: bind(\.name))
                    .labelsHidden()
                    .textFieldStyle(.roundedBorder)
            }
            LabeledContent("Handle") {
                TextField("Handle", text: bind(\.handle))
                    .labelsHidden()
                    .textFieldStyle(.roundedBorder)
                    .font(.system(.body, design: .monospaced))
            }
            if !TeammateNames.isValidHandle(teammate.handle) {
                Text("Use lowercase letters, numbers, and hyphens, starting with a letter.")
                    .font(.caption)
                    .foregroundStyle(.orange)
                    .fixedSize(horizontal: false, vertical: true)
            }
            LabeledContent("Access") {
                Picker("Access", selection: bind(\.scope)) {
                    ForEach(TeammateScope.allCases) { scope in Text(scope.title).tag(scope) }
                }
                .labelsHidden()
                .pickerStyle(.segmented)
                .fixedSize()
            }
            Text(teammate.scope.detail)
                .font(.caption)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
        }
        // The fields reach the sheet's trailing edge, where the scroll view
        // clips: this keeps their borders whole.
        .padding(.trailing, 2)
    }

    private func bind<Value>(_ path: WritableKeyPath<CeremonyStore.TeammateChoice, Value>) -> Binding<Value> {
        Binding(get: { teammate[keyPath: path] }, set: { value in change { $0[keyPath: path] = value } })
    }
}
