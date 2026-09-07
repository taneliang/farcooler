import Foundation

// Every theme the phone offers, assembled from every runner rather than
// overwritten by whichever answered last.
//
// The arithmetic behind `Themes`, with no `AppStorage` in it and no phone. Here
// for `FleetMembership.swift`'s reason exactly: the rule has no screen in it,
// and the iOS UI suite is compiled by CI and never executed, so a merge left
// inside a singleton is a merge nothing checks. It runs under
// `swift test --package-path apps/shared/AgentKit`.
//
// **This is the bug the multi-runner port left behind.** `Themes.merge` took
// ONE runner's list and rebuilt the whole catalog from it —
// `builtIn() + thatRunner's themes` — which is correct for an app that holds
// one connection and wrong for one that holds several. With two runners it
// meant the catalog was whatever the runner that polled MOST RECENTLY defines,
// so a theme belonging to the other runner left the list a moment after it
// arrived; and `Themes.current` falls back when the stored name no longer
// resolves, so the visible symptom was the whole app reverting to Nord, at
// random, while nobody touched anything.
//
// Two rules, and both are pinned by `ThemeCatalogTests`:
//
// - **Every runner's themes are in the list at once.** A phone talking to three
//   runners offers all three runners' themes, because the person picking one
//   has three runners and one picker.
// - **The answer does not depend on the order the runners answered in.** That
//   is what "at random" meant: the same fleet, polled in a different order,
//   produced a different catalog. Contributions arrive keyed by runner and are
//   folded in sorted runner order, so the result is a function of the fleet and
//   not of the network's timing.
//
// Generic over the theme type rather than importing one, because there isn't
// one to import: `Theme` is declared in the iOS target and this package cannot
// see it. `Identifiable` is the whole surface the rules need — `Theme.id` is
// its name, and a name is what a collision is about.
enum ThemeCatalog {
    /// The catalog: the built-ins, with every runner's themes folded in.
    ///
    /// The built-ins come first and stay in their own order, because they are
    /// this phone's and do not depend on which runners it happens to be talking
    /// to. A phone on a train still has to render something.
    ///
    /// A host theme whose name matches one already in the list REPLACES it in
    /// place rather than appending — the host's is the one somebody edited a
    /// file on purpose to make, and replacing in place keeps "Nord" where the
    /// eye already found it instead of moving it to the end of the picker.
    ///
    /// Two runners defining the same name is the one case with no right answer,
    /// so what matters is only that the answer is STABLE: runners are folded in
    /// sorted id order, so the last one in that order wins, every time, on every
    /// launch. The alternative the port shipped — whoever answered last — is the
    /// same rule with a coin flip in it.
    static func merged<Theme: Identifiable, RunnerID: Comparable & Hashable>(
        builtIn: [Theme], hostThemes: [RunnerID: [Theme]]
    ) -> [Theme] {
        var merged = builtIn
        for runner in hostThemes.keys.sorted() {
            for theme in hostThemes[runner] ?? [] {
                if let index = merged.firstIndex(where: { $0.id == theme.id }) {
                    merged[index] = theme
                } else {
                    merged.append(theme)
                }
            }
        }
        return merged
    }

    /// Which theme is in force, given the name that was stored.
    ///
    /// Falls back rather than to nothing when a stored name no longer resolves:
    /// a theme that vanished because a host's config file moved should cost you
    /// your colors, not your terminal.
    ///
    /// Separated from the catalog so that both halves of the reverting-to-Nord
    /// bug are readable: this function is the part that was always right, and
    /// keeping it here is what makes a test able to say "the stored name still
    /// resolves after the second runner answers" in one assertion.
    static func inForce<Theme: Identifiable>(
        named name: Theme.ID, in catalog: [Theme], fallback: Theme
    ) -> Theme {
        catalog.first { $0.id == name } ?? fallback
    }
}
