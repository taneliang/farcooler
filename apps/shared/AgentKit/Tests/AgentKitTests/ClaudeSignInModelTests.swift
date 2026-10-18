import Foundation
import Testing

@testable import AgentKit

/// The sign-in sheet's steps (ov-584), against a runner that records what it
/// was asked.
@MainActor
struct ClaudeSignInModelTests {
    /// What the model asked of the runner, and what it answers.
    final class Runner: @unchecked Sendable {
        var started: [String] = []
        var finished: [(String, String)] = []
        var cancelled: [String] = []
        var url = ClaudeSignInTests.claudeAi
        var expiresAt: Date?
        var startError: Error?
        var finishError: Error?
        var email = "alex@example.com"
        private var count = 0

        var calls: ClaudeSignInModel.Calls {
            .init(
                start: { [self] handle in
                    started.append(handle)
                    if let startError { throw startError }
                    count += 1
                    return .init(session: "session-\(count)", url: url, expiresAt: expiresAt)
                },
                finish: { [self] session, code in
                    finished.append((session, code))
                    if let finishError { throw finishError }
                    return ClaudeLogin(state: .signedIn, email: email)
                },
                cancel: { [self] session in cancelled.append(session) })
        }
    }

    func model(_ runner: Runner, now: @escaping () -> Date = Date.init) -> ClaudeSignInModel {
        ClaudeSignInModel(handle: "bob", runner: "Studio", calls: runner.calls, now: now)
    }

    @Test("A link comes, the member pastes the code, and they're signed in")
    func happyPath() async throws {
        let runner = Runner()
        let m = model(runner)
        #expect(m.step == .starting)
        await m.begin()
        #expect(m.step == .page(try #require(URL(string: ClaudeSignInTests.claudeAi))))
        #expect(runner.started == ["bob"])
        #expect(!m.canFinish)
        m.code = "  abc#def\n"
        #expect(m.canFinish)
        await m.finish()
        // The code goes as pasted, trimmed, and nowhere else.
        #expect(runner.finished.map { $0.0 } == ["session-1"] && runner.finished.map { $0.1 } == ["abc#def"])
        #expect(m.step == .done(email: "alex@example.com"))
        #expect(m.code.isEmpty)
        // Done: nothing left to cancel.
        await m.close()
        #expect(runner.cancelled.isEmpty)
    }

    @Test("A page the app won't open ends the sign-in, and the runner is told")
    func refusesAPage() async {
        for bad in ["https://evil.example/cai/oauth/authorize?x=1", "http://claude.com/cai/oauth/authorize?x=1", ""] {
            let runner = Runner()
            runner.url = bad
            let m = model(runner)
            await m.begin()
            #expect(
                m.step == .failed(.init("Couldn’t start sign-in on Studio. Claude Code there needs to be updated.")),
                "took \(bad)")
            #expect(runner.cancelled == ["session-1"])
            #expect(m.pageToOpen() == nil)
        }
    }

    @Test("A code with no # is said so without asking the runner, and the sign-in goes on")
    func incompleteCode() async {
        let runner = Runner()
        let m = model(runner)
        await m.begin()
        m.code = "abcdef"
        await m.finish()
        #expect(runner.finished.isEmpty)
        #expect(m.codeNotice == "That code isn’t complete. Copy the whole code and try again.")
        if case .page = m.step {} else { Issue.record("left the page step: \(m.step)") }
        m.code = "abc#def"
        await m.finish()
        #expect(m.codeNotice == nil && m.step == .done(email: "alex@example.com"))
    }

    @Test("A runner that finds the code incomplete leaves the sign-in up")
    func runnerSaysIncomplete() async {
        let runner = Runner()
        runner.finishError = ClaudeSignIn.CallError.refused(word: "invalid-argument", what: "sign_in_code")
        let m = model(runner)
        await m.begin()
        m.code = "a#b"
        await m.finish()
        #expect(m.codeNotice == ClaudeSignIn.incompleteCode)
        if case .page = m.step {} else { Issue.record("left the page step: \(m.step)") }
    }

    @Test("A code claude refuses ends the sign-in with Try Again, which asks for a new link")
    func codeRefused() async throws {
        let runner = Runner()
        runner.finishError = ClaudeSignIn.CallError.refused(word: "resource-conflict", what: "code_refused")
        let m = model(runner)
        await m.begin()
        m.code = "a#b"
        await m.finish()
        #expect(
            m.step
                == .failed(
                    .init("That code didn’t work. Open the sign-in page again to get a new one.", retry: .tryAgain)))
        runner.finishError = nil
        await m.begin()
        let page = try #require(URL(string: ClaudeSignInTests.claudeAi))
        #expect(runner.started.count == 2 && m.step == .page(page))
        #expect(m.code.isEmpty)
        m.code = "a#b"
        await m.finish()
        // The second link's session, not the spent first.
        #expect(runner.finished.last?.0 == "session-2")
    }

    @Test("A link that has run out is expired before the app opens it or sends a code to it")
    func expiry() async {
        let runner = Runner()
        runner.expiresAt = Date(timeIntervalSince1970: 1000)
        var clock = Date(timeIntervalSince1970: 900)
        let m = model(runner, now: { clock })
        await m.begin()
        #expect(m.pageToOpen() != nil)
        m.code = "a#b"
        clock = Date(timeIntervalSince1970: 1001)
        await m.finish()
        #expect(runner.finished.isEmpty)
        #expect(m.step == .failed(.init("The sign-in link expired.", retry: .startAgain)))
        await m.begin()
        clock = Date(timeIntervalSince1970: 1002)
        #expect(m.pageToOpen() == nil)
        #expect(m.step == .failed(.init("The sign-in link expired.", retry: .startAgain)))
    }

    @Test("A runner that doesn't answer says so, and offers Try Again")
    func unreachable() async {
        let runner = Runner()
        runner.startError = ClaudeSignIn.CallError.unreachable
        let m = model(runner)
        await m.begin()
        #expect(
            m.step
                == .failed(.init("Studio isn’t answering. Check your connection and try again.", retry: .tryAgain)))
        // An error this build can't place is a plain failure, not silence.
        runner.startError = URLError(.timedOut)
        await m.begin()
        #expect(m.step == .failed(.init("Couldn’t sign in. Try again.", retry: .tryAgain)))
    }

    @Test("Closing the sheet ends a sign-in under way, and one that arrives after it was closed")
    func closing() async {
        let runner = Runner()
        let m = model(runner)
        await m.begin()
        await m.close()
        #expect(runner.cancelled == ["session-1"])
        await m.close()
        #expect(runner.cancelled == ["session-1"])
        let late = Runner()
        let n = model(late)
        await n.close()
        await n.begin()
        #expect(late.started.isEmpty)
        #expect(n.pageToOpen() == nil)
    }

    @Test("Starting again ends the link before it")
    func restart() async {
        let runner = Runner()
        let m = model(runner)
        await m.begin()
        await m.begin()
        #expect(runner.cancelled == ["session-1"] && runner.started.count == 2)
    }

    // MARK: - A call in flight when the sheet closes

    /// A runner whose start and finish wait to be let go.
    final class Held: @unchecked Sendable {
        var cancelled: [String] = []
        private var startGate: CheckedContinuation<Void, Never>?
        private var finishGate: CheckedContinuation<Void, Never>?
        var holdingStart = false
        var holdingFinish = false

        var calls: ClaudeSignInModel.Calls {
            .init(
                start: { [self] _ in
                    if holdingStart { await withCheckedContinuation { startGate = $0 } }
                    return .init(session: "late", url: ClaudeSignInTests.claudeAi)
                },
                finish: { [self] _, _ in
                    if holdingFinish { await withCheckedContinuation { finishGate = $0 } }
                    return ClaudeLogin(state: .signedIn, email: "a@b.c")
                },
                cancel: { [self] session in cancelled.append(session) })
        }

        func releaseStart() { startGate?.resume(); startGate = nil }
        func releaseFinish() { finishGate?.resume(); finishGate = nil }
        var startWaiting: Bool { startGate != nil }
        var finishWaiting: Bool { finishGate != nil }
    }

    @Test("A link that arrives after the sheet closed is ended, and the screen isn't touched")
    func closeDuringStart() async {
        let held = Held()
        held.holdingStart = true
        let m = ClaudeSignInModel(handle: "bob", runner: "Studio", calls: held.calls)
        let begun = Task { await m.begin() }
        while !held.startWaiting { await Task.yield() }
        await m.close()
        held.releaseStart()
        await begun.value
        #expect(held.cancelled == ["late"])
        #expect(m.step == .starting)
    }

    @Test("A sign-in that lands after the sheet closed doesn't show Done")
    func closeDuringFinish() async {
        let held = Held()
        let m = ClaudeSignInModel(handle: "bob", runner: "Studio", calls: held.calls)
        await m.begin()
        held.holdingFinish = true
        m.code = "a#b"
        let finishing = Task { await m.finish() }
        while !held.finishWaiting { await Task.yield() }
        await m.close()
        held.releaseFinish()
        await finishing.value
        #expect(m.step == .finishing)
        #expect(held.cancelled == ["late"])
    }

    @Test("A start answered after a newer start is ended, not shown")
    func staleStart() async {
        let held = Held()
        held.holdingStart = true
        let m = ClaudeSignInModel(handle: "bob", runner: "Studio", calls: held.calls)
        let first = Task { await m.begin() }
        while !held.startWaiting { await Task.yield() }
        held.holdingStart = false
        await m.begin()
        held.releaseStart()
        await first.value
        #expect(held.cancelled == ["late"])
        if case .page = m.step {} else { Issue.record("lost the newer link: \(m.step)") }
    }

    @Test("The incomplete-code notice goes as soon as the text changes")
    func noticeClears() async {
        let runner = Runner()
        let m = model(runner)
        await m.begin()
        m.code = "abc"
        await m.finish()
        #expect(m.codeNotice != nil)
        m.code = "abcd"
        #expect(m.codeNotice == nil)
    }

    @Test("A finish with no answer keeps the sign-in and the code, since the runner may have taken it")
    func finishWithNoAnswer() async {
        let runner = Runner()
        let m = model(runner)
        await m.begin()
        m.code = "abc#def"
        runner.finishError = ClaudeSignIn.CallError.unreachable
        await m.finish()
        if case .page = m.step {} else { Issue.record("left the page step: \(m.step)") }
        #expect(m.code == "abc#def")
        #expect(m.codeNotice == "Studio isn’t answering. Check your connection and try again.")
        // Sent again, to the same session.
        runner.finishError = nil
        await m.finish()
        #expect(runner.finished.map { $0.0 } == ["session-1", "session-1"])
        #expect(m.step == .done(email: "alex@example.com"))
    }

    @Test("A sheet that has gone asks the runner for nothing")
    func noStartAfterClose() async {
        let runner = Runner()
        let m = model(runner)
        await m.close()
        await m.begin()
        #expect(runner.started.isEmpty && runner.cancelled.isEmpty)
    }

    @Test("A paste longer than any code is refused with supporting text, and the field stays as it was")
    func overLongPaste() async {
        let runner = Runner()
        let m = model(runner)
        await m.begin()
        m.code = "abc#def"
        m.code = String(repeating: "a", count: ClaudeSignIn.maxCode + 1)
        #expect(m.code == "abc#def")
        #expect(m.codeNotice == "That’s longer than a sign-in code.")
        if case .page = m.step {} else { Issue.record("left the page step") }
        m.code = String(repeating: "a", count: ClaudeSignIn.maxCode)
        #expect(m.code.utf8.count == ClaudeSignIn.maxCode && m.codeNotice == nil)
    }

    @Test("A page no browser took is said, and the sign-in stays up")
    func pageNotOpened() async {
        let runner = Runner()
        let m = model(runner)
        await m.begin()
        m.pageNotOpened()
        #expect(m.openNotice == ClaudeSignIn.pageNotOpened)
        #expect(!ClaudeSignIn.pageNotOpened.contains("updated"))
        if case .page = m.step {} else { Issue.record("left the page step") }
    }
}
