import Foundation
import Testing

@testable import AgentKit

/// Sign Out Everywhere (ov-564): what the app sends, how it reads the answer,
/// and that the Settings action can't fire without the confirmation.
@MainActor
struct SignOutEverywhereTests {
    /// The relay skips this install in the notice only if it is named.
    @Test("The request names this install's push token")
    func namesThePushToken() {
        #expect(Account.signOutEverywhereBody(push: "abc123")["pushToken"] as? String == "abc123")
        #expect(Account.signOutEverywhereBody(push: "").isEmpty)
        #expect(Account.signOutEverywhereBody(push: nil).isEmpty)
    }

    /// Anything but an explicit `true` is "not sure": a reply this app can't
    /// read must not pass for every session ended.
    @Test("Only an explicit complete: true counts as every session ended")
    func completeNeedsATrue() {
        #expect(Account.endedEverywhere(["ok": true, "complete": true, "ended": 2]))
        #expect(!Account.endedEverywhere(["ok": true, "complete": false, "ended": 0]))
        #expect(!Account.endedEverywhere(["ok": true]))
        #expect(!Account.endedEverywhere(["complete": "true"]))
        #expect(!Account.endedEverywhere([:]))
    }

    /// Pairings survive, so the person is told which are recent (M1).
    @Test("Recent pairings are named, and a clean sign-out says nothing")
    func namesRecentRunners() {
        #expect(Account.afterSignOutEverywhere(["complete": true, "runners": []]) == nil)
        let line = Account.afterSignOutEverywhere(
            ["complete": true, "runners": [["id": "a", "label": "Dev Box"], ["id": "b", "label": "CI"]]])
        #expect(line == "Runners paired recently: Dev Box, CI. Sign in and remove any you don’t recognize.")
        let partly = Account.afterSignOutEverywhere(["complete": false, "runners": [["label": "Dev Box"]]])
        #expect(partly?.hasPrefix(Account.partlySignedOutMessage) == true)
        #expect(partly?.contains("Dev Box") == true)
        #expect(Account.afterSignOutEverywhere(["complete": false]) == Account.partlySignedOutMessage)
    }

    @Test("The partial-failure line says what to do, in plain words")
    func partialLine() {
        let line = Account.partlySignedOutMessage
        #expect(line.contains("Sign Out Everywhere"))
        #expect(!line.lowercased().contains("error"))
    }

    /// The action's button only raises the dialog; the call lives inside the
    /// dialog's own destructive button. Checked in the source, which is where
    /// a later edit would move the call out.
    @Test("The Settings action asks first")
    func asksFirst() throws {
        let url = URL(filePath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .appending(path: "Sources/AgentKit/AccountSection.swift")
        let source = try String(contentsOf: url, encoding: .utf8)
        #expect(source.components(separatedBy: "account.signOutEverywhere()").count - 1 == 1)
        let dialog = try #require(source.range(of: ".confirmationDialog("))
        let call = try #require(source.range(of: "account.signOutEverywhere()"))
        #expect(dialog.lowerBound < call.lowerBound)
        #expect(source.contains("Button(\"Sign Out Everywhere…\")"))
        #expect(AccountSection.signOutEverywhereWarning.contains("Paired runners keep working. Check them for any you don’t recognize."))
    }

    /// The call signs this device out through `signOut()`, the one place that
    /// clears the session, and only on the relay's success.
    @Test("This device signs out only after the relay answers")
    func signsOutOnSuccess() throws {
        let url = URL(filePath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .appending(path: "Sources/AgentKit/Account.swift")
        let source = try String(contentsOf: url, encoding: .utf8)
        let start = try #require(source.range(of: "public func signOutEverywhere() async {"))
        let body = source[start.upperBound...]
        let end = try #require(body.range(of: "\n    }\n"))
        let text = String(body[..<end.lowerBound])
        let failure = try #require(text.range(of: "case .failure"))
        let success = try #require(text.range(of: "case .success"))
        let signOut = try #require(text.range(of: "await signOut()"))
        #expect(failure.lowerBound < success.lowerBound)
        #expect(success.lowerBound < signOut.lowerBound)
    }
}
