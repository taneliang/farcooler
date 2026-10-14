import Foundation
import Testing

@testable import AgentKit

/// Putting a runner in a team from the Mac (ov-528).
///
/// The relay is a script (`Relay`) and the runner is a recording (`Runner`);
/// the sequence between them, and every sentence a person reads when it goes
/// wrong, is the production code.
@MainActor
struct RunnerRegistrationTests {
    static let token = "tok-5f3a9c1e-never-shown"
    static let personal = #"{"id":"team-personal","name":"Personal","personal":true,"role":"admin"}"#
    static let big = #"{"id":"team-big","name":"The Platform Infrastructure and Developer Experience Guild of Northern Europe","personal":false,"role":"member"}"#
    static let billing = #"{"id":"team-bill","name":"Finance","personal":false,"role":"billing"}"#

    /// What the runner prints for `sign-registration`.
    static let proof =
        #"{"install":"inst-1","runnerKey":"ssh-ed25519 AAAA","relay":"https://relay.test","issuedAt":1790000000000,"proof":"-----BEGIN SSH SIGNATURE-----"}"#

    /// A relay that answers from a script and remembers what it was asked.
    @MainActor final class Relay {
        var teams = [RunnerRegistrationTests.personal, RunnerRegistrationTests.big, RunnerRegistrationTests.billing]
        var register: Result<DirectoryReply, AccountError> = .success(
            DirectoryReply(status: 200, body: ["runner": "r1", "team": "team-personal", "token": RunnerRegistrationTests.token]))
        var me: Result<DirectoryReply, AccountError>?
        var claim: Result<DirectoryReply, AccountError> = .success(DirectoryReply(status: 200, body: [:]))
        private(set) var asked: [(path: String, body: [String: Any])] = []

        var client: DirectoryClient {
            DirectoryClient { path, body in
                self.asked.append((path, body))
                switch path {
                case "/v1/directory/me":
                    if let me = self.me { return me }
                    let json = #"{"user":{"id":"user_01ME","email":"me@example.com"},"teams":[\#(self.teams.joined(separator: ","))],"devices":[]}"#
                    let object = try! JSONSerialization.jsonObject(with: Data(json.utf8)) as! [String: Any]
                    return .success(DirectoryReply(status: 200, body: object))
                case "/v1/directory/runners/register":
                    return self.register
                case "/v1/directory/runners/claim":
                    return self.claim
                default:
                    return .failure(.endpointMissing)
                }
            }
        }

        func reply(_ status: Int, _ error: String) -> Result<DirectoryReply, AccountError> {
            .success(DirectoryReply(status: status, body: ["error": error]))
        }
    }

    /// A runner whose `farcooler` answers from a script.
    @MainActor final class Runner {
        var signs = RunnerRegistration.CLIOutput(ok: true, output: RunnerRegistrationTests.proof, said: RunnerRegistrationTests.proof)
        var pairs = RunnerRegistration.CLIOutput(ok: true, output: "paired", said: "paired")
        private(set) var calls: [(arguments: [String], stdin: String?)] = []

        func run(_ arguments: [String], _ stdin: String?) async -> RunnerRegistration.CLIOutput {
            calls.append((arguments, stdin))
            return arguments.contains("sign-registration") || arguments.contains("claim") ? signs : pairs
        }
    }

    func registration(
        _ relay: Relay, _ runner: Runner, address: String = "https://relay.test",
        defaultRelay: String = "https://relay.test"
    ) -> RunnerRegistration {
        RunnerRegistration(
            directory: relay.client, relay: address, defaultRelay: defaultRelay, run: { await runner.run($0, $1) })
    }

    static let personalTeam = DirectoryTeam(id: "team-personal", name: "Personal", isPersonal: true, role: "admin")
    static let bigTeam = DirectoryTeam(
        id: "team-big", name: "The Platform Infrastructure and Developer Experience Guild of Northern Europe",
        isPersonal: false, role: "member")

    // MARK: - The sequence

    @Test func aRunnerIsRegisteredInTheChosenTeamAndThenPaired() async throws {
        let relay = Relay()
        let runner = Runner()
        let outcome = await registration(relay, runner).register(target: "dev@box", team: Self.bigTeam, directoryName: "build-box")

        #expect(outcome.succeeded)
        // The relay was asked who this is, then to register under the chosen
        // team with the runner's own proof and nothing else.
        #expect(relay.asked.map(\.path) == ["/v1/directory/me", "/v1/directory/runners/register"])
        let body = relay.asked[1].body
        #expect(body["team"] as? String == "team-big")
        #expect(body["install"] as? String == "inst-1")
        #expect(body["runnerKey"] as? String == "ssh-ed25519 AAAA")
        #expect((body["issuedAt"] as? NSNumber)?.int64Value == 1_790_000_000_000)
        #expect(body["proof"] as? String == "-----BEGIN SSH SIGNATURE-----")
        // The team sees the runner's own name, never the ssh target.
        #expect(body["name"] as? String == "build-box")
        #expect(Set(body.keys) == ["team", "name", "install", "runnerKey", "issuedAt", "proof"])
        // The runner was asked for the proof first, naming this account, this
        // team and this relay; and paired second.
        #expect(
            runner.calls[0].arguments == [
                "--host", "dev@box", "runner", "sign-registration", "--account", "user_01ME", "--team", "team-big",
                "--relay", "https://relay.test",
            ])
        #expect(runner.calls[1].arguments == ["--host", "dev@box", "push", "pair"])
    }

    @Test func thisMacIsAskedWithoutAHostFlag() async {
        let runner = Runner()
        _ = await registration(Relay(), runner).register(target: "", team: Self.personalTeam, directoryName: "build-box")
        #expect(!runner.calls.flatMap(\.arguments).contains("--host"))
        #expect(runner.calls[1].arguments == ["push", "pair"])
    }

    // MARK: - The token

    @Test func theTokenGoesDownThePipeAndIsNeverAnArgument() async {
        let relay = Relay()
        let runner = Runner()
        _ = await registration(relay, runner).register(target: "dev@box", team: Self.personalTeam, directoryName: "build-box")

        #expect(runner.calls[1].stdin == Self.token)
        for call in runner.calls {
            #expect(!call.arguments.contains { $0.contains(Self.token) }, "the token was in argv: \(call.arguments)")
        }
        #expect(runner.calls[0].stdin == nil)
    }

    @Test func theTokenIsNotInWhatThePersonReadsEvenWhenTheRunnerEchoesIt() async {
        let relay = Relay()
        let runner = Runner()
        runner.pairs = .init(ok: false, output: "", said: "pairing failed: \(Self.token) was refused")
        let failed = await registration(relay, runner).register(target: "dev@box", team: Self.personalTeam, directoryName: "build-box")

        guard case .failed(let message, let detail, _) = failed else { Issue.record("expected a failure"); return }
        #expect(!message.contains(Self.token))
        #expect(!(detail ?? "").contains(Self.token), "the runner's echo reached the screen: \(detail ?? "")")

        // And the success sentence, and every refusal the relay can give.
        let ok = await registration(Relay(), Runner()).register(target: "dev@box", team: Self.personalTeam, directoryName: "build-box")
        #expect(!ok.message.contains(Self.token))
        for status in [400, 403, 404, 409, 429, 500, 418] {
            let relay = Relay()
            relay.register = relay.reply(status, "anything")
            let outcome = await registration(relay, Runner()).register(target: "dev@box", team: Self.personalTeam, directoryName: "build-box")
            #expect(!outcome.message.contains(Self.token), "\(status)")
        }
    }

    @Test func nothingInThePairingPathLogsOrPrints() throws {
        // The token is bound in `RunnerRegistration.register` and in
        // `DirectoryClient.register`; neither may write anywhere. A source
        // check, because a log line is exactly the thing a runtime test of the
        // happy path cannot see.
        let sources = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("Sources/AgentKit")
        for file in ["RunnerRegistration.swift", "Directory.swift", "AccountDirectory.swift"] {
            let text = try String(contentsOf: sources.appendingPathComponent(file), encoding: .utf8)
            let code = text.split(separator: "\n").filter { !$0.trimmingCharacters(in: .whitespaces).hasPrefix("//") }
            for word in ["print(", "NSLog", "Logger", "os_log", "debugPrint", "dump(", ".log("] {
                #expect(!code.contains { $0.contains(word) }, "\(file) uses \(word)")
            }
        }
    }

    // MARK: - The team

    @Test func theDefaultIsThePersonalTeamAndBillingTeamsAreNotOffered() async throws {
        let relay = Relay()
        let me = try await relay.client.me().get()
        #expect(me.defaultTeam?.id == "team-personal")
        let offered = try await registration(relay, Runner()).teams().get()
        #expect(offered.map(\.id) == ["team-personal", "team-big"])
    }

    @Test func aTeamThePersonCanNoLongerAddToStopsBeforeTheRunnerIsAsked() async {
        let relay = Relay()
        let runner = Runner()
        let billing = DirectoryTeam(id: "team-bill", name: "Finance", isPersonal: false, role: "billing")
        let outcome = await registration(relay, runner).register(target: "dev@box", team: billing, directoryName: "build-box")

        #expect(!outcome.succeeded)
        #expect(outcome.message.contains("isn’t available to you"))
        #expect(runner.calls.isEmpty)
        #expect(relay.asked.map(\.path) == ["/v1/directory/me"])
    }

    // MARK: - The runner's side

    @Test func aRunnerTooOldToSignIsToldToReinstallAndTheRelayIsNeverAsked() async {
        let relay = Relay()
        let runner = Runner()
        runner.signs = .init(ok: false, output: "", said: "error: unrecognized subcommand 'sign-registration'")
        let outcome = await registration(relay, runner).register(target: "dev@box", team: Self.personalTeam, directoryName: "build-box")

        guard case .failed(_, _, let reason) = outcome else { Issue.record("expected a failure"); return }
        #expect(reason == .runnerTooOld)
        #expect(outcome.message == "dev@box wasn’t added because the Far Cooler on it is too old to prove it’s yours. Install Far Cooler on it again, then retry.")
        #expect(relay.asked.map(\.path) == ["/v1/directory/me"])
        #expect(runner.calls.count == 1)
    }

    @Test func aRunnerThatCannotBeReachedSaysWhatDidNotHappenAndWhy() async {
        let runner = Runner()
        runner.signs = .init(ok: false, output: "", said: "ssh: connect to host box port 22: Connection refused")
        let outcome = await registration(Relay(), runner).register(target: "dev@box", team: Self.personalTeam, directoryName: "build-box")

        #expect(outcome.message.hasPrefix("dev@box wasn’t added because it couldn’t prove it’s yours."))
        #expect(outcome.message.contains("SSH"))
        guard case .failed(_, let detail, _) = outcome else { Issue.record("expected a failure"); return }
        #expect(detail?.contains("Connection refused") == true)
    }

    @Test func aProofThatIsNotAProofIsNotPostedToTheRelay() async {
        let relay = Relay()
        let runner = Runner()
        runner.signs = .init(ok: true, output: "welcome to box\n", said: "welcome to box")
        let outcome = await registration(relay, runner).register(target: "dev@box", team: Self.personalTeam, directoryName: "build-box")

        #expect(!outcome.succeeded)
        #expect(!relay.asked.map(\.path).contains("/v1/directory/runners/register"))
        #expect(runner.calls.count == 1)
    }

    // MARK: - The relay's side

    @Test func aRunnerSomeoneElseRegisteredSaysSoAndOffersNoCommand() async {
        let relay = Relay()
        relay.register = relay.reply(409, "registered")
        let runner = Runner()
        let outcome = await registration(relay, runner).register(target: "dev@box", team: Self.personalTeam, directoryName: "build-box")

        #expect(!outcome.succeeded)
        #expect(outcome.message == "This runner wasn’t added because someone else registered it. Ask them to remove it from their team, then try again.")
        // No command to run: nothing posts a claim for someone else's runner.
        #expect(!outcome.message.contains("claim"))
        // Nothing was paired: there was no token.
        #expect(runner.calls.count == 1)
    }

    @Test(arguments: [
        (403, "forbidden", "Your role in that team doesn’t let you add runners"),
        (403, "stale", "Check that the runner’s clock is right"),
        (403, "proof", "didn’t accept the runner’s proof"),
        (400, "name", "didn’t accept its name"),
        (404, "not found", "isn’t available to you any more"),
        (400, "runnerKey", "didn’t make sense to the relay"),
        (418, "teapot", "The relay turned this request down"),
    ])
    func everyRefusalHasItsOwnSentence(status: Int, code: String, says: String) async {
        let relay = Relay()
        relay.register = relay.reply(status, code)
        let outcome = await registration(relay, Runner()).register(target: "dev@box", team: Self.personalTeam, directoryName: "build-box")

        #expect(!outcome.succeeded)
        #expect(outcome.message.contains(says), "\(status) \(code): \(outcome.message)")
        // Each says what didn't happen, in the first clause or two.
        #expect(outcome.message.contains("wasn’t added") || outcome.message.contains("isn’t available"))
    }

    @Test func aRelayThatCannotBeReachedOrWillNotAnswerIsNamedAsThat() async {
        for (answer, says) in [
            (Result<DirectoryReply, AccountError>.failure(.unreachable(reason: "offline")), "Couldn’t reach the relay"),
            (.failure(.unauthorized), "wouldn’t accept your session"),
            (.failure(.notSignedIn), "not signed in"),
        ] {
            let relay = Relay()
            relay.me = answer
            let runner = Runner()
            let outcome = await registration(relay, runner).register(target: "dev@box", team: Self.personalTeam, directoryName: "build-box")
            #expect(outcome.message.contains(says), "\(outcome.message)")
            #expect(runner.calls.isEmpty)
        }
    }

    @Test func aRelayWithoutADirectoryIsNotMistakenForAMissingTeam() async {
        let relay = Relay()
        relay.me = relay.reply(404, "not found")
        let outcome = await registration(relay, Runner()).register(target: "dev@box", team: Self.personalTeam, directoryName: "build-box")
        #expect(outcome.message.contains("doesn’t offer what the app asked for"))
    }

    @Test func aTokenThatCouldNotBeSavedOnTheRunnerSaysTheRunnerStoppedNotifying() async {
        let runner = Runner()
        runner.pairs = .init(ok: false, output: "", said: "pairing failed on dev@box")
        let outcome = await registration(Relay(), runner).register(target: "dev@box", team: Self.bigTeam, directoryName: "build-box")

        #expect(outcome.message.hasPrefix("dev@box is in \(Self.bigTeam.name), but it couldn’t finish setting up notifications"))
        #expect(outcome.message.contains("won’t notify your devices until you try again"))
    }

    // MARK: - The relay address

    @Test func theProofNamesTheRelaysOriginAndAPathNeverReachesIt() async {
        let runner = Runner()
        _ = await registration(Relay(), runner, address: "https://relay.test:8443/v1/", defaultRelay: "https://relay.test:8443")
            .register(target: "dev@box", team: Self.personalTeam, directoryName: "build-box")
        let arguments = runner.calls[0].arguments
        #expect(arguments.last == "https://relay.test:8443")
        // The runner's default is the same relay, so it isn't told again.
        #expect(!runner.calls[1].arguments.contains("--relay"))
    }

    @Test func aSelfHostedRelayIsPassedToThePairingSoTheRunnerBeatsAtTheRightOne() async {
        let runner = Runner()
        _ = await registration(Relay(), runner, address: "https://relay.mine.test", defaultRelay: "https://relay.farcooler.com")
            .register(target: "dev@box", team: Self.personalTeam, directoryName: "build-box")
        #expect(runner.calls[1].arguments == ["--host", "dev@box", "push", "pair", "--relay", "https://relay.mine.test"])
    }

    @Test func aRelayUnderAPathPrefixKeepsItsPrefixForThePairingButNotForTheProof() async {
        let runner = Runner()
        _ = await registration(Relay(), runner, address: "https://Host.Test:443/relay/", defaultRelay: "https://relay.farcooler.com")
            .register(target: "dev@box", team: Self.personalTeam, directoryName: "b")
        #expect(runner.calls[0].arguments.last == "https://host.test")
        #expect(runner.calls[1].arguments.suffix(2) == ["--relay", "https://Host.Test:443/relay"])
    }

    @Test func anOriginIsSpelledTheWayTheRelaySpellsItsOwn() {
        #expect(RunnerRegistration.origin(of: "HTTPS://Relay.Example:443/x") == "https://relay.example")
        #expect(RunnerRegistration.origin(of: "http://relay.example:80") == "http://relay.example")
        #expect(RunnerRegistration.origin(of: "https://relay.example:8443/v1") == "https://relay.example:8443")
        #expect(RunnerRegistration.origin(of: "https://[::1]:8443") == "https://[::1]:8443")
    }

    // MARK: - Moving a runner

    @Test func aRunnerOfYoursInAnotherTeamAsksBeforeItMovesAndSignsNoClaim() async {
        let relay = Relay()
        relay.register = relay.reply(409, "elsewhere")
        let runner = Runner()
        let outcome = await registration(relay, runner).register(target: "dev@box", team: Self.bigTeam, directoryName: "build-box")

        guard case .needsMove(let message, let key) = outcome else { Issue.record("expected a question, got \(outcome)"); return }
        #expect(message.hasPrefix("dev@box is already in another of your teams. Move it to \(Self.bigTeam.name)?"))
        #expect(message.hasSuffix("Anyone else with access to it loses that access."))
        #expect(!message.contains("token"))
        // The key the registration showed, for the move to hold the runner to.
        #expect(key == "ssh-ed25519 AAAA")
        // Asking changed nothing: one signature for the registration, no claim, no pairing.
        #expect(runner.calls.count == 1)
        #expect(!runner.calls.flatMap(\.arguments).contains("claim"))
        #expect(!relay.asked.map(\.path).contains("/v1/directory/runners/claim"))
    }

    @Test func movingSignsAClaimPostsItAndPipesTheNewToken() async {
        let relay = Relay()
        relay.claim = .success(DirectoryReply(status: 200, body: ["runner": "r1", "team": "team-big", "token": RunnerRegistrationTests.token]))
        let runner = Runner()
        let outcome = await registration(relay, runner).move(target: "dev@box", team: Self.bigTeam, directoryName: "build-box", runnerKey: "ssh-ed25519 AAAA")

        #expect(outcome.succeeded)
        #expect(outcome.message == "dev@box moved to \(Self.bigTeam.name) and can notify your devices.")
        #expect(relay.asked.map(\.path) == ["/v1/directory/me", "/v1/directory/runners/claim"])
        #expect(runner.calls[0].arguments.contains("claim") && !runner.calls[0].arguments.contains("sign-registration"))
        #expect(runner.calls[1].stdin == Self.token)
        #expect(!runner.calls.flatMap(\.arguments).contains { $0.contains(Self.token) })
        #expect(!outcome.message.contains(Self.token))
    }

    @Test func aClaimSignedByAnotherKeyIsRefusedAndNothingIsPostedOrPaired() async {
        let relay = Relay()
        relay.claim = .success(DirectoryReply(status: 200, body: ["token": RunnerRegistrationTests.token]))
        let runner = Runner()
        // The runner now answers as a different one from the key the question was about.
        let outcome = await registration(relay, runner).move(
            target: "dev@box", team: Self.bigTeam, directoryName: "b", runnerKey: "ssh-ed25519 SOMEONEELSE")

        #expect(outcome.message == "dev@box wasn’t moved because it answered as a different runner.")
        #expect(!relay.asked.map(\.path).contains("/v1/directory/runners/claim"))
        #expect(runner.calls.count == 1, "something was piped to a runner that answered as another")
    }

    @Test func aMoveThatTheRelayRefusesSaysWhyAndPairsNothing() async {
        for (status, code, says) in [(404, "not found", "doesn’t have it any more"), (403, "proof", "didn’t accept the runner’s proof")] {
            let relay = Relay()
            relay.claim = relay.reply(status, code)
            let runner = Runner()
            let outcome = await registration(relay, runner).move(target: "dev@box", team: Self.bigTeam, directoryName: "b", runnerKey: "ssh-ed25519 AAAA")
            #expect(!outcome.succeeded)
            #expect(outcome.message.contains(says), "\(outcome.message)")
            #expect(runner.calls.count == 1)
        }
    }

    @Test func aMoveIsNeverOfferedWhenTheRunnerIsSomeoneElses() async {
        let relay = Relay()
        relay.register = relay.reply(409, "registered")
        let outcome = await registration(relay, Runner()).register(target: "dev@box", team: Self.bigTeam, directoryName: "b")
        if case .needsMove = outcome { Issue.record("offered to move someone else's runner") }
    }

    // MARK: - The name

    @Test func aNameIsCutToTheRelaysLimitByUTF16UnitsNotCharacters() {
        let emoji = String(repeating: "👩🏽‍💻", count: 30)  // one character, 7 UTF-16 units each
        let cut = DirectoryClient.limited(emoji)
        #expect(cut.utf16.count <= 80)
        #expect(cut.count == 11)
        #expect(DirectoryClient.limited("short") == "short")
    }

    // MARK: - A chatty login shell

    @Test func aBannerBeforeTheProofDoesNotMakeItUnreadable() {
        let noisy = "Welcome to build-box\nnvm: using node 22\n" + Self.proof + "\n"
        #expect(RunnerProof(printed: noisy)?.install == "inst-1")
        #expect(RunnerProof(printed: "{not json}\nWelcome") == nil)
    }

    @Test func aProofThatCannotBeReadShowsTheRunnersWordsAndDoesNotBlameTheInstall() async {
        let runner = Runner()
        runner.signs = .init(ok: true, output: "welcome\n", said: "welcome")
        let outcome = await registration(Relay(), runner).register(target: "dev@box", team: Self.personalTeam, directoryName: "b")
        guard case .failed(let message, let detail, _) = outcome else { Issue.record("expected a failure"); return }
        #expect(!message.contains("Reinstall"))
        #expect(detail == "welcome")
    }

    // MARK: - The proof

    @Test func aPrintedProofIsReadAndAHalfProofIsNot() {
        #expect(RunnerProof(printed: Self.proof)?.issuedAt == 1_790_000_000_000)
        #expect(RunnerProof(printed: #"{"install":"i","runnerKey":"k","issuedAt":1}"#) == nil)
        #expect(RunnerProof(printed: #"{"install":"","runnerKey":"k","issuedAt":1,"proof":"p"}"#) == nil)
        #expect(RunnerProof(printed: "not json") == nil)
    }
}
