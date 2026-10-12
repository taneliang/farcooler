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
    }

    static func stage(cli body: String) throws -> Staged {
        let directory = try scratch()
        // A space in the path, as the app bundle's own has.
        let link = directory.appendingPathComponent("Far Cooler/farcooler")
        try cli(at: link, body)
        let staged = Staged(directory: directory, link: link, ssh: directory.appendingPathComponent("ssh").path)
        let runner = CeremonyRunner(
            id: staged.id, label: "cosmo", alias: "", user: "you", host_key: JoinApplyTests.pin(1),
            reach: .tailcat(token: "t"), pending: false)
        let entry = SshConfig.entry(
            for: runner, alias: staged.alias, identity: directory.appendingPathComponent("id"),
            program: SshConfig.proxyProgram(link.path))
        let config = directory.appendingPathComponent("config")
        try (SshConfig.render([entry]).joined(separator: "\n") + "\n").write(to: config, atomically: true, encoding: .utf8)
        try cli(at: URL(fileURLWithPath: staged.ssh), "exec /usr/bin/ssh -F '\(config.path)' \"$@\"")
        return staged
    }

    @Test("The block the app writes is followed, through the real ssh -G")
    func theAppsBlockIsFollowed() async throws {
        let staged = try Self.stage(cli: "echo '\(Self.pin)'")
        defer { try? FileManager.default.removeItem(at: staged.directory) }
        let resolved = await RemoteReach.resolve(staged.alias, ssh: staged.ssh, cli: staged.link.path)
        guard case .success(let reach) = resolved else {
            Issue.record("refused: \(resolved)")
            return
        }
        #expect(reach.pipe == [staged.link.path, "runner", "pipe", staged.id])
        // The key it must show is the grant's pin; known_hosts is /dev/null in the block.
        #expect(reach.knownKeys == [Self.pin])
        #expect(reach.revokedKeys.isEmpty)
    }

    @Test("A tunneled runner with no pin on file is not dialed")
    func noPinIsAnUnknownHost() async throws {
        let staged = try Self.stage(cli: "exit 1")
        defer { try? FileManager.default.removeItem(at: staged.directory) }
        let resolved = await RemoteReach.resolve(staged.alias, ssh: staged.ssh, cli: staged.link.path)
        guard case .failure(.unknownHost) = resolved else {
            Issue.record("expected unknownHost, got \(resolved)")
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

    @Test("A stable link that is gone is a refusal, not a dial")
    func aMissingLinkIsRefused() async throws {
        let staged = try Self.stage(cli: "echo '\(Self.pin)'")
        defer { try? FileManager.default.removeItem(at: staged.directory) }
        try FileManager.default.removeItem(at: staged.link)
        let resolved = await RemoteReach.resolve(staged.alias, ssh: staged.ssh, cli: staged.link.path)
        guard case .failure(.proxied) = resolved else {
            Issue.record("expected proxied, got \(resolved)")
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
