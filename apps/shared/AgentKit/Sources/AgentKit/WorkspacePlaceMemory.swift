import Foundation

// Where each workspace was left, on the phone and the iPad (ov-442).
//
// `PhoneLaunch.stackKey` keeps the one stack the phone is on, for a relaunch.
// Opening a workspace from Needs You replaced it with the workspace alone, so
// going out and back in landed on the workspace's first screen however deep
// you had been. This keeps, per workspace, the screens that were pushed over
// it and the iPad's canvas, and gives them back when the workspace is opened.

enum WorkspacePlaceMemory {
    /// The screens pushed over `place`, as `PhoneLaunch.encode` writes them.
    static func stackKey(_ place: PhoneWorkspace) -> String {
        "workspace.place.\(place.runner).\(place.workspace)"
    }

    /// What the iPad's plan column showed in `place`, as JSON.
    static func canvasKey(_ place: PhoneWorkspace) -> String {
        "workspace.canvas.\(place.runner).\(place.workspace)"
    }

    /// The screens a route is about: its workspace, or nil for a worktree,
    /// which covers the stack and is never kept as a place to come back to.
    private static func workspace(of route: PhoneRoute) -> PhoneWorkspace? {
        switch route {
        case .workspace(let place), .task(let place, _), .history(let place, _), .plan(let place, _),
            .tree(let place, _):
            return place
        case .worktree:
            return nil
        }
    }

    /// Keep what's pushed over a workspace, when `stack` is on one. A stack
    /// that isn't (Needs You, or a worktree alone) keeps nothing and changes
    /// nothing: going back to Needs You is leaving a place, not clearing it.
    static func keep(_ stack: [PhoneRoute], in defaults: UserDefaults = .standard) {
        guard case .workspace(let place)? = stack.first else { return }
        let over = stack.dropFirst().prefix { workspace(of: $0) == place }
        defaults.set(PhoneLaunch.encode(Array(over)), forKey: stackKey(place))
    }

    /// The stack that opens `place`: the workspace, then what was pushed over
    /// it that `exists` still. A screen that's gone ends the stack there.
    static func opening(
        _ place: PhoneWorkspace, in defaults: UserDefaults = .standard, exists: (PhoneRoute) -> Bool = { _ in true }
    ) -> [PhoneRoute] {
        let kept = PhoneLaunch.decode(defaults.data(forKey: stackKey(place)))
        let over = kept.prefix { workspace(of: $0) == place }.prefix(while: exists)
        return [.workspace(place)] + over
    }

    static func keep(_ canvas: PadCanvas, for place: PhoneWorkspace, in defaults: UserDefaults = .standard) {
        defaults.set(try? JSONEncoder().encode(canvas), forKey: canvasKey(place))
    }

    /// The canvas last shown in `place`, or the plan.
    static func canvas(for place: PhoneWorkspace, in defaults: UserDefaults = .standard) -> PadCanvas {
        defaults.data(forKey: canvasKey(place)).flatMap { try? JSONDecoder().decode(PadCanvas.self, from: $0) }
            ?? .plan
    }
}
