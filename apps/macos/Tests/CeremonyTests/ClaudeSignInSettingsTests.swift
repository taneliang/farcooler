import AgentKit
import AppKit
import SwiftUI
import TestWindows
import Testing

@testable import Far_Cooler

/// A member signs in to Claude from a runner's settings sheet (ov-584): the
/// calls go through the CLI with the pasted code on a pipe, the Claude section
/// is a member's own, and the owner's People says each member's state.
@MainActor
@Suite(.serialized)
struct ClaudeSignInSettingsTests {
    /// `farcooler --json member list`, from a runner that signs a member in.
    /// `you` is "" for the owner, who is told every email.
    static func list(
        you: String, state: String = "none", email: String = "bob@claude.example", signIn: Bool = true
    ) -> String {
        let told = you.isEmpty || you == "bob"
        return #"{"members":[{"handle":"alice","name":"Alice Chen","devices":[],"reps":0,"rep_panes":[],"#
            + #""claude_login":{"state":"signed_in","email":"\#(you.isEmpty ? "alice@claude.example" : "")"}},"#
            + #"{"handle":"bob","name":"Bob","devices":[],"reps":0,"rep_panes":[],"#
            + #""claude_login":{"state":"\#(state)","email":"\#(told && state != "none" ? email : "")"}}],"#
            + #""claude_account":"","you":"\#(you)","claude_sign_in":\#(signIn)}"#
    }

    /// Every call a store made, in order: its arguments, and the input it was fed.
    final class Calls {
        var made: [(args: [String], input: String?)] = []
        var answer: ([String]) -> CLI.Result = { _ in CLI.Result(ok: true, output: "{}", errors: "") }
    }

    static func store(you: String, state: String = "none", signIn: Bool = true, calls: Calls = Calls()) -> RunnerSettingsStore {
        let store = RunnerSettingsStore(target: "box")
        store.cliForTesting = { args in
            calls.made.append((args, nil))
            if args.suffix(2) == ["member", "list"] {
                return CLI.Result(ok: true, output: list(you: you, state: state, signIn: signIn), errors: "")
            }
            if args.contains("settings") { return CLI.Result(ok: true, output: #"{"branchPrefix":""}"#, errors: "") }
            if args.contains("theme") { return CLI.Result(ok: true, output: #"{"themes":[]}"#, errors: "") }
            return calls.answer(args)
        }
        store.cliWithInputForTesting = { args, input in
            calls.made.append((args, input))
            return calls.answer(args)
        }
        return store
    }

    static let started =
        #"{"session":"0198f2c0-0000-7000-8000-00000000c0de","url":"https://claude.com/cai/oauth/authorize?code=true","expires_at":1760000000000}"#

    @Test("Signing in goes through the CLI: the session in the arguments, the code on a pipe")
    func calls() async throws {
        let calls = Calls()
        let store = Self.store(you: "bob", calls: calls)
        await store.load()
        let support = try #require(store.claudeSignIn(runner: "Studio"))
        calls.made = []
        calls.answer = { args in
            switch args.dropFirst(4).first {
            case "claude-sign-in": CLI.Result(ok: true, output: Self.started, errors: "")
            case "claude-sign-in-finish": CLI.Result(ok: true, output: #"{"state":"signed_in","email":"bob@claude.example"}"#, errors: "")
            default: CLI.Result(ok: true, output: #"{"state":"none","email":""}"#, errors: "")
            }
        }
        let started = try await support.calls.start("bob")
        #expect(started.session == "0198f2c0-0000-7000-8000-00000000c0de")
        #expect(started.url == "https://claude.com/cai/oauth/authorize?code=true")
        let login = try await support.calls.finish(started.session, "abc#def")
        #expect(login == ClaudeLogin(state: .signedIn, email: "bob@claude.example"))
        await support.calls.cancel(started.session)
        #expect(await support.signOut("bob"))
        let made = calls.made.map { ($0.args.dropFirst(2).joined(separator: " "), $0.input) }
        #expect(made.map(\.0) == [
            "--json member claude-sign-in bob",
            "--json member claude-sign-in-finish \(started.session)",
            "--json member claude-sign-in-cancel \(started.session)",
            "--json member claude-sign-out bob",
        ])
        // The code is the input of the finish and in no argument of any call.
        #expect(made.map(\.1) == [nil, "abc#def", nil, nil])
        #expect(!calls.made.contains { $0.args.joined(separator: " ").contains("abc") })
    }

    @Test("A refusal reaches the model with the runner's word and case; silence is no answer")
    func refusals() async throws {
        let calls = Calls()
        let store = Self.store(you: "bob", calls: calls)
        await store.load()
        let support = try #require(store.claudeSignIn(runner: "Studio"))
        calls.answer = { _ in
            CLI.Result(ok: false, output: "", errors: "error: the sign-in didn't accept that code\ncode: resource-conflict\nwhat: code_refused")
        }
        do {
            _ = try await support.calls.finish("s", "a#b")
            Issue.record("a refused code was taken")
        } catch {
            #expect(error as? ClaudeSignIn.CallError == .refused(word: "resource-conflict", what: "code_refused"))
        }
        calls.answer = { _ in CLI.Result(ok: false, output: "", errors: "ssh: connect to host box port 22: Operation timed out") }
        do {
            _ = try await support.calls.start("bob")
            Issue.record("a start was taken")
        } catch {
            #expect(error as? ClaudeSignIn.CallError == .unreachable)
        }
        #expect(await support.signOut("bob") == false)
    }

    @Test("A runner that doesn't sign in, and the owner's Mac, give no sign-in to offer")
    func noSupport() async throws {
        let old = Self.store(you: "bob", signIn: false)
        await old.load()
        #expect(old.claudeSignIn(runner: "Studio") == nil)
        let unread = RunnerSettingsStore(target: "box")
        #expect(unread.claudeSignIn(runner: "Studio") == nil)
        // The owner's list offers sign-in to the runner, but they have no login to make.
        let owner = Self.store(you: "")
        await owner.load()
        #expect(try #require(owner.people).ownClaudeLogin(signInOffered: true) == nil)
    }

    // MARK: - Drawn

    /// The settings sheet over `members`, probed; the window is the caller's to close.
    static func drawn(
        you: String, state: String = "none", scheme: ColorScheme = .light, name: String = "box"
    ) async -> (Set<String>, NSWindow) {
        let email = "bob.montgomery-fitzgerald@platform-infrastructure.example-corporation.com"
        return await RunnerPeopleSettingsTests.drawn(
            members: list(you: you, state: state, email: email), scheme: scheme, name: name)
    }

    @Test("A member's sheet has the Claude section, and the owner's has each member's state instead")
    func sections() async {
        let (member, memberWindow) = await Self.drawn(you: "bob")
        memberWindow.close()
        #expect(member.contains("claude-login-state") && member.contains("claude-sign-in"), "\(member)")
        // Their own login is the section's; the head line says nothing of one account.
        #expect(!member.contains("runner-people-account"), "\(member)")
        let (signedIn, signedInWindow) = await Self.drawn(you: "bob", state: "signed_in")
        signedInWindow.close()
        #expect(signedIn.contains("claude-sign-in-again") && signedIn.contains("claude-sign-out"), "\(signedIn)")
        let (owner, ownerWindow) = await Self.drawn(you: "")
        ownerWindow.close()
        #expect(!owner.contains("claude-login-state") && !owner.contains("claude-sign-in"), "\(owner)")
        #expect(owner.contains("runner-people-account"), "\(owner)")
    }

    // MARK: - Captures

    /// A runner whose calls answer from a script, for the sheet's states.
    static func support(runner: String, started: @escaping @Sendable () async throws -> ClaudeSignInModel.Started) -> ClaudeSignInSupport {
        ClaudeSignInSupport(
            runner: runner,
            calls: .init(
                start: { _ in try await started() },
                finish: { _, code in
                    if code.hasPrefix("silent") { throw ClaudeSignIn.CallError.unreachable }
                    if code.hasPrefix("refused") { throw ClaudeSignIn.CallError.refused(word: "resource-conflict", what: "code_refused") }
                    return ClaudeLogin(state: .signedIn, email: "bob.montgomery-fitzgerald@platform-infrastructure.example-corporation.com")
                },
                cancel: { _ in }),
            signOut: { _ in true }, changed: {})
    }

    static let email = "bob.montgomery-fitzgerald@platform-infrastructure.example-corporation.com"

    /// `expecting` names the step the model must be in, and `until` says so: the shot waits for it and fails
    /// without writing when it never comes (ov-642).
    static func shoot(
        _ model: ClaudeSignInModel, support: ClaudeSignInSupport, scheme: ColorScheme, to url: URL,
        expecting: String, until reached: @MainActor (ClaudeSignInModel) -> Bool
    ) async throws {
        try await Capture.wait(expecting: expecting, until: { reached(model) })
        let host = NSHostingView(
            rootView: ClaudeSignInSheet(model: model, support: support, openPage: { _ in true })
                // A sheet's window draws this behind it. style-exempt: the system window background, as the sheet's window has
                .background(Color(nsColor: .windowBackgroundColor))
                .environment(\.colorScheme, scheme))
        let size = host.fittingSize
        let window = NSWindow(
            contentRect: NSRect(x: -4000, y: -4000, width: ClaudeSignInSheet.width, height: max(size.height, 240)),
            styleMask: [.borderless], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.appearance = NSAppearance(named: scheme == .dark ? .darkAqua : .aqua)
        window.contentView = host
        TestWindow.show(window)
        await Settle.until { host.layoutSubtreeIfNeeded(); return true }
        await Settle.still(host, reading: { "\(host.fittingSize)" })
        window.setContentSize(NSSize(width: ClaudeSignInSheet.width, height: max(host.fittingSize.height, 240)))
        host.layoutSubtreeIfNeeded()
        try await Capture.keep(host, to: url, expecting: expecting, until: { reached(model) })
        window.close()
    }

    /// Every state of the sheet, and the settings sheet's Claude section, light and
    /// dark, with a long email and a long runner name, into `FARCOOLER_CAPTURE_OUT`.
    /// Opt-in: a capture tool, never run in CI or a normal run.
    @Test("Captures of the sign-in", .enabled(if: ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"] != nil))
    func captures() async throws {
        let out = URL(fileURLWithPath: ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"]!)
        try FileManager.default.createDirectory(at: out, withIntermediateDirectories: true)
        let longRunner = "Elias’s Mac Studio in the Platform Infrastructure Lab"
        let page = ClaudeSignInModel.Started(
            session: "s", url: "https://claude.com/cai/oauth/authorize?code=true&client_id=x&state=y")
        let code = "Hk3dP9aQ7rWm2VxZ4tN8sLbC6yFeJ1uGoI5pDnRqAh0#8f3b2c91-4d7e-4a6b-9c15-e2a7d4f60b83"
        for scheme in [ColorScheme.light, .dark] {
            let tag = scheme == .dark ? "dark" : "light"
            for (runner, label) in [("Studio", "short"), (longRunner, "long")] {
                let ready = Self.support(runner: runner, started: { page })
                func model(_ support: ClaudeSignInSupport) -> ClaudeSignInModel {
                    ClaudeSignInModel(handle: "bob", runner: runner, calls: support.calls)
                }
                // The link on its way: a start that never answers.
                let waiting = Self.support(runner: runner, started: { try await Task.sleep(for: .seconds(600)); return page })
                let starting = model(waiting)
                let begun = Task { await starting.begin() }
                try await Task.sleep(for: .milliseconds(300))
                try await Self.shoot(starting, support: waiting, scheme: scheme, to: out.appendingPathComponent("mac-sheet-starting-\(label)-\(tag).png"), expecting: "starting", until: { $0.step == .starting })
                begun.cancel()

                // A fresh model for each state: closing a sheet's window ends its
                // sign-in (`onDisappear`), so one shot can't carry on into the next.
                func shoot(_ name: String, _ m: ClaudeSignInModel, expecting: String, until: @MainActor (ClaudeSignInModel) -> Bool) async throws {
                    try await Self.shoot(m, support: ready, scheme: scheme, to: out.appendingPathComponent("mac-sheet-\(name)-\(label)-\(tag).png"), expecting: expecting, until: until)
                }
                func onPage(code: String = "") async -> ClaudeSignInModel {
                    let m = model(ready)
                    await m.begin()
                    m.code = code
                    return m
                }
                try await shoot("page", await onPage(), expecting: "the page step, code empty", until: { $0.isOnPage && $0.code.isEmpty })
                try await shoot("code", await onPage(code: code), expecting: "the page step, the whole code typed", until: { $0.isOnPage && $0.code == code })
                let incomplete = await onPage(code: "abcdef")
                await incomplete.finish()
                #expect(incomplete.codeNotice == ClaudeSignIn.incompleteCode, "no incomplete notice: \(incomplete.step)")
                try await shoot("incomplete", incomplete, expecting: "the incomplete-code notice", until: { $0.isOnPage && $0.codeNotice == ClaudeSignIn.incompleteCode })
                let signedIn = await onPage(code: code)
                await signedIn.finish()
                #expect(signedIn.step == .done(email: Self.email), "\(signedIn.step)")
                try await shoot("done", signedIn, expecting: "the Done step", until: { $0.step == .done(email: Self.email) })

                let silent = await onPage(code: "silent#x")
                await silent.finish()
                #expect(silent.codeNotice != nil && silent.code == "silent#x", "\(silent.step)")
                try await shoot("finish-unreachable", silent, expecting: "the page step with the unreachable notice", until: { $0.isOnPage && $0.codeNotice != nil && $0.code == "silent#x" })

                let refused = model(ready)
                await refused.begin()
                refused.code = "refused#x"
                await refused.finish()
                try await Self.shoot(refused, support: ready, scheme: scheme, to: out.appendingPathComponent("mac-sheet-refused-\(label)-\(tag).png"), expecting: "a failed step", until: { $0.isFailed })

                let expired = model(Self.support(runner: runner, started: { throw ClaudeSignIn.CallError.unreachable }))
                await expired.begin()
                try await Self.shoot(expired, support: ready, scheme: scheme, to: out.appendingPathComponent("mac-sheet-unreachable-\(label)-\(tag).png"), expecting: "a failed step", until: { $0.isFailed })
            }
            // The settings sheet's Claude section, not signed in and signed in, and the owner's People.
            // `shows` is an identifier only that state draws: Sign In for a member with no login, Sign Out once there is one,
            // and the owner's People line for the owner.
            for (name, you, state, shows) in [
                ("section-none", "bob", "none", "claude-sign-in"), ("section-signed-in", "bob", "signed_in", "claude-sign-out"),
                ("owner", "", "signed_in", "runner-people-account"),
            ] {
                let (ids, window) = await Self.drawn(you: you, state: state, scheme: scheme, name: longRunner)
                let view = try #require(window.contentView)
                try await Capture.keep(
                    view, to: out.appendingPathComponent("mac-\(name)-\(tag).png"), expecting: shows,
                    until: { ids.contains(shows) })
                window.close()
            }
        }
    }
}

extension ClaudeSignInModel {
    var isOnPage: Bool { if case .page = step { true } else { false } }
    var isFailed: Bool { if case .failed = step { true } else { false } }
}
