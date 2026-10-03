import Foundation
import Testing

@testable import AgentKit

/// The page a terminal with no running pane opens to (ov-191).
struct LostPaneTests {
    /// Both spellings the runner's word arrives in: the Mac's CLI writes
    /// `LOST`, the phone's core `lost`. A page keyed on one of them would be
    /// the inert row this card was about, on the other device.
    @Test func bothSpellingsOfLostAreLost() {
        #expect(LostPane.Kind(state: "LOST") == .lost)
        #expect(LostPane.Kind(state: "lost") == .lost)
        #expect(LostPane.Kind(state: "exited") == .exited)
        #expect(LostPane.Kind(state: "ERROR") == .error)
    }

    /// A pane that's running, starting, or can't be read right now gets the
    /// terminal, not this page.
    @Test func aLiveOrUnreadPaneIsNotThisPage() {
        for state in ["running", "starting", "unknown", ""] {
            #expect(LostPane.Kind(state: state) == nil, "\(state)")
        }
    }

    /// Restart and Dismiss for a lost terminal; Restart alone otherwise,
    /// because `terminal.dismiss_lost` refuses anything that isn't lost.
    @Test func dismissIsOfferedOnlyWhereTheRunnerTakesIt() {
        #expect(LostPane.actions(for: .lost) == [.restart, .dismiss])
        #expect(LostPane.actions(for: .exited) == [.restart])
        #expect(LostPane.actions(for: .error) == [.restart])
    }

    /// The why, in words, naming every way a pane goes missing: the runner
    /// can't tell them apart.
    @Test func lostSaysWhy() {
        let why = LostPane.explanation(for: .lost)
        #expect(why.contains("closed outside Far Cooler"))
        #expect(why.contains("tmux was quit"))
        #expect(why.contains("the runner restarted"))
    }

    /// **Restart without a recorded command.** A shell's preset is `shell`,
    /// and what was typed into it was never recorded, so Restart says before
    /// it's pressed that it brings back a bare shell.
    @Test func aShellSaysItsCommandWasNotRecorded() {
        for preset in ["shell", ""] {
            let note = LostPane.restartNote(preset: preset)
            #expect(note.hasPrefix("Restart opens a new shell"), "\(preset)")
            #expect(note.contains("wasn’t recorded"), "\(preset)")
        }
    }

    /// **Restart with a recorded command.** Any other preset is run again,
    /// and the note names it, model or not.
    @Test func aRecordedPresetIsNamed() {
        #expect(LostPane.restartNote(preset: "claude:opus").contains("Claude Code"))
        #expect(LostPane.restartNote(preset: "codex").contains("Codex"))
        #expect(LostPane.restartNote(preset: "sleepnomore") == "Restart runs sleepnomore again in this worktree.")
        for preset in ["claude", "codex", "cursor", "sleepnomore"] {
            #expect(!LostPane.restartNote(preset: preset).contains("wasn’t recorded"), "\(preset)")
        }
    }

    /// Title case and contractions, as every other title in the app.
    @Test func titlesAreTitleCase() {
        #expect(LostPane.title(for: .lost) == "Terminal Lost")
        #expect(LostPane.title(for: .error) == "Terminal Didn’t Start")
    }
}
