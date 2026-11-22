import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

struct TaskNameTests {
    // MARK: - What a failed start says

    @Test func aFailedStartIsSaidInThisAppsWordsNeverTheRunners() {
        let conflict = TaskFailure.sentence(for: "error: resource version is stale\ncode: resource-conflict")
        #expect(!conflict.contains("stale") && !conflict.contains("error"), "\(conflict)")
        #expect(conflict.hasPrefix("Couldn’t start the agent"))
        // No word at all is a link that dropped; a word nothing here knows is
        // a runner that answered, and is not said to be unreachable.
        let generic = TaskFailure.sentence(for: nil)
        #expect(generic.contains("Check that it’s reachable"), "\(generic)")
        let newer = TaskFailure.sentence(for: "error: x\ncode: some-word-from-a-newer-runner")
        #expect(!newer.contains("reachable"), "\(newer)")

        let taken = TaskFailure.sentence(for: "error: branch already exists\ncode: branch-exists")
        #expect(taken.contains("already has a branch or folder"), "\(taken)")
        #expect(TaskFailure.sentence(for: "error: worktree path already exists\ncode: worktree-exists") == taken)
        #expect(TaskFailure.sentence(for: "error: tmux is unavailable\ncode: tmux-unavailable").contains("tmux"))
        // A runner that answered slowly has tmux; telling it to install tmux is wrong.
        let slow = TaskFailure.sentence(for: "error: tmux did not answer in time\ncode: tmux-timed-out")
        #expect(slow.contains("took too long") && !slow.contains("install"), "\(slow)")
        // A device enrolled to look is not a runner that couldn't be reached.
        let looked = TaskFailure.sentence(for: "error: no\ncode: scope-denied")
        #expect(looked.contains(RunnerRefusal.scopeDenied.sentence) && !looked.contains("reachable"), "\(looked)")
    }

    /// The code word decides, not the prose: a reworded message still maps,
    /// and the old prose without a word does not.
    @Test func aFailureIsRecognizedByItsCodeWordNotItsWording() {
        let taken = TaskFailure.sentence(for: "error: a branch by that name is here already\ncode: branch-exists")
        #expect(taken.contains("already has a branch or folder"), "\(taken)")
        let prose = TaskFailure.sentence(for: "error: branch already exists")
        #expect(!prose.contains("already has a branch or folder"), "\(prose)")
    }
}
