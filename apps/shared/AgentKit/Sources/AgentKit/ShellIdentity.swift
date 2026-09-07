import Foundation

/// What a workspace and a tab are CALLED, once the shell holds more than one
/// runner's fleet at a time.
///
/// # Why this is arithmetic rather than string handling
///
/// A workspace id is the last eight hex characters of a UUID minted **per
/// daemon** (`FleetEntry`, `crates/daemon`), so it is unique on its runner and
/// says nothing at all about which runner that is. One connection made that
/// harmless: every id the app held came from the same daemon. N connections
/// make it a collision — two runners can hand back the same eight characters
/// for two unrelated worktrees, and nothing anywhere reports it.
///
/// What a collision costs is not a cosmetic mix-up. These strings are SwiftUI
/// identities and dictionary keys: `ShellPaneTrack` retains a mounted pane per
/// tab id, `ShellOverview` gives each card `.id(_:)` and an accessibility
/// identifier, and `ShellFleetMap.refs` maps a tab id back to the pane it
/// draws. Two tabs sharing an identity resolve by drawing one of them — so the
/// symptom is a pane on the wrong runner, arrived at silently.
///
/// # Why it lives here
///
/// The shell's ids were composed inside `ShellScreen.tabID`, in the iOS target,
/// whose tests CI compiles and never runs (`.github/workflows/ci.yml`). This
/// package is what `swift test --package-path apps/shared/AgentKit` executes,
/// which is the difference between a rule and a rule somebody believes. The
/// same move `ShellNavigation`, `FleetMembership` and `PaneDrafts` were made
/// for.
///
/// # Composition only, and deliberately no decoder
///
/// Nothing here parses an id back apart. `ShellPaneRef` is a side table keyed
/// by the id, which is what `ShellPaneRef`'s own header argues for: packing
/// fields into a string and reading them back out is a decoder standing between
/// the fleet and the screen, and nobody writes tests for one. These functions
/// exist so that the packing is done once, the same way, everywhere.
public enum ShellIdentity {
    /// The separator, spelled once.
    ///
    /// A slash, matching what the cache next door already writes —
    /// `RunnerDirectory.group()` has built `"\(runner)/\(workspace)/\(index)"`
    /// since the grid first listed other runners. Two spellings of the same
    /// composite would be two answers to "is this the same tab".
    private static let separator = "/"

    /// One workspace, named across the whole fleet.
    ///
    /// The same string `FleetEntry.id` is, and that is on purpose rather than a
    /// coincidence to be relied on quietly: the store's merged list and the
    /// shell's fleet have to agree on what one workspace is called, or a card
    /// tapped in the overview and the pane it opens are two different lookups.
    public static func workspace(runner: String, workspace: String) -> String {
        "\(runner)\(separator)\(workspace)"
    }

    /// One tab, named across the whole fleet.
    ///
    /// The workspace is in it for a reason that predates any of this and has
    /// not changed: `Pane.changes` has one pane id for the whole app — it is
    /// the only pane with no object on the runner behind it — so forty
    /// workspaces each with a Changes tab would otherwise be forty tabs sharing
    /// one identity. The RUNNER is in it for the collision above. Terminal ids
    /// are already unique per runner and gain nothing from either prefix except
    /// being legible in a probe.
    public static func tab(runner: String, workspace: String, pane: String) -> String {
        "\(self.workspace(runner: runner, workspace: workspace))\(separator)\(pane)"
    }
}
