import AppKit
import SwiftUI
import Testing
import Vision

@testable import Far_Cooler

/// Joining over the tunnel, from the sheet to the first byte `ssh` runs.
///
/// **The seam these two tests cross.** Swift writes `runners.json` through the
/// CLI and `~/.ssh/config` through the fence writer; `ssh` then runs the
/// `ProxyCommand` the second wrote, and the CLI that line names reads the first.
/// Each half has a test of its own. What nothing else catches is the two
/// disagreeing — an id spelled differently on either side, a path the shell
/// splits at a space — and the symptom is `ssh` saying "no runner with id" about
/// a runner the screen just called ready.
///
/// The scratch store has no node key on purpose. The pipe checks for one before
/// it dials, so the answer is "no tunnel key yet" whether or not the CLI under
/// test has the tunnel linked, and nothing here waits on a network.
struct TunnelJoinTests {
    private static var cli: String? { RealCLIPagesTests.cli }

    private static func home() -> String { "/tmp/fc-t/j-\(UUID().uuidString.prefix(6))" }

    private func manifest(_ id: String) -> CeremonyManifest {
        CeremonyManifest(
            v: 2, ceremony: "c", account: "a", channel: "release", target: "t",
            runners: [
                CeremonyRunner(
                    id: id, label: "cosmo", alias: "", user: "you", host_key: "SHA256:x",
                    reach: .tailcat(token: "tc-not-a-real-token"), pending: false)
            ])
    }

    /// Join with the real CLI as the keeper, then ask real `ssh` to use the
    /// block that came out. Returns what ssh said.
    private func joinAndConnect(
        _ id: String, store: Bool, home: String, folder: URL
    ) async throws -> (applied: JoinApply.Applied, ssh: ProcessRunner.Result) {
        let cli = try #require(Self.cli, "no farcooler CLI in target/: build it first")
        try FileManager.default.createDirectory(
            atPath: home, withIntermediateDirectories: true)

        // The bundle's path has a space in it, as the real one does.
        let bundled = folder.appendingPathComponent("Far Cooler.app").appendingPathComponent("farcooler")
        try FileManager.default.createDirectory(
            at: bundled.deletingLastPathComponent(), withIntermediateDirectories: true)
        try FileManager.default.createSymbolicLink(atPath: bundled.path, withDestinationPath: cli)

        var tool = CLINodeKeyTool()
        tool.binary = { cli }
        tool.run = { binary, arguments, stdin in
            var environment = ProcessInfo.processInfo.environment
            ScratchDaemon.isolate(&environment, home: home)
            return await ProcessRunner.run(
                binary, arguments, environment: environment, stdin: stdin, deadline: 30)
        }
        let config = folder.appendingPathComponent("config")
        var join = JoinApply(keeper: store ? tool : Skipping())
        join.cli = bundled.path
        join.patternsInUse = { [] }
        join.writeConfig = { try SshConfig.write($0, identity: $1, into: config) }
        let identity = ShellKey.directory.appendingPathComponent("farcooler-macbook-air")
        let applied = await join.run(manifest(id), configIdentity: identity) {
            $0.alias.isEmpty ? $0.reach.name(user: $0.user) : $0.alias
        }
        let alias = try #require(applied.targets.first, "nothing was added")

        var environment = ProcessInfo.processInfo.environment
        ScratchDaemon.isolate(&environment, home: home)
        let ssh = await ProcessRunner.run(
            "/usr/bin/ssh",
            ["-F", config.path, "-o", "BatchMode=yes", "-o", "ConnectTimeout=10", alias, "true"],
            environment: environment, deadline: 60)
        return (applied, ssh)
    }

    /// Says yes and stores nothing: the failure this test exists to catch.
    private struct Skipping: TunnelRunnerKeeper {
        func keep(_ runners: [CeremonyRunner]) async -> Bool { true }
    }

    private func folder() throws -> URL {
        let url = URL(fileURLWithPath: "/tmp/fc-t/f-\(UUID().uuidString.prefix(6))")
        try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        return url
    }

    /// The runner the join stored is the runner the `ProxyCommand` finds.
    @Test(.enabled(if: RealCLIPagesTests.cli != nil || RealCLIPagesTests.onCI))
    func theBlockTheJoinWritesIsOneRealSshRunsAndTheCLIFindsTheRunnerItStored() async throws {
        let home = Self.home()
        defer { try? FileManager.default.removeItem(atPath: home) }
        let (applied, ssh) = try await joinAndConnect(
            "you@cosmo; touch /tmp/fc-t/pwned", store: true, home: home, folder: try folder())

        let said = String(decoding: ssh.stderr, as: UTF8.self)
        #expect(applied.targets == ["cosmo"])
        #expect(said.contains("no tunnel key"), "ssh did not reach the pipe's own answer: \(said)")
        #expect(!said.contains("no runner with id"), "the pipe did not find the stored runner: \(said)")
        #expect(
            !said.contains("not found") && !said.contains("No such file"),
            "the ProxyCommand's path was split at the space: \(said)")
        #expect(!FileManager.default.fileExists(atPath: "/tmp/fc-t/pwned"))
        #expect(!said.contains("tc-not-a-real-token"))
    }

    /// The same run with the store step skipped must say the opposite, or the
    /// test above cannot tell a stored runner from a missing one.
    @Test(.enabled(if: RealCLIPagesTests.cli != nil || RealCLIPagesTests.onCI))
    func aRunnerThatWasNeverStoredIsNamedAsUnknownByTheSamePath() async throws {
        let home = Self.home()
        defer { try? FileManager.default.removeItem(atPath: home) }
        let (_, ssh) = try await joinAndConnect(
            "you@cosmo", store: false, home: home, folder: try folder())

        let said = String(decoding: ssh.stderr, as: UTF8.self)
        #expect(said.contains("no runner with id"), "expected the pipe to not know it: \(said)")
    }

    // MARK: - The screen

    /// The text a person can read in `view`, as Vision reads it off the pixels
    /// the view draws. Reading what is drawn rather than the accessibility
    /// tree is the point: an unattached window has no tree, and a sentence that
    /// is in the model but not on the screen is exactly what this should catch.
    @MainActor
    private func readText(of view: some View) async throws -> String {
        let host = NSHostingView(rootView: view.frame(width: 460).background(Color.white))
        host.appearance = NSAppearance(named: .aqua)
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 460, height: 500), styleMask: [.borderless],
            backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = host
        for _ in 0..<10 {
            host.layoutSubtreeIfNeeded()
            try? await Task.sleep(for: .milliseconds(20))
        }
        host.setFrameSize(host.fittingSize)
        host.layoutSubtreeIfNeeded()
        defer { window.close() }
        let rep = try #require(host.bitmapImageRepForCachingDisplay(in: host.bounds))
        host.cacheDisplay(in: host.bounds, to: rep)
        let cg = try #require(rep.cgImage)
        var lines: [String] = []
        let request = VNRecognizeTextRequest { request, _ in
            lines = (request.results as? [VNRecognizedTextObservation] ?? [])
                .compactMap { $0.topCandidates(1).first?.string }
        }
        request.recognitionLevel = .accurate
        request.usesLanguageCorrection = false
        try VNImageRequestHandler(cgImage: cg).perform([request])
        return lines.joined(separator: " ")
    }

    /// The last screen says what was left out and what to do, in the orange
    /// note, beside the headline that says nothing was added.
    @Test @MainActor func theFinishedSheetNamesATunnelThatCouldNotBeSetUp() async throws {
        let lost = CeremonyRunner(
            id: "fc-1", label: "cosmo", alias: "", user: "you", host_key: "",
            reach: .tailcat(token: "tc"), pending: false)
        let joined = Joined([lost]).losing([lost], because: .couldNotKeep)

        let shown = try await readText(
            of: JoinFinishedView(
                headline: joined.headline, sentence: joined.summary ?? "", note: joined.note,
                configNote: nil, done: {}))

        #expect(shown.contains("No runners were added"), "\(shown)")
        #expect(
            shown.contains("save the tunnel to cosmo"),
            "the sheet did not say what was left out: \(shown)")
        #expect(shown.contains("Done"), "\(shown)")
    }

    /// The sheet's own state is private to it, so what it calls cannot be driven
    /// from here. These are the two calls that make the rest reachable: without
    /// the first a tunneled reply is added by the granter's name and never kept;
    /// without the second an offer carries a key to a Mac that will not write
    /// the block its tunneled runners are reached through.
    @Test func theSheetJoinsThroughJoinApplyAndOffersThroughTheShellChoice() throws {
        var url = URL(fileURLWithPath: #filePath)
        for _ in 0..<3 { url.deleteLastPathComponent() }
        let source = try String(
            contentsOf: url.appendingPathComponent("Sources/FarCooler/Ceremony/JoinView.swift"),
            encoding: .utf8)
        #expect(source.contains("JoinApply(keeper: CLINodeKeyTool()).run("))
        #expect(source.contains("offering: MacNodeIdentity.offers(shell: shell)"))
    }
}
