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
                Text("Add this Mac to your runners").font(.title3.weight(.semibold))

                Text("**Far Cooler access** — run agents and terminals on the runners you pick.")
                    .font(.callout)

                VStack(alignment: .leading, spacing: 8) {
                    Toggle(isOn: $shell.wanted) {
                        Text("**Shell access** — Zed, git and Terminal on this Mac reach them too.")
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
            Text("Scan this from a device you already use").font(.headline)
            Text(
                "Open Far Cooler there, choose Add Device, and point its camera at this. "
                    + "The code is good for two minutes."
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
                Button("Scan the Reply") {
                    scanningReply = true
                    Task { await scanner.start() }
                }
                .keyboardShortcut(.defaultAction)
            }
        }
    }

    private var scanningTheReply: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Scan the code it shows back").font(.headline)
            Text("That one carries the runners you were granted.")
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
            Text("This Mac is added").font(.headline)
            Text(sentence)
                .font(.callout)
                .fixedSize(horizontal: false, vertical: true)
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
                ?? "Far Cooler couldn't prepare this Mac's keys."
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
    private func apply(_ manifest: CeremonyManifest) {
        for runner in manifest.runners {
            // The alias when there is one, because the alias is the thing
            // `~/.ssh/config` names. `Editors.swift` builds `ssh://{host}{path}`
            // out of this exact string.
            runners.add(target(for: runner))
        }

        if shell.wanted, shell.addToConfig {
            writeConfig(manifest.runners)
        }

        let count = manifest.runners.count
        store.finish(
            count == 1
                ? "It can reach \(manifest.runners[0].label)."
                : "It can reach \(count) runners.")
    }

    private func target(for runner: CeremonyRunner) -> String {
        runner.alias.isEmpty ? "\(runner.user)@\(runner.address)" : runner.alias
    }

    private func writeConfig(_ granted: [CeremonyRunner]) {
        guard let identity = identityForConfig() else { return }
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
