import AgentKit
import Foundation

/// A page opened from the plan column, as a navigation stack draws it
/// (ov-873): the way back is the crumbs, "Plan › Builder 1 › the ticket" or
/// "Plan › Web client › the ticket" through an initiative, with Back and
/// Forward in the title bar, and no tab strip or close button over it. The
/// plan stays mounted beneath, so Back returns to it where it was.
///
/// Pure, so a test reads the page as the window draws it.
enum PlanStack {
    typealias Crumb = WorkspaceNavigation.Crumb
    typealias Selection = ContentView.Selection

    /// The agent a ticket was opened under, from its card in the plan column.
    struct Origin: Equatable {
        var task: String
        /// "Builder 1".
        var agent: String
        /// What it is doing now, as its card says it ("Reviewing the diff").
        var doing = ""
        /// Its chat, else its ticket's page; nil when it has neither.
        var target: Selection?
    }

    /// What the jump bar and the task view draw for `place`.
    struct Page: Equatable {
        var crumbs: [Crumb]
        /// Whether the crumbs' menus (the sidebar's outline) still fit them: not
        /// once the agent stands in for the initiative.
        var keepsMenus: Bool
        /// Overview, Agent and Changes over a task: not beside the plan, where
        /// the ticket's page shows its agent in place and "Open Chat" goes on.
        var showsTabs: Bool
    }

    /// Overview, Agent and Changes over a task: not beside the plan.
    static func showsTabs(besidePlan: Bool) -> Bool { !besidePlan }

    /// `base` (the path through the outline, or the worktree path) as a
    /// page opened `besidePlan`, or as it was.
    static func page(base: [Crumb], place: Selection, origin: Origin?, besidePlan: Bool) -> Page {
        guard besidePlan, !base.isEmpty else {
            return Page(crumbs: base, keepsMenus: true, showsTabs: showsTabs(besidePlan: besidePlan))
        }
        var crumbs = base
        // The workspace's own crumb is the plan, the page they came from.
        crumbs[0].title = "Plan"
        crumbs[0].glyph = nil
        guard case .workspace(_, _, .task(let id)?) = place else {
            return Page(crumbs: crumbs, keepsMenus: true, showsTabs: showsTabs(besidePlan: besidePlan))
        }
        if let origin, origin.task == id, let ticket = crumbs.last, crumbs.count > 1 {
            let agent = Crumb(title: origin.agent, target: origin.target)
            return Page(
                crumbs: [crumbs[0], agent, Crumb(title: ticket.title, target: nil, glyph: ticket.glyph)],
                keepsMenus: false, showsTabs: showsTabs(besidePlan: besidePlan))
        }
        return Page(crumbs: crumbs, keepsMenus: true, showsTabs: showsTabs(besidePlan: besidePlan))
    }
}
