import AgentKit
import AppKit
import Foundation
import SwiftUI
import Testing

@testable import Far_Cooler

/// The conversation view on a runner reached over ssh, end to end (ov-408).
///
/// A scratch `sshd` runs as this user on loopback, with its own host key, its
/// own `AuthorizedKeysFile`, and `HOME` and `FARCOOLER_HOME` set under
/// `/tmp/fc-t` by `SetEnv`, so the forced command's `~/.local/bin/farcoolerd-*`
/// is this tree's daemon and the line the daemon writes is the file sshd
/// reads. A stand-in "owner" key and an `ssh` wrapper (`-F` a scratch config)
/// stand in for the owner's own ssh setup, which is never read or written.
///
/// Then the real thing: `NativeAgents` pairs the Mac's key through the real
/// CLI over ssh, the client core dials the runner pinned to the host key in
/// the scratch `known_hosts`, the projector is turned on over ssh, rows are
/// read and followed, a send and Stop come back with the runner's word, a
/// revoke on the runner ends the session and brings the terminal back with
/// no re-pair, and a runner that stops answering does the same.
///
/// Opt-in (`FARCOOLER_SSH_E2E=1`): it starts an sshd, which CI has no use for.
@MainActor
@Suite(.serialized)
struct RemoteConversationSshTests {
    nonisolated static var runnable: Bool {
        ProcessInfo.processInfo.environment["FARCOOLER_SSH_E2E"] == "1" && NativeAgentRunnerTests.runnable
            && FileManager.default.isExecutableFile(atPath: "/usr/sbin/sshd")
    }

    /// Short, under the lane's scratch directory.
    let base = "/tmp/fc-t/mac-remote-view/e"
    let target = "scratch-runner"
    var cli: String { NativeAgentRunnerTests.cliPath! }
    var daemon: String { (cli as NSString).deletingLastPathComponent + "/farcoolerd" }

    /// The runner's side: its home, its Far Cooler, a stand-in agent.
    var runnerEnvironment: [String: String] {
        var environment = ProcessInfo.processInfo.environment
        ScratchDaemon.isolate(&environment, home: base + "/h", config: base + "/config.toml")
        environment["HOME"] = base + "/home"
        environment["FARCOOLER_TEST_STUB_AGENTS"] = "1"
        environment["CLAUDE_CONFIG_DIR"] = base + "/claude"
        environment.removeValue(forKey: "FARCOOLER_PROJECTOR")
        return environment
    }

    /// This Mac's side: the CLI reaching the runner through the wrapper.
    var macEnvironment: [String: String] {
        var environment = ProcessInfo.processInfo.environment
        ScratchDaemon.isolate(&environment, home: base + "/mac", config: base + "/mac/config.toml")
        environment["HOME"] = base + "/mac"
        environment["PATH"] = base + "/bin:" + (environment["PATH"] ?? "/usr/bin:/bin")
        return environment
    }

    @discardableResult
    func run(_ binary: String, _ args: [String], _ environment: [String: String]? = nil) async -> (ok: Bool, out: Data, err: String) {
        let ran = await ProcessRunner.run(binary, args, environment: environment, deadline: 60)
        return (ran.succeeded, ran.stdout, String(decoding: ran.stderr, as: UTF8.self))
    }

    func write(_ text: String, _ path: String, mode: Int = 0o644) throws {
        try text.write(toFile: path, atomically: true, encoding: .utf8)
        try FileManager.default.setAttributes([.posixPermissions: mode], ofItemAtPath: path)
    }

    /// The sshd, its keys, the wrapper; the port it listens on.
    func stage() async throws -> Int {
        let fm = FileManager.default
        for dir in ["home/.ssh", "home/.local/bin", "sshd", "bin", "mac/.ssh", "claude", "repos/demo"] {
            try fm.createDirectory(atPath: "\(base)/\(dir)", withIntermediateDirectories: true)
        }
        for name in ["farcoolerd", "farcoolerd-local", "farcoolerd-canary", "farcoolerd-preview"] {
            try fm.createSymbolicLink(atPath: "\(base)/home/.local/bin/\(name)", withDestinationPath: daemon)
        }
        try write("", "\(base)/home/.ssh/authorized_keys", mode: 0o600)
        for key in ["host", "owner"] {
            await run("/usr/bin/ssh-keygen", ["-q", "-t", "ed25519", "-N", "", "-f", "\(base)/sshd/\(key)"])
        }
        try fm.copyItem(atPath: "\(base)/sshd/owner.pub", toPath: "\(base)/sshd/owner_keys")
        let port = Int.random(in: 20000...29999)
        let hostKey = try String(contentsOfFile: "\(base)/sshd/host.pub", encoding: .utf8)
            .split(separator: " ").prefix(2).joined(separator: " ")
        try write("[127.0.0.1]:\(port) \(hostKey)\n", "\(base)/sshd/known_hosts")
        let runner = runnerEnvironment
        let path = "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin"
        try write(
            """
            Port \(port)
            ListenAddress 127.0.0.1
            HostKey \(base)/sshd/host
            PidFile \(base)/sshd/sshd.pid
            AuthorizedKeysFile \(base)/home/.ssh/authorized_keys \(base)/sshd/owner_keys
            StrictModes no
            PasswordAuthentication no
            KbdInteractiveAuthentication no
            UsePAM no
            AllowUsers \(NSUserName())
            SetEnv HOME=\(base)/home FARCOOLER_HOME=\(runner["FARCOOLER_HOME"]!) FARCOOLER_CONFIG=\(runner["FARCOOLER_CONFIG"]!) PATH=\(path)

            """, "\(base)/sshd/sshd_config")
        try write(
            """
            Host \(target)
              HostName 127.0.0.1
              Port \(port)
              User \(NSUserName())
              IdentityFile \(base)/sshd/owner
              IdentitiesOnly yes
              UserKnownHostsFile \(base)/sshd/known_hosts
              GlobalKnownHostsFile /dev/null
              StrictHostKeyChecking yes

            """, "\(base)/sshd/ssh_config")
        try write("#!/bin/sh\nexec /usr/bin/ssh -F \(base)/sshd/ssh_config \"$@\"\n", "\(base)/bin/ssh", mode: 0o755)
        try fm.createSymbolicLink(atPath: "\(base)/bin/ssh-keygen", withDestinationPath: "/usr/bin/ssh-keygen")
        let started = await run("/usr/sbin/sshd", ["-f", "\(base)/sshd/sshd_config", "-E", "\(base)/sshd/log"])
        #expect(started.ok, "sshd: \(started.err)")
        return port
    }

    /// The sshd's pid and every session it forked.
    func stopSshd() async {
        guard let pid = (try? String(contentsOfFile: "\(base)/sshd/sshd.pid", encoding: .utf8))
            .flatMap({ Int32($0.trimmingCharacters(in: .whitespacesAndNewlines)) })
        else { return }
        let table = String(decoding: (await run("/bin/ps", ["-axo", "pid=,ppid="])).out, as: UTF8.self)
        var doomed: Set<Int32> = [pid]
        var grew = true
        while grew {
            grew = false
            for line in table.split(separator: "\n") {
                let fields = line.split(separator: " ").compactMap { Int32($0) }
                if fields.count == 2, doomed.contains(fields[1]), doomed.insert(fields[0]).inserted { grew = true }
            }
        }
        for victim in doomed { kill(victim, SIGTERM) }
    }

    /// With `FARCOOLER_CAPTURE_OUT`, the real view in each appearance, in a
    /// titled window off every screen; never input.
    func capture(_ name: String, _ view: some View) async throws {
        guard let out = ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"] else { return }
        try FileManager.default.createDirectory(atPath: out, withIntermediateDirectories: true)
        let window = NativeAgentTests.window(view)
        defer { window.close() }
        for (variant, appearance) in RealWindowCaptures.variants {
            window.appearance = NSAppearance(named: appearance)
            await NativeAgentTests.settle(window, 800)
            let image = try #require(RealWindowCaptures.windowImage(window), "screencapture -l isn't allowed here")
            try #require(image.representation(using: .png, properties: [:]))
                .write(to: URL(fileURLWithPath: out).appendingPathComponent("\(name)-\(variant).png"))
        }
    }

    @Test("Pairs, reads, sends and falls back, against a runner over ssh", .enabled(if: RemoteConversationSshTests.runnable))
    func theViewWorksOverSsh() async throws {
        try? FileManager.default.removeItem(atPath: base)
        defer { try? FileManager.default.removeItem(atPath: base) }
        _ = try await stage()
        await run(cli, ["--json", "daemon", "ensure"], runnerEnvironment)
        do {
            try await exercise()
        } catch {
            Issue.record("\(error)")
        }
        await stopSshd()
        await ScratchDaemon.stop(cli: cli, farcoolerHome: base + "/h")
    }

    private func exercise() async throws {
        let runner = runnerEnvironment
        // A claude pane on the runner, and the transcript claude would write.
        let demo = base + "/repos/demo"
        for args in [["init", "-q"], ["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty", "-m", "i"]] {
            await run("/usr/bin/git", ["-C", demo] + args)
        }
        await run(cli, ["root", "add", base + "/repos"], runner)
        await run(cli, ["repo", "register", demo], runner)
        let made = await run(cli, ["--json", "worktree", "create", "demo", "wt1", "--branch", "wt1", "--fork-only"], runner)
        let worktree = try #require((try JSONSerialization.jsonObject(with: made.out) as? [String: Any])?["short"] as? String, "\(made.err)")
        let created = await run(cli, ["--json", "terminal", "create", worktree, "--preset", "claude"], runner)
        let id = try #require((try JSONSerialization.jsonObject(with: created.out) as? [String: Any])?["id"] as? String, "\(created.err)")
        let listed = await run(cli, ["worktree", "list", "--json"], runner)
        let worktrees = (try JSONSerialization.jsonObject(with: listed.out) as? [String: Any])?["worktrees"] as? [[String: Any]] ?? []
        let mine = try #require(worktrees.first { ($0["terminals"] as? [[String: Any]])?.contains { $0["id"] as? String == id } == true })
        let session = try #require((mine["terminals"] as? [[String: Any]])?.first { $0["id"] as? String == id }?["agentSessionId"] as? String)
        let real = URL(fileURLWithPath: try #require(mine["worktree"] as? String)).resolvingSymlinksInPath().path
        let project = base + "/claude/projects/" + String(real.map { $0.isASCII && ($0.isLetter || $0.isNumber || $0 == "-") ? $0 : "-" })
        try FileManager.default.createDirectory(atPath: project, withIntermediateDirectories: true)
        let transcript = project + "/\(session).jsonl"
        try Data(
            #"{"type":"user","promptId":"p1","promptSource":"typed","uuid":"u-p1","timestamp":"2026-10-06T10:00:00Z","message":{"content":"Tidy the parser."}}"#
                .utf8 + Data("\n".utf8)
        ).write(to: URL(fileURLWithPath: transcript))
        let terminal = try NativeAgentTests.terminal(id: id)

        // The Mac: a key in a memory vault, the real CLI over the wrapper.
        let defaults = RemoteConversationTests.defaults()
        defaults.set(true, forKey: NativeAgents.settingKey)
        let pairing = RemotePairing(key: ConversationKey(vault: MemoryConversationKeyVault()), defaults: defaults)
        let mac = macEnvironment
        let cli = cli
        pairing.cli = { args in
            let ran = await ProcessRunner.run(cli, args, environment: mac, deadline: 60)
            print("SSH-E2E farcooler \(args.prefix(6).joined(separator: " ")) -> \(ran.succeeded) \(String(decoding: ran.stderr, as: UTF8.self).prefix(600))")
            return CLI.Result(
                ok: ran.succeeded, output: String(decoding: ran.stdout, as: UTF8.self), errors: String(decoding: ran.stderr, as: UTF8.self))
        }
        let ssh = base + "/bin/ssh"
        pairing.resolve = { await RemoteReach.resolve($0, ssh: ssh) }
        let agents = NativeAgents(defaults: defaults, pairing: pairing)
        agents.keepThroughFailures = 1
        agents.retryDelay = .milliseconds(50)
        defer { NativePaneModel.remember(false, for: id) }

        agents.start(target: target)
        try await NativeDefaultOnTests.until(60) { agents.offers(terminal, target: target) || pairing.states[target].map { $0 != .paired } == true }
        #expect(agents.offers(terminal, target: target), "not offered: \(String(describing: pairing.states[target]))")
        #expect(pairing.states[target] == .paired)

        // The line the runner's daemon wrote: restricted, control, no shell.
        let lines = try String(contentsOfFile: base + "/home/.ssh/authorized_keys", encoding: .utf8)
        print("SSH-E2E authorized_keys:\n\(lines)")
        let line = try #require(lines.split(separator: "\n").first { $0.contains(pairing.key.clientID!) })
        #expect(line.hasPrefix("restrict,command=\""))
        #expect(line.contains("--scope control"))
        #expect(try String(contentsOfFile: base + "/config.toml", encoding: .utf8).contains("projector = true"))

        // Rows, followed.
        let model = agents.model(for: id, target: target)
        model.setOnScreen(true, by: UUID())
        model.showsNative = true
        defer { model.store.stop() }
        try await NativeDefaultOnTests.until(30) { model.store.ids.contains("turn:p1") }
        #expect(model.store.ids.contains("turn:p1"), "no rows over ssh")
        let handle = try FileHandle(forWritingAtPath: transcript).unwrap()
        handle.seekToEndOfFile()
        handle.write(
            Data(
                #"{"type":"assistant","uuid":"a1","timestamp":"2026-10-06T10:00:03Z","message":{"content":[{"type":"text","text":"Tidied over ssh."}],"stop_reason":"end_turn"}}"#
                    .utf8 + Data("\n".utf8)))
        try handle.close()
        try await NativeDefaultOnTests.until(30) {
            model.store.ids.contains { if case .prose(let p)? = model.store.box($0)?.row.kind { return p.text == "Tidied over ssh." } else { return false } }
        }
        #expect(model.store.ids.count >= 2, "the follow brought nothing")

        try await capture("ssh-conversation", NativeAgentView(model: model, isFocused: false, showTerminal: {}))
        try await capture("ssh-devices-paired", Form { ConversationPairingSection(agents: agents) }.formStyle(.grouped))

        // A send and Stop reach the runner and come back with its word: a
        // stand-in pane takes neither.
        let sink = try #require(model.sink)
        do {
            _ = try await sink.compose(terminal: id, text: "and the docs")
            Issue.record("a stand-in pane took a message")
        } catch let failure as RunnerCore.Failure {
            print("SSH-E2E compose refused: \(failure)")
            #expect(failure.what != nil, "\(failure)")
        }
        let keys = try #require(model.keys)
        do {
            try await keys.interrupt(terminal: id)
            Issue.record("a stand-in pane took Stop")
        } catch let failure as RunnerCore.Failure {
            #expect(failure.what == "not_an_agent", "\(failure)")
        }

        // Revoked on the runner: the session ends, the terminal comes back,
        // and the Mac doesn't put its key back.
        let clientID = pairing.key.clientID!
        let revoked = await run(cli, ["client", "revoke", clientID], runner)
        #expect(revoked.ok, "\(revoked.err)")
        try await NativeDefaultOnTests.until(60) { !agents.offers(terminal, target: target) }
        #expect(!agents.offers(terminal, target: target), "a revoked Mac kept its view")
        #expect(pairing.states[target] == .removed, "\(String(describing: pairing.states[target]))")
        try await capture("ssh-devices-removed", Form { ConversationPairingSection(agents: agents) }.formStyle(.grouped))
        let after = try String(contentsOfFile: base + "/home/.ssh/authorized_keys", encoding: .utf8)
        #expect(!after.contains(clientID), "the Mac put its key back by itself")

        // Pair Again, then the runner stops answering: the terminal again.
        agents.pairAgain(target)
        try await NativeDefaultOnTests.until(60) { agents.offers(terminal, target: target) }
        #expect(agents.offers(terminal, target: target), "Pair Again didn't: \(String(describing: pairing.states[target]))")
        model.showsNative = true
        try await NativeDefaultOnTests.until(30) { model.store.phase == .live }
        await stopSshd()
        try await NativeDefaultOnTests.until(60) { !agents.offers(terminal, target: target) }
        #expect(!agents.offers(terminal, target: target), "a runner that stopped answering kept its view")
    }
}

private extension Optional {
    func unwrap() throws -> Wrapped {
        guard let self else { throw CocoaError(.fileNoSuchFile) }
        return self
    }
}
