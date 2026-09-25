import Foundation
import Testing

@testable import Far_Cooler

/// Naming a task from its description: the heuristic that always answers, and
/// the deadline that keeps a model from ever holding it up.
struct TaskNameTests {
    @Test func aDescriptionLosesItsFillerAndKeepsItsMeaning() {
        #expect(
            TaskName.heuristic(
                "Please fix the flaky reconnect test in the iOS app when the runner restarts")
                == "fix-flaky-reconnect-test")
        #expect(TaskName.heuristic("Can you add a dark mode toggle") == "add-dark-mode-toggle")
        #expect(TaskName.heuristic("I want to bump Sparkle") == "bump-sparkle")
        #expect(
            TaskName.heuristic(
                "can you look into why codex sometimes gets stuck if there was a prompt to update version")
                == "codex-stuck-prompt")
        #expect(
            TaskName.heuristic("the cmd+n panel is able to create workspaces, but the names are long")
                == "cmd-n-panel-create")
    }

    @Test func aNameIsNeverLongerThanTheBudget() {
        for description in [
            "Investigate memory leak in the terminal renderer after two hours of use",
            "Supercalifragilisticexpialidocious-internationalization refactor",
            String(repeating: "word ", count: 40),
            "the cmd+n panel is able to create workspaces, but the workspace names are very long",
        ] {
            let name = TaskName.heuristic(description)
            #expect(name.count <= TaskName.maxLength, "\(name)")
            #expect(!name.isEmpty)
            #expect(WorktreeName.isValid(name), "\(name)")
            #expect(!name.hasSuffix("-") && !name.hasPrefix("-"), "\(name)")
        }
    }

    @Test func aDescriptionOfNothingButFillerStillHasAName() {
        #expect(TaskName.heuristic("can you do this for me please") == "can-you-do-this")
        #expect(TaskName.heuristic("日本語のテスト") == "workspace")
        #expect(TaskName.heuristic("naïve café") == "naive-cafe")
        // An apostrophe joins a word rather than splitting it.
        #expect(TaskName.heuristic("Don't break the build") == "dont-break-build")
        #expect(TaskName.heuristic("it’s broken again") == "broken")
    }

    @Test func aModelsDecoratedAnswerIsReadAsWords() {
        #expect(TaskName.fromModel("fix - reconnect - test") == "fix-reconnect-test")
        #expect(TaskName.fromModel("Diff View - Load - Huge Files.") == "diff-view-load-huge")
        #expect(TaskName.fromModel("tmux-window-retry\nThis names the task.") == "tmux-window-retry")
        #expect(TaskName.fromModel("   ") == nil)
        #expect(TaskName.fromModel("Here is the name: fix-bug") == "fix-bug")
        #expect(TaskName.fromModel("the settings toggle") == "settings-toggle")
        #expect(
            TaskName.fromModel("I would name this task something about fixing the flaky test for you")
                == nil, "a sentence is not a name")
    }

    @Test func aTakenNameGetsANumberAndStaysInBudget() {
        #expect(TaskName.unique("fix-it", taken: []) == "fix-it")
        #expect(TaskName.unique("fix-it", taken: ["fix-it"]) == "fix-it-2")
        #expect(TaskName.unique("fix-it", taken: ["fix-it", "fix-it-2"]) == "fix-it-3")
        #expect(TaskName.unique("fix-it") { ["fix-it", "fix-it-2"].contains($0) } == "fix-it-3")
        let long = "memory-leak-renderer-abc"  // 24
        let next = TaskName.unique(long, taken: [long])
        #expect(next.count <= TaskName.maxLength && next.hasSuffix("-2"), "\(next)")
    }

    // MARK: - The model, and its deadline

    @Test func aModelsAnswerIsUsedWhenItComesInTime() async {
        let namer = TaskNamer(model: { _ in "Reconnect - Flake" }, timeout: .seconds(2))
        #expect(await namer.name(for: "Please fix the flaky reconnect test") == "reconnect-flake")
    }

    @Test func aModelThatIgnoresTheDeadlineIsAbandonedAtIt() async {
        // Sleeps without checking for cancellation, the way a stuck model call
        // would: only a race that does not wait for it can return on time.
        let namer = TaskNamer(
            model: { _ in
                let until = ContinuousClock.now + .seconds(3)
                while ContinuousClock.now < until { usleep(10_000) }
                return "too-late"
            },
            timeout: .milliseconds(200))
        let started = ContinuousClock.now
        let name = await namer.name(for: "Please fix the flaky reconnect test")
        let took = ContinuousClock.now - started
        #expect(name == "fix-flaky-reconnect-test", "the heuristic, not the late answer")
        #expect(took < .seconds(1), "returned at the deadline: \(took)")
    }

    @Test func aModelThatFailsOrRamblesFallsBackToTheHeuristic() async {
        struct Refused: Error {}
        let failing = TaskNamer(model: { _ in throw Refused() }, timeout: .seconds(1))
        #expect(await failing.name(for: "add a dark mode toggle") == "add-dark-mode-toggle")
        let rambling = TaskNamer(
            model: { _ in "Sure! Here is a short name that describes the task you have described" },
            timeout: .seconds(1))
        #expect(await rambling.name(for: "add a dark mode toggle") == "add-dark-mode-toggle")
        #expect(await TaskNamer(model: nil).name(for: "add a dark mode toggle") == "add-dark-mode-toggle")
    }

    // MARK: - What a failed start says

    @Test func aFailedStartIsSaidInThisAppsWordsNeverTheRunners() {
        let generic = TaskFailure.sentence(for: "error: resource version is stale\ncode: resource-conflict")
        #expect(!generic.contains("stale") && !generic.contains("error"), "\(generic)")
        #expect(generic.hasPrefix("Couldn’t start the agent"))
        #expect(TaskFailure.sentence(for: nil) == generic)

        let taken = TaskFailure.sentence(for: "error: branch already exists\ncode: branch-exists")
        #expect(taken.contains("already has a branch or folder"), "\(taken)")
        #expect(TaskFailure.sentence(for: "error: worktree path already exists\ncode: worktree-exists") == taken)
        #expect(TaskFailure.sentence(for: "error: tmux is unavailable\ncode: tmux-unavailable").contains("tmux"))
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
