import AgentKit
import SwiftUI

/// What the navigator draws while its filter (⌘F) narrows it (ov-177): only
/// what matches. A section with no match is left out rather than drawn as a
/// header reading 0, Unread is left out rather than saying "You're all
/// caught up." about a list it only narrowed, and with no match anywhere the
/// navigator says so once (`NavigatorNoResults`).
///
/// Unfiltered, everything shows, as before: every status's header, empty or
/// not, and Unread with its sentence when it has nothing.
struct NavigatorFiltering {
    /// Whether a filter is narrowing the navigator at all.
    let filtering: Bool
    /// The orchestrator's row: always, unfiltered; filtered, when it's
    /// what was typed ("orch"), or its agent is.
    let showsOrchestrator: Bool
    /// The Unread strip: always, unfiltered; filtered, when anything in it
    /// matches.
    let showsUnread: Bool
    /// The task statuses drawn: every one, unfiltered; filtered, the ones
    /// with a match.
    let sections: [TaskBoardColumn]
    /// The Worktrees section's, filtered: the matching ones, without the
    /// hidden ones or New Worktree….
    let worktrees: BoardWorktrees

    /// Whether the Tasks section is drawn: unfiltered, always.
    func showsTasks(unreadable: Bool) -> Bool {
        !filtering || showsUnread || !sections.isEmpty || unreadable
    }

    /// Whether the Worktrees section is drawn.
    var showsWorktrees: Bool { filtering ? !worktrees.shown.isEmpty : !worktrees.isEmpty }

    /// Nothing anywhere matches: the one "No Results" in place of every
    /// section.
    func isEmpty(unreadable: Bool) -> Bool {
        filtering && !showsOrchestrator && !showsTasks(unreadable: unreadable) && !showsWorktrees
    }

    /// - Parameters:
    ///   - board: the board already narrowed by `filter` (`BoardFilter.narrowed`).
    ///   - orchestrator: the orchestrator's agent, or nil with no row for it.
    ///   - unreadMatches: whether Unread, narrowed by the filter, lists anything.
    static func make(
        filter: String, board: TaskBoardModel, hasOrchestrator: Bool, agent: String?, unreadMatches: Bool,
        worktrees: BoardWorktrees
    ) -> NavigatorFiltering {
        guard !BoardFilter.isEmpty(filter) else {
            return NavigatorFiltering(
                filtering: false, showsOrchestrator: hasOrchestrator, showsUnread: true, sections: board.sections,
                worktrees: worktrees)
        }
        var narrowed = worktrees
        narrowed.shown = worktrees.shown.filter { BoardFilter.matches(key: $0.task, title: $0.branch, filter) }
        narrowed.hidden = []
        narrowed.onNew = nil
        return NavigatorFiltering(
            filtering: true,
            showsOrchestrator: hasOrchestrator
                && BoardFilter.matches(key: "Orchestrator", title: agent ?? "", filter),
            showsUnread: unreadMatches,
            sections: board.sections.filter { !$0.rows.isEmpty },
            worktrees: narrowed)
    }
}

/// Nothing in the navigator matches the filter: the system's own search
/// empty state, "No Results for "testso"" and its line under it, as Mail
/// and Finder say it, once for the whole navigator.
struct NavigatorNoResults: View {
    let filter: String

    var body: some View {
        ContentUnavailableView.search(text: filter.trimmingCharacters(in: .whitespaces))
            .frame(maxWidth: .infinity)
            .padding(.top, 2 * ColumnGrid.rhythm)
            .accessibilityIdentifier("navigator-no-results")
    }
}

/// How the navigator's list changes as the filter does (ov-177): at once.
/// Typing narrows the list in place, rows, sections and the selection's
/// highlight all jumping to where they now are; the shared spring
/// (`BoardMotion`) is for what really changed on the board: a task arriving,
/// leaving or moving, a section opening or closing.
enum NavigatorFilterMotion {
    /// The transaction a filter change runs in: no animation, and none that
    /// a `.animation(_:value:)` inside the list may add (`disablesAnimations`).
    static func apply(_ transaction: inout Transaction) {
        transaction.animation = nil
        transaction.disablesAnimations = true
    }
}

extension View {
    /// This list, unanimated whenever `filter` changes (`NavigatorFilterMotion`).
    func unanimatedWhenFiltering(_ filter: String) -> some View {
        transaction(value: filter, NavigatorFilterMotion.apply)
    }
}
