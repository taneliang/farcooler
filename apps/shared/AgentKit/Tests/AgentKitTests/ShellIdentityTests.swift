import Foundation
import Testing

@testable import AgentKit

/// What the shell's ids have to say, now that it holds several runners' fleets.
///
/// Every one of these fails against the composition the shell used before the
/// port — `"\(workspace)/\(pane.id)"`, with no runner in it — which is the
/// whole reason they are here.
///
/// **The workspace ids below are written short for readability, and that is a
/// fixture choice rather than the wire's shape.** The app decodes the daemon's
/// full UUIDv7; the eight-character form is `Workspace.short`, which nothing
/// uses as an identity. See `ShellIdentity`, which corrects the premise this
/// port was scoped on. What these pin is not that a collision happens — it is
/// that a tab id names the runner, so resolving one back to a connection is a
/// lookup rather than a search across every runner for a matching id.
struct ShellIdentityTests {
    /// One workspace id, two runners, two different worktrees.
    ///
    /// The mutation this exists for is deleting the runner from
    /// `ShellIdentity.workspace`: it leaves every single-runner screen working
    /// and leaves the merged fleet with two cards under one identity.
    @Test func twoRunnersSharingAWorkspaceIDAreStillTwoWorkspaces() {
        let one = ShellIdentity.workspace(runner: "RUNNER-A", workspace: "3f9a1c07")
        let two = ShellIdentity.workspace(runner: "RUNNER-B", workspace: "3f9a1c07")
        #expect(one != two)
    }

    /// The same, one level down, where it would cost a mounted pane rather than
    /// a card: `ShellPaneTrack` retains by tab id.
    @Test func twoRunnersSharingAWorkspaceIDAreStillTwoTabs() {
        let one = ShellIdentity.tab(runner: "RUNNER-A", workspace: "3f9a1c07", pane: "changes")
        let two = ShellIdentity.tab(runner: "RUNNER-B", workspace: "3f9a1c07", pane: "changes")
        #expect(one != two)
    }

    /// The rule that predates the port and must survive it: the Changes pane
    /// has ONE pane id for the whole app, so the workspace has to be in the tab
    /// id or every workspace's diff is the same tab.
    @Test func twoWorkspacesOnOneRunnerHaveDifferentChangesTabs() {
        let one = ShellIdentity.tab(runner: "RUNNER-A", workspace: "3f9a1c07", pane: "changes")
        let two = ShellIdentity.tab(runner: "RUNNER-A", workspace: "b21e4d55", pane: "changes")
        #expect(one != two)
    }

    /// Two panes of one workspace are two tabs. The case that was never broken,
    /// pinned so a composition that dropped the PANE instead would be caught by
    /// the same file.
    @Test func twoPanesOfOneWorkspaceAreTwoTabs() {
        let diff = ShellIdentity.tab(runner: "RUNNER-A", workspace: "3f9a1c07", pane: "changes")
        let agent = ShellIdentity.tab(runner: "RUNNER-A", workspace: "3f9a1c07", pane: "t-77")
        #expect(diff != agent)
    }

    /// A tab id is its workspace's id plus the pane, so a screen holding one
    /// and a screen holding the other are talking about the same worktree.
    ///
    /// Not a decoder — nothing parses these apart, and `ShellIdentity`'s header
    /// says why. What this pins is that the two functions compose the same
    /// prefix, which is what lets `ShellFleetMap` key one side table by
    /// workspace and another by tab without a second spelling of "the same
    /// runner".
    @Test func aTabIDIsBuiltOnItsWorkspaceID() {
        let workspace = ShellIdentity.workspace(runner: "RUNNER-A", workspace: "3f9a1c07")
        let tab = ShellIdentity.tab(runner: "RUNNER-A", workspace: "3f9a1c07", pane: "t-77")
        #expect(tab.hasPrefix(workspace))
    }

    /// The cache next door already spells a remembered tab
    /// `"\(runner)/\(workspace)/\(index)"` — `RunnerDirectory.group()` — and
    /// has since before any of this. A live tab and a cached one that named the
    /// same worktree differently would be two answers to "is this the same
    /// thing", which is the drift the composition is centralized to prevent.
    @Test func theLiveSpellingMatchesTheCacheTheGridAlreadyWrites() {
        #expect(
            ShellIdentity.tab(runner: "RUNNER-A", workspace: "3f9a1c07", pane: "0")
                == "RUNNER-A/3f9a1c07/0")
    }
}
