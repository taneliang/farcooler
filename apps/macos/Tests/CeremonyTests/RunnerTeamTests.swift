import AgentKit
import AppKit
import SwiftUI
import TestWindows
import Testing

@testable import Far_Cooler

/// A runner joins a team from the Mac (ov-528): what the menu offers, what a
/// real pairing puts on the process's command line and what it pipes, and the
/// sheet drawn at its widths.
@MainActor
@Suite(.serialized)
struct RunnerTeamTests {
    static let token = "tok-0d8c41aa-never-in-argv"
    static let personal = DirectoryTeam(id: "team-personal", name: "Personal", isPersonal: true, role: "admin")
    static let guild = DirectoryTeam(
        id: "team-guild", name: "The Platform Infrastructure and Developer Experience Guild of Northern Europe",
        isPersonal: false, role: "member")

    // MARK: - The menu

    @Test func aRunnerThatNotifiesIsOfferedAddToTeamAndOneThatDoesNotIsOfferedPairing() {
        #expect(RunnerTeamRequest.Purpose(paired: true).menuTitle == "Add to Team…")
        #expect(RunnerTeamRequest.Purpose(paired: false).menuTitle == "Notify Me from This Runner…")
        // Not asked yet is not paired: the safe offer is the one that works for both.
        #expect(RunnerTeamRequest.Purpose(paired: nil).menuTitle == "Notify Me from This Runner…")
    }

    @Test func thePersonalTeamIsChosenUntilSomeoneChoosesAnother() {
        #expect(RunnerTeamSheet.defaultSelection(in: [Self.guild, Self.personal]) == "team-personal")
        #expect(RunnerTeamSheet.defaultSelection(in: [Self.guild]) == "team-guild")
        #expect(RunnerTeamSheet.defaultSelection(in: []) == nil)
    }

    // MARK: - A real pairing, through a stand-in `farcooler`

    /// A directory scratch: a stand-in CLI that records what it was run with and
    /// what it was fed, and a relay that is a script.
    @MainActor final class Scratch {
        let dir: URL
        let runners: Runners
        let key = "ov528-test-\(UUID().uuidString)"
        var claimAnswer: Result<DirectoryReply, AccountError> = .success(
            DirectoryReply(status: 200, body: ["runner": "r", "team": "team-guild", "token": RunnerTeamTests.token]))
        var registerAnswer: Result<DirectoryReply, AccountError> = .success(
            DirectoryReply(status: 200, body: ["runner": "r", "team": "team-personal", "token": RunnerTeamTests.token]))

        init(status: String = "paired · relay https://relay.test") throws {
            dir = FileManager.default.temporaryDirectory.appendingPathComponent("ov528-\(UUID().uuidString)")
            try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
            let proof = #"{"install":"i1","runnerKey":"ssh-ed25519 AAAA","relay":"https://relay.test","issuedAt":1790000000000,"proof":"SIG"}"#
            try proof.write(to: dir.appendingPathComponent("proof.json"), atomically: true, encoding: .utf8)
            let script = """
                #!/bin/sh
                echo "$@" >> '\(dir.path)/argv'
                case "$*" in
                *sign-registration*|*"runner claim"*) cat '\(dir.path)/proof.json' ;;
                *"push pair"*) cat > '\(dir.path)/stdin' ;;
                *"push status"*) echo '\(status)' ;;
                *"push forget"*) echo forgotten ;;
                esac
                """
            let path = dir.appendingPathComponent("farcooler")
            try script.write(to: path, atomically: true, encoding: .utf8)
            try FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: path.path)
            runners = Runners(storageKey: key)
            runners.executableForTesting = path.path
            runners.directoryForTesting = DirectoryClient { [unowned self] path, _ in
                switch path {
                case "/v1/directory/me":
                    return .success(DirectoryReply(status: 200, body: [
                        "user": ["id": "user_01ME"],
                        "teams": [
                            ["id": "team-personal", "name": "Personal", "personal": true, "role": "admin"],
                            ["id": "team-guild", "name": RunnerTeamTests.guild.name, "personal": false, "role": "member"],
                        ],
                    ]))
                case "/v1/directory/runners/claim":
                    return self.claimAnswer
                default:
                    return self.registerAnswer
                }
            }
        }

        var argv: String { (try? String(contentsOf: dir.appendingPathComponent("argv"), encoding: .utf8)) ?? "" }
        var piped: String? { try? String(contentsOf: dir.appendingPathComponent("stdin"), encoding: .utf8) }

        func clean() {
            try? FileManager.default.removeItem(at: dir)
            UserDefaults.standard.removeObject(forKey: key)
        }
    }

    @Test func theTokenReachesTheRunnerOnItsStdinAndNeverOnACommandLine() async throws {
        let scratch = try Scratch()
        defer { scratch.clean() }
        let outcome = await scratch.runners.join("dev@box", team: Self.guild)

        #expect(outcome.succeeded, "\(outcome.message)")
        // What `ps` would have shown for every process this pairing started.
        #expect(!scratch.argv.contains(Self.token), "the token was on a command line:\n\(scratch.argv)")
        #expect(scratch.argv.contains("--host dev@box runner sign-registration --account user_01ME --team team-guild"))
        #expect(scratch.argv.contains("--host dev@box push pair"))
        // What the runner was handed.
        #expect(scratch.piped == Self.token)
        // And nothing the person reads carries it.
        #expect(!outcome.message.contains(Self.token))
    }

    @Test func aRefusedRegistrationPairsNothingAndLeavesTheRunnerAsItWas() async throws {
        let scratch = try Scratch()
        defer { scratch.clean() }
        scratch.runners.add("dev@box")
        scratch.registerAnswer = .success(DirectoryReply(status: 409, body: ["error": "registered"]))
        let outcome = await scratch.runners.join("dev@box", team: Self.personal)

        #expect(!outcome.succeeded)
        #expect(outcome.message.contains("someone else registered it"))
        #expect(!outcome.message.contains("claim"))
        #expect(scratch.piped == nil, "a token was piped after a refusal")
        #expect(!scratch.argv.contains("push pair"))
        #expect(scratch.runners.all.first?.probe?.paired == nil)
    }

    @Test func thisMacsOwnPairingIsReadFromItsLastStatusLine() async throws {
        let paired = try Scratch(status: "paired · relay https://relay.test")
        defer { paired.clean() }
        await paired.runners.refreshLocalPairing()
        #expect(paired.runners.localPaired == true)
        let unpaired = try Scratch(status: "not paired")
        defer { unpaired.clean() }
        await unpaired.runners.refreshLocalPairing()
        #expect(unpaired.runners.localPaired == false)
    }

    @Test func aBannerBeforeStatusDoesNotPassForPairedAndNeitherLineIsUnknown() {
        #expect(Runners.paired(inStatus: "Welcome to box\nnot paired") == false)
        #expect(Runners.paired(inStatus: "last login: never\npaired · relay x") == true)
        #expect(Runners.paired(inStatus: "Welcome to box") == nil)
        #expect(Runners.paired(inStatus: "") == nil)
    }

    @Test func aProbeCarriesPairedAndTheHostnameInItsOneAnswer() throws {
        let json = #"{"platform":"linux","installedCli":"farcooler 1","installedDaemon":"farcoolerd 1","hostname":"build-box","paired":true}"#
        let probe = try JSONDecoder().decode(RunnerProbe.self, from: Data(json.utf8))
        #expect(probe.hostname == "build-box" && probe.paired == true)
        // A runner from before this decodes: both are just unknown.
        let old = try JSONDecoder().decode(RunnerProbe.self, from: Data(#"{"platform":"linux"}"#.utf8))
        #expect(old.hostname == nil && old.paired == nil)
    }

    // MARK: - What the team sees

    @Test func theTeamSeesAComputerNameOrAHostnameNeverAUserAndAddress() {
        #expect(Runners.directoryName(target: "", hostname: nil, computerName: "E-Liang’s MacBook Pro") == "E-Liang’s MacBook Pro")
        #expect(Runners.directoryName(target: "", hostname: nil, computerName: nil) == "Mac")
        #expect(Runners.directoryName(target: "alexandra@10.0.4.17", hostname: "build-box", computerName: nil) == "build-box")
        // No probe yet: the host part, never the user.
        #expect(Runners.directoryName(target: "alexandra@build.example.com", hostname: nil, computerName: nil) == "build.example.com")
        #expect(Runners.directoryName(target: "build-alias", hostname: " ", computerName: nil) == "build-alias")
        // An address names nobody.
        #expect(Runners.directoryName(target: "alexandra@10.0.4.17", hostname: nil, computerName: nil) == "Runner")
        #expect(Runners.directoryName(target: "[::1]", hostname: nil, computerName: nil) == "Runner")
        #expect(Runners.directoryName(target: "fe80::1", hostname: nil, computerName: nil) == "Runner")
        #expect(Runners.directoryName(target: "me@[2001:db8::7]", hostname: nil, computerName: nil) == "Runner")
        #expect(Runners.directoryName(target: "me@abc:def", hostname: nil, computerName: nil) == "Runner")
    }

    @Test func thisMacsNameComesFromTheDynamicStoreWithoutTouchingTheNetwork() {
        // Whatever the machine is called, it's a name and not nil, on a Mac that has one.
        #expect(Runners.computerName()?.isEmpty == false)
    }

    @Test func theNameTheRelayGetsIsTheHostnameNotTheTarget() async throws {
        let scratch = try Scratch()
        defer { scratch.clean() }
        scratch.runners.add("alexandra@10.0.4.17")
        scratch.runners.record(RunnerProbe(hostname: "build-box"), for: "alexandra@10.0.4.17")
        var sent: [String: Any] = [:]
        let inner = scratch.runners.directoryForTesting
        _ = inner
        scratch.runners.directoryForTesting = DirectoryClient { path, body in
            if path.hasSuffix("/me") {
                return .success(DirectoryReply(status: 200, body: ["user": ["id": "u"], "teams": [["id": "team-personal", "name": "Personal", "personal": true, "role": "admin"]]]))
            }
            sent = body
            return .success(DirectoryReply(status: 200, body: ["token": RunnerTeamTests.token]))
        }
        _ = await scratch.runners.join("alexandra@10.0.4.17", team: Self.personal)
        #expect(sent["name"] as? String == "build-box", "\(sent)")
        #expect(!(sent["name"] as? String ?? "").contains("10.0.4.17"))
    }

    // MARK: - Moving a runner of yours

    @Test func aRunnerOfYoursInAnotherTeamAsksAndThenMovesWithItsClaim() async throws {
        let scratch = try Scratch()
        defer { scratch.clean() }
        scratch.runners.add("dev@box")
        scratch.registerAnswer = .success(DirectoryReply(status: 409, body: ["error": "elsewhere"]))
        let asked = await scratch.runners.join("dev@box", team: Self.guild)
        guard case .needsMove(_, let key) = asked else { Issue.record("expected a question, got \(asked)"); return }
        #expect(key == "ssh-ed25519 AAAA")
        #expect(scratch.piped == nil, "asking paired the runner")
        #expect(!scratch.argv.contains("claim"))

        let moved = await scratch.runners.join("dev@box", team: Self.guild, moveKey: "ssh-ed25519 AAAA")
        #expect(moved.succeeded, "\(moved.message)")
        #expect(scratch.argv.contains("--host dev@box runner claim --account user_01ME --team team-guild"))
        #expect(scratch.piped == Self.token)
        #expect(!scratch.argv.contains(Self.token))
    }

    @Test func joiningMarksTheRunnerPairedAndStoppingMarksItNot() async throws {
        let scratch = try Scratch()
        defer { scratch.clean() }
        scratch.runners.add("dev@box")
        scratch.runners.record(RunnerProbe(), for: "dev@box")
        _ = await scratch.runners.join("dev@box", team: Self.personal)
        #expect(scratch.runners.all.first?.probe?.paired == true)
        _ = await scratch.runners.unpairNotifications("dev@box")
        #expect(scratch.runners.all.first?.probe?.paired == false)
    }

    // MARK: - Captures

    /// The sheet in a window of `width`, off every screen.
    static func drawn(
        _ request: RunnerTeamRequest, teams: [DirectoryTeam], failure: (message: String, detail: String?, tooOld: Bool)? = nil, moveQuestion: String? = nil,
        width: CGFloat, scheme: ColorScheme
    ) async -> NSWindow {
        let sheet = RunnerTeamSheet(
            request: request, onDone: { _ in }, onInstallAgain: { _ in }, loaded: .teams(teams), failure: failure,
            moveQuestion: moveQuestion
        )
        // A sheet's window draws this behind it; a borderless test window
        // doesn't. style-exempt: the system window background, as the sheet's window has
        .background(Color(nsColor: .windowBackgroundColor))
        .frame(width: width)
        .environment(\.colorScheme, scheme)
        let host = NSHostingView(rootView: sheet)
        let height = max(120, host.fittingSize.height)
        let window = NSWindow(
            contentRect: NSRect(x: -4000, y: -4000, width: width, height: height),
            styleMask: [.borderless], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.appearance = NSAppearance(named: scheme == .dark ? .darkAqua : .aqua)
        window.contentView = host
        TestWindow.show(window)
        for _ in 0..<10 {
            host.layoutSubtreeIfNeeded()
            try? await Task.sleep(for: .milliseconds(30))
        }
        let settled = host.fittingSize.height
        window.setContentSize(NSSize(width: width, height: max(120, settled)))
        host.layoutSubtreeIfNeeded()
        return window
    }

    @Test("The team sheet draws its list and its button at the narrowest and widest widths")
    func theSheetDraws() async {
        let teams = [Self.personal, Self.guild]
        for width in [320.0, 520.0] {
            let window = await Self.drawn(
                RunnerTeamRequest(target: "dev@box", purpose: .addToTeam), teams: teams, width: width, scheme: .light)
            let size = window.contentView?.fittingSize ?? .zero
            window.close()
            #expect(size.width <= width + 1, "the sheet is wider than its window at \(width): \(size)")
            #expect(size.height > 200, "the sheet collapsed at \(width): \(size)")
        }
    }

    @Test("Captures of the team sheet", .enabled(if: ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"] != nil))
    func captures() async throws {
        let out = URL(fileURLWithPath: ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"]!)
        try FileManager.default.createDirectory(at: out, withIntermediateDirectories: true)
        let many = (1...9).map {
            DirectoryTeam(
                id: "team-\($0)", name: "Customer Onboarding and Lifecycle Experiments, Region \($0), Shared Services",
                isPersonal: false, role: $0 % 3 == 0 ? "admin" : "member")
        }
        let long = "alexandra-montgomery-fitzgerald@platform-infrastructure.example-corporation.co.uk"
        let cases: [(String, RunnerTeamRequest, [DirectoryTeam], (message: String, detail: String?, tooOld: Bool)?, String?)] = [
            ("picker-short", RunnerTeamRequest(target: "dev@box", purpose: .notify), [Self.personal, Self.guild], nil, nil),
            ("picker-many", RunnerTeamRequest(target: long, purpose: .notify), [Self.personal] + many, nil, nil),
            ("add-short", RunnerTeamRequest(target: "dev@box", purpose: .addToTeam), [Self.personal, Self.guild], nil, nil),
            ("add-long", RunnerTeamRequest(target: long, purpose: .addToTeam), [Self.personal, Self.guild] + many, nil, nil),
            (
                "add-refused", RunnerTeamRequest(target: long, purpose: .addToTeam), [Self.personal, Self.guild],
                (DirectoryFailure.alreadyRegistered.message, nil, false), nil
            ),
            (
                "add-too-old", RunnerTeamRequest(target: "dev@box", purpose: .notify), [Self.personal, Self.guild],
                (
                    RunnerRegistration.signFailure(tooOld: true, name: "dev@box", verb: "added"),
                    "error: unrecognized subcommand 'sign-registration'", true
                ), nil
            ),
            (
                "move-question", RunnerTeamRequest(target: long, purpose: .addToTeam), [Self.personal, Self.guild], nil,
                "\(long) is already in another of your teams. Move it to \(Self.guild.name)? "
                    + "Anyone else with access to it loses that access."
            ),
        ]
        for (name, request, teams, failure, question) in cases {
            for width in [320.0, 420.0, 520.0] {
                for scheme in [ColorScheme.light, .dark] {
                    let window = await Self.drawn(request, teams: teams, failure: failure, moveQuestion: question, width: width, scheme: scheme)
                    let view = try #require(window.contentView)
                    let rep = try #require(view.bitmapImageRepForCachingDisplay(in: view.bounds))
                    view.cacheDisplay(in: view.bounds, to: rep)
                    let png = try #require(rep.representation(using: .png, properties: [:]))
                    try png.write(to: out.appendingPathComponent("mac-team-\(name)-\(Int(width))-\(scheme == .dark ? "dark" : "light").png"))
                    window.close()
                }
            }
        }
    }
}
