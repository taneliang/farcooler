import Foundation
import Testing

@testable import AgentKit

/// What a phone says before it closes a terminal.
///
/// This suite exists for the reason `ShellNavigationTests` does — the iOS app
/// generates only a UI test target, so a sentence assembled in a `View.body` is
/// a sentence nothing reads back — and for one of its own: closing is the only
/// irreversible thing either phone can do to a pane. The pane is killed and the
/// record is deleted, `remain-on-exit` takes the dead rectangle with it, and
/// there is no bin. The sentence in front of that is the last thing anybody
/// gets to read, and a duration that says `0s`, an agent named as `shell`
/// because the preset was empty, or a claim that something is running when it
/// exited an hour ago are all defects that look perfectly normal on a
/// screenshot.
///
/// The terminals are DECODED rather than constructed, the way `FleetDecodeTests`
/// does it: `Terminal` has forty fields and a memberwise call listing them is a
/// fixture nobody can read, while the JSON below is the shape the daemon
/// actually sends.
///
/// Android's `ShellCloseTest` asserts the same table against the same words.
/// The two phones may differ in GESTURE — a swipe here, a menu there — and must
/// not differ in what they tell somebody they are about to lose.
struct ShellCloseTests {
    /// Unix milliseconds `seconds` before `now`.
    private static func ago(_ seconds: Double, from now: Date) -> Double {
        (now.timeIntervalSince1970 - seconds) * 1000
    }

    private static func terminal(
        state: String, preset: String = "claude", title: String = "",
        activity: String? = nil, activitySince: Double? = nil, turnStartedAt: Double? = nil
    ) throws -> Terminal {
        var body: [String: Any] = [
            "id": "t1", "short": "t1", "title": title, "preset": preset, "state": state,
            // Required on the wire, and required here: `epoch` is the one field
            // on `Terminal` that is not optional beyond the first five, so a
            // fixture without it fails the decode rather than the assertion.
            "epoch": 1,
        ]
        if let activity { body["activity"] = activity }
        if let activitySince { body["activitySince"] = activitySince }
        if let turnStartedAt { body["turnStartedAt"] = turnStartedAt }
        let data = try JSONSerialization.data(withJSONObject: body)
        return try JSONDecoder().decode(Terminal.self, from: data)
    }

    private static let now = Date(timeIntervalSince1970: 1_757_000_000)

    /// **A stopped pane is closed without a word.**
    ///
    /// Nil is not "no opinion" — it is the answer that means close it now, and
    /// `ShellScreen.close(tab:in:)` reads it that way. Every state the daemon
    /// will REMOVE without complaint is here, because the confirmation exists
    /// to cover the stop that has to happen first, and a pane with nothing to
    /// stop is a pane with nothing to ask about. A sheet in front of every
    /// close would be a tax charged on the harmless case to protect the rare
    /// one, which is the trade the ruling refused.
    @Test func aPaneWithNothingRunningIsNotWorthAsking() throws {
        for state in ["exited", "error", "lost", "something-new"] {
            let terminal = try Self.terminal(state: state)
            #expect(ShellClose.mustAsk(about: terminal) == false, "\(state) asked")
            #expect(ShellClose.question(about: terminal, at: Self.now) == nil, "\(state) asked")
        }
    }

    /// **The two states the daemon refuses to remove are the two that ask.**
    ///
    /// `Service::remove_terminal` answers `RunningProcesses` for `Running` and
    /// `Starting`, which is what makes the stop mandatory — so those are
    /// exactly the panes where closing interrupts something. The pair is not a
    /// coincidence to be restated in two places; it is one fact, and this is
    /// the assertion that keeps the phone's copy of it in step with the
    /// daemon's.
    @Test func aLivePaneAlwaysAsks() throws {
        for state in ["running", "starting"] {
            #expect(ShellClose.mustAsk(about: try Self.terminal(state: state)), "\(state)")
        }
    }

    /// **The sheet names the pane in its title and the agent in its body.**
    ///
    /// Two different names on purpose. The title is `Terminal.label` — the
    /// conversation's own name where the agent has given it one — because that
    /// is the string on the row that was swiped, and a confirmation that
    /// renamed the thing it is about would be asking about something else. The
    /// body is the COMMAND, which is what is actually going to be killed.
    @Test func aRunningAgentIsNamedTwiceAndTimedOnce() throws {
        let terminal = try Self.terminal(
            state: "running", preset: "claude", title: "Rewrite the parser",
            activity: "working", activitySince: Self.ago(20, from: Self.now),
            turnStartedAt: Self.ago(754, from: Self.now))
        let question = try #require(ShellClose.question(about: terminal, at: Self.now))
        #expect(question.title == "Close “Rewrite the parser”?")
        #expect(
            question.message == "It’s running claude, which has been working for 12m. "
                + "Closing stops it and removes the tab. There’s no undo.")
    }

    /// **A working agent is timed by its TURN and a blocked one by its STATE.**
    ///
    /// The two clocks answer different questions and conflating them is the bug
    /// `Terminal.displayDuration(at:)` exists to fix: `working` is only ever
    /// mid-turn, so the turn clock is the honest answer to "how long has this
    /// been going", while a prompt held for twenty minutes is the thing to
    /// notice about a blocked one rather than how long the turn around it has
    /// run. Both fixtures carry BOTH timestamps, and they carry different
    /// values — a version that read one clock for both would answer 12m twice.
    @Test func aBlockedAgentIsTimedByHowLongItHasBeenWaiting() throws {
        let terminal = try Self.terminal(
            state: "running", preset: "codex", title: "",
            activity: "blocked", activitySince: Self.ago(1300, from: Self.now),
            turnStartedAt: Self.ago(9000, from: Self.now))
        let question = try #require(ShellClose.question(about: terminal, at: Self.now))
        #expect(question.title == "Close “codex”?")
        #expect(
            question.message == "It’s running codex, which has been waiting on you for 21m. "
                + "Closing stops it and removes the tab. There’s no undo.")
    }

    /// **An agent with no clock loses the clause, not the sentence.**
    ///
    /// Three ways to get here and all of them are ordinary: a plain shell,
    /// which has no agent and therefore no activity at all; an idle or finished
    /// agent, whose age is noise — "idle for three days" is not a reason to
    /// keep a pane; and a runner too old to send a timestamp, where nil means
    /// "nobody said" and must never be rendered as "just now".
    ///
    /// What it must not do is say `0s`. A duration under five seconds is nil
    /// from `Terminal.displayDuration(at:)` for that reason, and this asserts
    /// the sentence survives it rather than growing a hole.
    @Test func aPaneWithNoClockStillGetsASentence() throws {
        let shell = try Self.terminal(state: "running", preset: "zsh")
        #expect(
            try #require(ShellClose.question(about: shell, at: Self.now)).message
                == "It’s running shell. Closing stops it and removes the tab. There’s no undo.")

        let idle = try Self.terminal(
            state: "running", preset: "claude", activity: "idle",
            activitySince: Self.ago(90000, from: Self.now))
        #expect(
            try #require(ShellClose.question(about: idle, at: Self.now)).message
                == "It’s running claude. Closing stops it and removes the tab. There’s no undo.")

        let justStarted = try Self.terminal(
            state: "running", preset: "claude", activity: "working",
            turnStartedAt: Self.ago(2, from: Self.now))
        #expect(
            try #require(ShellClose.question(about: justStarted, at: Self.now)).message
                == "It’s running claude. Closing stops it and removes the tab. There’s no undo.")
    }

    /// **The button says the noun, in title case, and never "Delete".**
    ///
    /// A dialog raised by a swipe has already lost the row it came from — the
    /// content slides out from under the action — so a bare verb is a verb with
    /// nothing attached to it. And the word is Close rather than Delete because
    /// that is what the product calls it everywhere else: the Mac's menu item
    /// is `Close Terminal`, and a phone that called the same act deleting would
    /// be a third vocabulary for one thing.
    @Test func theConfirmButtonNamesWhatItCloses() {
        #expect(ShellClose.confirm == "Close Terminal")
    }
}
