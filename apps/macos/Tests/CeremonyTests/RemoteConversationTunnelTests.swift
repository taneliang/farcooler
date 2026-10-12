import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// The conversation view over the Tailcat tunnel (ov-496): `RemoteReach`
/// follows the one `ProxyCommand` the app writes for a tunneled runner
/// (`<cli> runner pipe <id>`), and refuses every other.
///
/// The config here is the app's own: `SshConfig.entry` renders it, and the
/// real `/usr/bin/ssh -G` reads it. No spelling is written by hand except the
/// ones that must be refused.
@Suite(.serialized)
struct RemoteConversationTunnelTests {
    @MainActor
    @Test("A tunneled runner showing another key than its grant's says so in the tunnel's terms, and nothing is paired")
    func aMismatchOverThePipeNamesTheGrant() async {
        let runner = RemoteConversationTests.Runner()
        runner.shown = "SHA256:someone-else"
        let pairing = RemoteConversationTests.pairing(runner, defaults: RemoteConversationTests.defaults())
        pairing.resolve = { _ in
            .success(
                RemoteReach(
                    host: "fc-cosmo", port: 22, user: "me", knownKeys: [RemoteConversationTests.known],
                    pipe: ["/x/farcooler", "runner", "pipe", "fc-1"]))
        }
        guard case .unavailable = await pairing.connect(target: "fc-cosmo") else {
            Issue.record("connected to a key that wasn't the grant's")
            return
        }
        guard case .unavailable(let sentence)? = pairing.states["fc-cosmo"] else {
            Issue.record("no sentence")
            return
        }
        #expect(sentence.contains("added with") && !sentence.contains("ssh"), Comment(rawValue: sentence))
        #expect(runner.calls.isEmpty)
    }

    static let pin = "SHA256:" + String(repeating: "A", count: 43)

    /// A scratch directory under `/tmp/fc-t`, short for the socket paths.
    static func scratch() throws -> URL {
        let url = URL(fileURLWithPath: "/tmp/fc-t/rct-\(UUID().uuidString.prefix(8))")
        try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        return url
    }

    /// A stand-in for the CLI, executable at `path`: `body` is its script.
    static func cli(at path: URL, _ body: String) throws {
        try FileManager.default.createDirectory(at: path.deletingLastPathComponent(), withIntermediateDirectories: true)
        try ("#!/bin/sh\n" + body + "\n").write(to: path, atomically: true, encoding: .utf8)
        try FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: path.path)
    }

    /// The app's block for a tunneled runner, the stable link it names, and an
    /// `ssh` that reads only that config.
    struct Staged {
        let directory: URL
        let link: URL
        let ssh: String
        let alias = "fc-cosmo"
        let id = "fc-0123456789abcdef"
        /// `/tmp` is `/private/tmp`: what `realpath` says the link is.
        var real: String {
            // Foundation maps /private/tmp back to /tmp; `realpath` doesn't.
            guard let resolved = realpath(link.path, nil) else { return link.path }
            defer { free(resolved) }
            return String(cString: resolved)
        }
    }

    static func stage(cli body: String) throws -> Staged {
        let directory = try scratch()
        // A space in the path, as the app bundle's own has.
        let link = directory.appendingPathComponent("Far Cooler/farcooler")
        try cli(at: link, body)
        let staged = Staged(directory: directory, link: link, ssh: directory.appendingPathComponent("ssh").path)
        // The block is the golden fixture the Rust e2e feeds a real ssh too
        // (`TunnelJoinTests.theE2EsBlockIsTheOneTheAppWrites` pins it to what
        // the app writes), with only the markers filled in.
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<5 { root.deleteLastPathComponent() }
        let golden = try String(contentsOf: root.appendingPathComponent("crates/daemon/tests/fixtures/app-ssh-block.txt"), encoding: .utf8)
        let block =
            golden
            .replacingOccurrences(of: "@PROGRAM@", with: SshConfig.proxyProgram(link.path))
            .replacingOccurrences(of: "@ALIAS@", with: staged.alias)
            .replacingOccurrences(of: "@ID@", with: staged.id)
            .replacingOccurrences(of: "@USER@", with: "you")
            .replacingOccurrences(of: "/@IDENTITY@", with: directory.appendingPathComponent("id").path)
        let config = directory.appendingPathComponent("config")
        try block.write(to: config, atomically: true, encoding: .utf8)
        try cli(at: URL(fileURLWithPath: staged.ssh), "exec /usr/bin/ssh -F '\(config.path)' \"$@\"")
        return staged
    }

    @Test("The block the app writes is followed, through the real ssh -G")
    func theAppsBlockIsFollowed() async throws {
        let staged = try Self.stage(cli: "echo '\(Self.pin)'")
        defer { try? FileManager.default.removeItem(at: staged.directory) }
        let resolved = await RemoteReach.resolve(staged.alias, ssh: staged.ssh, cli: staged.link.path, bundled: staged.link.path)
        guard case .success(let reach) = resolved else {
            Issue.record("refused: \(resolved)")
            return
        }
        // The resolved path runs, not the link as the config spells it.
        #expect(reach.pipe == [staged.real, "runner", "pipe", staged.id])
        // The key it must show is the grant's pin; known_hosts is /dev/null in the block.
        #expect(reach.knownKeys == [Self.pin])
        #expect(reach.revokedKeys.isEmpty)
    }

    @Test("A tunneled runner with no pin on file is not dialed")
    func noPinIsAnUnknownHost() async throws {
        let staged = try Self.stage(cli: "exit 1")
        defer { try? FileManager.default.removeItem(at: staged.directory) }
        let resolved = await RemoteReach.resolve(staged.alias, ssh: staged.ssh, cli: staged.link.path, bundled: staged.link.path)
        guard case .failure(.notGranted) = resolved else {
            Issue.record("expected notGranted, got \(resolved)")
            return
        }
    }

    @Test("Any other ProxyCommand is refused, whatever it is shaped like")
    func otherProxyCommandsAreRefused() {
        let cli = "/Users/you/.config/farcooler/canary/bin/farcooler"
        for line in [
            "nc %h %p",
            "none-of-ours runner pipe fc-1",
            "/bin/sh runner pipe fc-1",
            "\(cli) runner store-runners fc-1",
            "\(cli) runner pipe",
            "\(cli) runner pipe fc-1 extra",
            "\(cli) runner pipe -oProxyCommand=x",
            "\(cli) runner pipe .hidden",
            "\(cli) runner pipe fc-1;id",
            "\(cli) runner pipe $(id)",
            "\(cli) runner pipe fc-1 | tee",
            "\(cli)2 runner pipe fc-1",
            "'\(cli)' runner pipe fc-1 'unclosed",
            "exec \(cli) runner pipe fc-1",
            "\(cli) runner pipe %h",
            "\"\(cli)\" runner pipe fc-1",
            "\(cli)\\ x runner pipe fc-1",
            "~/.config/farcooler/canary/bin/farcooler runner pipe fc-1",
            "/Users/you/.config/farcooler/canary/x/../bin/farcooler runner pipe fc-1",
            "\(cli) runner pipe f\u{0441}-1",
            "\(cli) RUNNER pipe fc-1",
            "\(cli)\trunner pipe fc-1",
            "\(cli) runner pipe ''",
        ] {
            let settings = RemoteReach.settings("hostname box\nport 22\nproxycommand \(line)\n")
            #expect(throws: RemoteReach.Refusal.proxied, Comment(rawValue: line)) {
                try RemoteReach.destination(settings, cli: cli).get()
            }
        }
        // And ProxyJump stays refused beside a good ProxyCommand.
        let both = RemoteReach.settings("hostname box\nproxyjump bastion\nproxycommand \(cli) runner pipe fc-1\n")
        #expect(throws: RemoteReach.Refusal.proxied) { try RemoteReach.destination(both, cli: cli).get() }
        // The exact line is followed.
        let good = RemoteReach.settings("hostname box\nproxycommand \(cli) runner pipe fc-1\n")
        #expect((try? RemoteReach.destination(good, cli: cli).get())?.pipe == [cli, "runner", "pipe", "fc-1"])
    }

    @Test("A link that isn't this app's own CLI, or an app with none, runs nothing")
    func aLinkToSomethingElseRunsNothing() async throws {
        let staged = try Self.stage(cli: "echo ran > '\(Self.marker)'; echo '\(Self.pin)'")
        defer { try? FileManager.default.removeItem(at: staged.directory) }
        let other = staged.directory.appendingPathComponent("other")
        try Self.cli(at: other, "echo ran > '\(Self.marker)'; echo '\(Self.pin)'")
        try? FileManager.default.removeItem(atPath: Self.marker)
        for bundled in [other.path, nil] {
            let resolved = await RemoteReach.resolve(staged.alias, ssh: staged.ssh, cli: staged.link.path, bundled: bundled)
            guard case .failure(.tunnelUnavailable) = resolved else {
                Issue.record("expected tunnelUnavailable, got \(resolved)")
                continue
            }
        }
        #expect(!FileManager.default.fileExists(atPath: Self.marker), "ran a CLI that wasn't the app's own")
    }

    static let marker = "/tmp/fc-t/rct-ran-marker"

    @Test("A stable link that is gone is a refusal, not a dial")
    func aMissingLinkIsRefused() async throws {
        let staged = try Self.stage(cli: "echo '\(Self.pin)'")
        defer { try? FileManager.default.removeItem(at: staged.directory) }
        try FileManager.default.removeItem(at: staged.link)
        let resolved = await RemoteReach.resolve(staged.alias, ssh: staged.ssh, cli: staged.link.path, bundled: staged.link.path)
        guard case .failure(.tunnelUnavailable) = resolved else {
            Issue.record("expected tunnelUnavailable, got \(resolved)")
            return
        }
    }

    @Test("The core is handed the pipe: connecting spawns the CLI and never dials the alias")
    func theCoreSpawnsThePipe() async throws {
        let directory = try Self.scratch()
        defer { try? FileManager.default.removeItem(at: directory) }
        let marker = directory.appendingPathComponent("ran")
        let link = directory.appendingPathComponent("farcooler")
        try Self.cli(at: link, "echo \"$@\" > '\(marker.path)'")
        let reach = RemoteReach(
            host: "fc-cosmo", port: 22, user: "you", knownKeys: [Self.pin],
            pipe: [link.path, "runner", "pipe", "fc-1"])
        let core = RunnerCore()
        // A real key: a bad one is refused before any dial.
        let key = await ProcessRunner.run(
            "/usr/bin/ssh-keygen", ["-q", "-t", "ed25519", "-N", "", "-f", directory.appendingPathComponent("k").path])
        #expect(key.succeeded)
        let pem = try String(contentsOf: directory.appendingPathComponent("k"), encoding: .utf8)
        do {
            _ = try await core.connect(reach: reach, privateKey: pem, fingerprint: Self.pin)
            Issue.record("a pipe that carries nothing connected")
        } catch {}
        let ran = try? String(contentsOf: marker, encoding: .utf8)
        #expect(ran?.trimmingCharacters(in: .whitespacesAndNewlines) == "runner pipe fc-1", "the CLI was not spawned: \(String(describing: ran))")
    }
}
