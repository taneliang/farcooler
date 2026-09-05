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
        .onChange(of: scanner.scanned?.payload) { _, _ in takeReply() }
    }

    // MARK: - Key B

    private var choosing: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                Text("Add This Mac").font(.title3.weight(.semibold))

                Text("Use Far Cooler with the runners you select.")
                    .font(.callout)

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
                    Button("Show Code", action: present)
                        .keyboardShortcut(.defaultAction)
                        .disabled(!ready)
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
            Text("Scan this code with another device").font(.headline)
            Text(
                "On a device you’ve already added, open Far Cooler and choose Add Device. "
                    + "Then scan this code."
            )
            .font(.callout)
            .foregroundStyle(.secondary)
            .fixedSize(horizontal: false, vertical: true)

            if let image = qrImage(offer) {
                Image(nsImage: image)
                    .resizable()
                    .interpolation(.none)
                    .scaledToFit()
                    .frame(width: 300, height: 300)
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

    private var scanningTheReply: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Scan the code on the other device").font(.headline)
            Text("This shares the runners selected for this Mac.")
                .font(.callout)
                .foregroundStyle(.secondary)

            if let problem = scanner.problem {
                Text(problem).font(.callout).foregroundStyle(.orange)
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

    private func finished(_ sentence: String) -> some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(joined?.headline ?? Joined.ready).font(.headline)
            if !sentence.isEmpty {
                Text(sentence)
                    .font(.callout)
                    .fixedSize(horizontal: false, vertical: true)
            }
            // The runners this Mac was NOT given a key for, named. Orange
            // rather than secondary because it is the half somebody has to act
            // on, and the alternative to reading it here is meeting it later as
            // a connection that fails for no stated reason.
            if let note = joined?.note {
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
                Button("Done", action: dismissed).keyboardShortcut(.defaultAction)
            }
        }
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
            store.present(
                name: thisMacName, account: account.userId, keyA: keyA.publicKey, keyB: keyB)
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
    /// that decision and its reasons live.
    private func apply(_ manifest: CeremonyManifest) {
        let joined = Joined(manifest.runners)
        self.joined = joined

        for runner in joined.reachable {
            // The alias when there is one, because the alias is the thing
            // `~/.ssh/config` names. `Editors.swift` builds `ssh://{host}{path}`
            // out of this exact string.
            runners.add(target(for: runner))
        }

        if shell.wanted, shell.addToConfig {
            writeConfig(joined.reachable)
        }

        store.finish(joined.summary ?? "")
    }

    private func target(for runner: CeremonyRunner) -> String {
        runner.alias.isEmpty ? runner.reach.name(user: runner.user) : runner.alias
    }

    private func writeConfig(_ granted: [CeremonyRunner]) {
        // Nothing to write is not the same as writing nothing. `SshConfig.write`
        // takes the WHOLE block every time and an empty array removes it, so a
        // ceremony where every runner came back pending would delete the block
        // an earlier one left behind — taking Zed's access to runners this
        // reply was never about.
        guard !granted.isEmpty, let identity = identityForConfig() else { return }
        let resolution = SshConfig.aliases(
            for: granted, avoiding: SshConfig.patternsInUse())
        var lines: [String] = []
        var chosen: [(alias: String, target: String)] = []
        for runner in granted {
            guard let alias = resolution.aliases[runner.id] else { continue }
            lines += SshConfig.block(for: runner, alias: alias, identity: identity)
            chosen.append((alias, target(for: runner)))
        }

        do {
            try SshConfig.write(lines, identity: identity)
            // Remembered only AFTER the file took the block, and this ordering has
            // teeth now that the write can actually fail. `SshConfigAliases` is
            // what `Editors.swift` substitutes into `ssh://{host}{path}`, so an
            // alias remembered for a block that was never written hands Zed
            // `ssh://box/path` for a `Host box` that does not exist — and ssh
            // fails to resolve a hostname instead of falling back to
            // `you@box.tail-1234.ts.net`, which would have worked. A failed write
            // has to leave the editor menu exactly as it was.
            for entry in chosen { SshConfigAliases.remember(entry.alias, for: entry.target) }
            configNote = resolution.message
        } catch {
            configNote = (error as? LocalizedError)?.errorDescription
        }
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

    init(_ runners: [CeremonyRunner]) {
        reachable = runners.filter { !$0.pending }
        pending = runners.filter(\.pending)
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
            return pending.isEmpty ? "The other device didn’t share any runners." : nil
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
        guard !pending.isEmpty else { return nil }
        let names = ListFormatter.localizedString(byJoining: pending.map(name(of:)))
        return pending.count == 1
            ? "\(names) doesn’t have this Mac’s key yet. To use it, add this Mac again "
                + "from a device that can reach it."
            : "\(names) don’t have this Mac’s key yet. To use them, add this Mac again "
                + "from a device that can reach them."
    }

    /// A label comes from the granting device and is what somebody ticked there,
    /// so it is the name to use. How the runner is reached is the fallback for a
    /// reply that carried none, because a sentence with a blank in it names
    /// nothing.
    private func name(of runner: CeremonyRunner) -> String {
        runner.label.isEmpty ? runner.reach.name(user: runner.user) : runner.label
    }
}
