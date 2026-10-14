import AgentKit
import SwiftUI

/// This Mac being added to somebody's runners.
///
/// Three steps, and the first is the one no other platform has: **choose Key
/// B**. The private half of that key never leaves this Mac, and neither does
/// `~/.ssh` — so the choice is made here, before the code is shown, and the
/// public half travels in the code as `key_b`. The Mac doing the granting sees
/// what was chosen and grants it; it cannot choose it.
struct JoinView: View {
    @Environment(\.dismiss) private var dismiss
    @StateObject private var store = CeremonyStore()
    @StateObject private var scanner = CodeScanner()
    @ObservedObject private var account = Account.shared
    @ObservedObject private var runners = Runners.shared

    @State private var shell = ShellKeyChoice(deviceName: thisMacName)
    @State private var existing: [ExistingKey] = []
    @State private var preparing = false
    @State private var problem: String?
    /// What happened to `~/.ssh/config`, in a sentence — a collision that was
    /// renamed, or a write that could not happen. Never a Rust error.
    @State private var configNote: String?
    /// The reply, split into what this Mac can use and what it cannot. Set once,
    /// by ``apply(_:)``, and read by the screen that follows it.
    @State private var joined: Joined?
    @State private var scanningReply = false

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            switch store.phase {
            case .showingOffer where scanningReply: scanningTheReply
            case .showingOffer(let offer): showing(offer)
            case .refused(let refusal): RefusalView(refusal: refusal, retry: retry, done: dismissed)
            case .done(let sentence): finished(sentence)
            default: choosing
            }
        }
        .padding(20)
        .frame(width: 460)
        .task { existing = ShellKey.existing() }
        .task { await MacNodeIdentity.shared.prepare() }
        .onChange(of: scanner.scanned?.payload) { _, _ in takeReply() }
    }

    // MARK: - Key B

    private var choosing: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                SheetHeader(
                    title: "Add This Mac", subtitle: "Use Far Cooler with the runners you select.")

                VStack(alignment: .leading, spacing: 8) {
                    Toggle(isOn: $shell.wanted) {
                        Text("Allow Zed, Git, and Terminal to connect to these runners")
                    }

                    Picker("", selection: $shell.source) {
                        Text("New key").tag(ShellKeyChoice.Source.new)
                        Text("An existing key in ~/.ssh").tag(ShellKeyChoice.Source.existing)
                    }
                    .pickerStyle(.radioGroup)
                    .labelsHidden()
                    .disabled(!shell.wanted)

                    if shell.source == .new {
                        TextField("Key name", text: $shell.name)
                            .disabled(!shell.wanted)
                    } else {
                        Picker("", selection: $shell.existing) {
                            Text("Choose…").tag(ExistingKey?.none)
                            ForEach(existing) { key in
                                Text(key.comment.isEmpty ? key.name : "\(key.name) — \(key.comment)")
                                    .tag(ExistingKey?.some(key))
                            }
                        }
                        .labelsHidden()
                        .disabled(!shell.wanted)
                    }

                    Toggle("Add to ~/.ssh/config", isOn: $shell.addToConfig)
                        .disabled(!shell.wanted)
                }

                // Generating is the default because it is INDEPENDENTLY
                // REVOCABLE, not because it is safer: an existing key may be
                // passphrase-protected or FIDO-backed and better protected than
                // a fresh 0600 file. What it cannot be is removed without
                // consequence, and this is where that is said.
                if let warning = shell.warning {
                    Label(warning, systemImage: "exclamationmark.triangle")
                        .font(.callout)
                        .foregroundStyle(.orange)
                        .fixedSize(horizontal: false, vertical: true)
                }

                if let problem {
                    Text(problem)
                        .font(.callout)
                        .foregroundStyle(.orange)
                        .fixedSize(horizontal: false, vertical: true)
                }

                HStack {
                    Spacer()
                    Button("Cancel", action: dismissed).keyboardShortcut(.cancelAction)
                    if preparing { ProgressView().controlSize(.small) }
                    Button("Show Code", action: present)
                        .keyboardShortcut(.defaultAction)
                        .disabled(!ready || preparing)
                }
            }
        }
        .frame(maxHeight: 460)
    }

    private var ready: Bool {
        guard account.isSignedIn else { return false }
        guard shell.wanted else { return true }
        return shell.source == .new
            ? !shell.name.trimmingCharacters(in: .whitespaces).isEmpty
            : shell.existing != nil
    }

    // MARK: - The code, and the reply

    private func showing(_ offer: String) -> some View {
        VStack(alignment: .leading, spacing: 12) {
            SheetHeader(
                title: "Scan This Code with Another Device",
                subtitle: "On a device you’ve already added, open Far Cooler and choose Add Device. "
                    + "Then scan this code.")

            if let image = qrImage(offer) {
                Image(nsImage: image)
                    .resizable()
                    .interpolation(.none)
                    .scaledToFit()
                    .frame(width: 300, height: 300)
                    .frame(maxWidth: .infinity)
            }

            // The key the other device shows on its confirmation, for the
            // person adding this Mac to compare in person (ov-525).
            if let key = Self.ownKey(in: offer) {
                Text("Key \(key)")
                    .font(.system(.callout, design: .monospaced))
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity)
            }

            HStack {
                Spacer()
                Button("Cancel", action: dismissed).keyboardShortcut(.cancelAction)
                Button("Scan Code") {
                    scanningReply = true
                    Task { await scanner.start() }
                }
                .keyboardShortcut(.defaultAction)
            }
        }
    }

    /// This Mac's key in the offer it's showing, in the form the other
    /// device's confirmation shows (`KeyFingerprint`).
    static func ownKey(in offer: String) -> String? {
        guard let decoded = try? JSONDecoder().decode(CeremonyOffer.self, from: Data(offer.utf8)),
            let fingerprint = decoded.fingerprint
        else { return nil }
        return KeyFingerprint.short(fingerprint)
    }

    private var scanningTheReply: some View {
        VStack(alignment: .leading, spacing: 12) {
            SheetHeader(
                title: "Scan the Code on the Other Device",
                subtitle: "This shares the runners selected for this Mac.")

            if let problem = scanner.problem {
                Text(problem).font(.callout).foregroundStyle(.red)
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

    private func finished(_ sentence: String) -> some View {
        JoinFinishedView(
            headline: joined?.headline ?? Joined.ready, sentence: sentence, note: joined?.note,
            configNote: configNote, done: dismissed)
    }

    // MARK: - Wiring

    /// Make the keys, then ask Rust for the code.
    ///
    /// Key A first and always; Key B only when shell access was chosen. Both
    /// are generated by `farcooler_client_generate_key` — this project
    /// implements no cryptography of its own, in Swift least of all.
    private func present() {
        problem = nil
        do {
            let keyA = try DeviceKey.privateKey(for: account.userId)
            var keyB: String?
            if shell.wanted {
                switch shell.source {
                case .new:
                    keyB = try ShellKey.generate(named: shell.name).publicKey
                case .existing:
                    keyB = shell.existing?.publicKey
                }
            }
            let name = thisMacName
            let userId = account.userId
            // The tunnel key is asked for off the button's call, because making
            // it can run the bundled CLI. Nil is the address-only offer: a Mac
            // that won't write `~/.ssh/config`, or that has no key to give.
            // One at a time: a second click would start a second ceremony.
            guard !preparing else { return }
            preparing = true
            Task {
                defer { preparing = false }
                let nodeKey = await MacNodeIdentity.shared.offer(for: shell)
                store.present(
                    name: name, account: userId, keyA: keyA.publicKey, keyB: keyB,
                    nodeKey: nodeKey)
            }
        } catch {
            problem = (error as? LocalizedError)?.errorDescription
                ?? "Couldn’t prepare keys for this Mac."
        }
    }

    private func takeReply() {
        guard let scanned = scanner.scanned else { return }
        guard let manifest = store.accept(scanned.payload, at: scanned.at) else { return }
        apply(manifest)
    }

    /// What a manifest means on this Mac: runners it can now reach, and — if
    /// shell access was chosen — a block in `~/.ssh/config` so Zed, git and
    /// Terminal reach them too.
    ///
    /// **Runners it can now reach**, which is not every runner in the reply.
    /// A pending one is added to nothing here — see ``Joined``, which is where
    /// that decision and its reasons live — and a tunneled one is added only
    /// once it can be dialed, which ``JoinApply`` decides.
    private func apply(_ manifest: CeremonyManifest) {
        let identity = shell.wanted && shell.addToConfig ? identityForConfig() : nil
        Task {
            // The alias when there is one, because the alias is the thing
            // `~/.ssh/config` names. `Editors.swift` builds `ssh://{host}{path}`
            // out of this exact string.
            let applied = await JoinApply(keeper: CLINodeKeyTool()).settle(
                manifest, configIdentity: identity, target: target(for:), runners: runners,
                aliases: DefaultsAliasStore())
            joined = applied.joined
            configNote = applied.configNote
            store.finish(applied.joined.summary ?? "")
        }
    }

    nonisolated private func target(for runner: CeremonyRunner) -> String {
        runner.alias.isEmpty ? runner.reach.name(user: runner.user) : runner.alias
    }

    private func identityForConfig() -> URL? {
        switch shell.source {
        case .new: return ShellKey.directory.appendingPathComponent(shell.name)
        case .existing: return shell.existing?.path
        }
    }

    private func retry() {
        store.scanAgain()
        scanner.rescan()
    }

    private func dismissed() {
        scanner.stop()
        dismiss()
    }
}

/// The last screen of joining: what this Mac can use, and what it can't.
struct JoinFinishedView: View {
    let headline: String
    let sentence: String
    let note: String?
    let configNote: String?
    let done: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            SheetHeader(title: headline)
            if !sentence.isEmpty {
                Text(sentence)
                    .font(.callout)
                    .fixedSize(horizontal: false, vertical: true)
            }
            // The runners this Mac was NOT given a key for, named. Orange
            // rather than secondary because it is the half somebody has to act
            // on, and the alternative to reading it here is meeting it later as
            // a connection that fails for no stated reason.
            if let note {
                Text(note)
                    .font(.callout)
                    .foregroundStyle(.orange)
                    .fixedSize(horizontal: false, vertical: true)
            }
            if let configNote {
                Text(configNote)
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            RemoteLoginView()
            HStack {
                Spacer()
                Button("Done", action: done).keyboardShortcut(.defaultAction)
            }
        }
    }
}

/// A reply, as the Mac RECEIVING it has to read it.
///
/// `CeremonyRunner.pending` means "this runner does NOT have this device's
/// key". The granting side decides it from what its writes actually did — see
/// ``Enrollment/Outcome/granting(_:)`` — so it arrives here as a fact about a
/// file on another machine, and a failed write, an unreachable runner and one
/// never attempted all arrive the same way, because the file is in the same
/// state in all three.
///
/// **Nothing here fixes one, and nothing anywhere else does either.** There is
/// no retry queue; a phone may never enroll a device's key at all. A runner a
/// granting device could not write to traveling pending is the intended end
/// state, so the only move this Mac has is to be honest about it — which is
/// what this split is.
///
/// ## What a pending runner does NOT get
///
/// It is not added to ``Runners``, it gets no `~/.ssh/config` block, and it is
/// not counted. All three used to happen to every runner in the reply, and all
/// three are claims of access: a row that can only fail to connect, a `Host`
/// alias whose `IdentityFile` that runner will refuse — the granting side skips
/// Key B entirely whenever Key A did not land, so neither key is there — and a
/// number that is simply wrong.
///
/// Leaving it out loses nothing that lasts. The remedy the note names is this
/// same ceremony, run again from a device that CAN reach that runner, and the
/// runner then arrives with `pending` false: added, with its block, at the
/// moment it works rather than before it does. A row marked "might not work"
/// would be worse than no row, because nothing in this app could ever clear the
/// mark.
///
/// iOS reads a reply the same way, in its own `Joined`. The two differ only in
/// what the screen does with the answer — the Mac counts runners in a sentence,
/// the phone lists them.
struct Joined {
    /// The runners whose `authorized_keys` has this Mac's key.
    let reachable: [CeremonyRunner]
    /// The runners that do not.
    let pending: [CeremonyRunner]
    /// Tunneled runners that took the key and were still left out of
    /// ``reachable``, and why. See ``JoinApply``.
    private(set) var lost: [(runner: CeremonyRunner, because: TunnelTrouble)] = []

    init(_ runners: [CeremonyRunner]) {
        reachable = runners.filter { !$0.pending }
        pending = runners.filter(\.pending)
    }

    private init(
        reachable: [CeremonyRunner], pending: [CeremonyRunner],
        lost: [(runner: CeremonyRunner, because: TunnelTrouble)]
    ) {
        self.reachable = reachable
        self.pending = pending
        self.lost = lost
    }

    /// Without `runners`, which this Mac cannot use after all.
    func losing(_ runners: [CeremonyRunner], because trouble: TunnelTrouble) -> Joined {
        let ids = Set(runners.map(\.id))
        return Joined(
            reachable: reachable.filter { !ids.contains($0.id) }, pending: pending,
            lost: lost + runners.map { ($0, trouble) })
    }

    static let ready = "This Mac is ready"

    /// Not ``ready`` when nothing landed, which a phone granting its one live
    /// connection can produce with a single failed write.
    var headline: String {
        reachable.isEmpty ? "No runners were added" : Self.ready
    }

    /// The line under it, or nil when ``note`` is the whole message — a reply
    /// where every runner is pending would otherwise be answered twice, once as
    /// a count of nothing.
    var summary: String? {
        guard let first = reachable.first else {
            return pending.isEmpty && lost.isEmpty ? "The other device didn’t share any runners." : nil
        }
        return reachable.count == 1
            ? "This Mac can access \(name(of: first))."
            : "This Mac can access \(reachable.count) runners."
    }

    /// What to say about the runners that did not take the key.
    ///
    /// It NAMES them, which the granting side deliberately does not: over there
    /// the sentence sits beside the list the person just ticked, and here that
    /// list is exactly what is missing — "which ones" is the question. What it
    /// does not do is guess WHY, for the reason
    /// `CeremonyStore.note(about:outcomes:)` on iOS gives: a runner asleep, a
    /// daemon never installed and a fence that could not be rewritten look
    /// identical from here, and a screen that guesses is how somebody ends up
    /// loosening an sshd setting that was never the problem.
    ///
    /// It promises nothing later, either. Nothing retries, so "yet" is as far as
    /// this goes, and the sentence after it is an instruction rather than a wait.
    var note: String? {
        let sentences = [pendingNote] + lostNotes
        let said = sentences.compactMap { $0 }
        return said.isEmpty ? nil : said.joined(separator: " ")
    }

    private var pendingNote: String? {
        guard !pending.isEmpty else { return nil }
        let names = ListFormatter.localizedString(byJoining: pending.map(name(of:)))
        return Self.capitalized(pending.count == 1
            ? "\(names) doesn’t have this Mac’s key yet. To use it, add this Mac again "
                + "from a device that can reach it."
            : "\(names) don’t have this Mac’s key yet. To use them, add this Mac again "
                + "from a device that can reach them.")
    }

    /// A runner with no label is named “a tunneled runner”, which cannot start a
    /// sentence in lowercase.
    static func capitalized(_ sentence: String) -> String {
        sentence.hasPrefix("a tunneled runner")
            ? sentence.prefix(1).uppercased() + sentence.dropFirst() : sentence
    }

    /// One sentence per reason, so two runners that failed the same way read as
    /// one problem.
    private var lostNotes: [String] {
        var order: [TunnelTrouble] = []
        for entry in lost where !order.contains(entry.because) { order.append(entry.because) }
        return order.map { trouble in
            let names = lost.filter { $0.because == trouble }.map { name(of: $0.runner) }
            return Self.capitalized(trouble.sentence(
                naming: ListFormatter.localizedString(byJoining: names), plural: names.count > 1))
        }
    }

    /// A label comes from the granting device and is what somebody ticked there,
    /// so it is the name to use. How the runner is reached is the fallback for a
    /// reply that carried none, because a sentence with a blank in it names
    /// nothing.
    private func name(of runner: CeremonyRunner) -> String {
        runner.label.isEmpty ? runner.reach.name(user: runner.user) : runner.label
    }
}
