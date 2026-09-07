import Foundation

/// What a workspace and a tab are CALLED, once the shell holds more than one
/// runner's fleet at a time.
///
/// # Why the runner is in the id
///
/// **Not because ids collide today.** That was the reason recorded when this
/// port was scoped, and it is wrong about this app: `Workspace.id` and
/// `Terminal.id` decode `uuid_of(&w.id).to_string()` — the FULL UUIDv7 the
/// daemon minted (`crates/client/src/session.rs`) — and the eight hex
/// characters are the separate `short` field, which nothing in these apps ever
/// uses as an identity. Two daemons minting UUIDv7s share a timestamp head and
/// differ in 62 random bits, so a genuine collision is not a thing to design
/// against. The "eight hex characters" that the port's scoping documents name
/// are `ids::short` in `crates/protocol` — a display form for logs and CLI
/// output, which is where that sentence is true and the only place it is.
///
/// The reason it is here anyway is that a merged fleet has to get from a tab
/// back to a **connection**, and the runner is the only part of that answer.
/// The alternative is deriving it — searching every connected runner for a
/// workspace with this id and taking the first match — which is a lookup that
/// silently picks one when the assumption it rests on fails, and makes
/// cross-daemon uniqueness load-bearing for a SwiftUI identity with nothing
/// anywhere enforcing it. `ShellPaneTrack` retains a mounted pane per tab id
/// and `ShellOverview` gives each card `.id(_:)` off the workspace id: two
/// entries sharing one identity resolve by drawing one of them, with no error.
/// Carrying the runner means that outcome does not depend on a property of
/// somebody else's random number generator.
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
    ///
    /// The runner leads, so the composite sorts and reads by machine.
    public static func workspace(runner: String, workspace: String) -> String {
        "\(runner)\(separator)\(workspace)"
    }

    /// One tab, named across the whole fleet.
    ///
    /// The workspace is in it for a reason that predates any of this and is a
    /// real collision rather than a theoretical one: `Pane.changes` has ONE
    /// pane id for the whole app — it is the only pane with no object on the
    /// runner behind it — so forty workspaces each with a Changes tab would
    /// otherwise be forty tabs sharing one SwiftUI identity, which resolves by
    /// drawing one of them. The runner is in it for the resolution above.
    public static func tab(runner: String, workspace: String, pane: String) -> String {
        "\(self.workspace(runner: runner, workspace: workspace))\(separator)\(pane)"
    }
}
