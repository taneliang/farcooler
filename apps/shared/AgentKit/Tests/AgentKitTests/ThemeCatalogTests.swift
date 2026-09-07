import Foundation
import Testing

@testable import AgentKit

/// A theme shaped like the app's, with only the field a catalog is about.
///
/// `id` is the name, exactly as `Theme.id` is on the phone, because a name is
/// what a collision between two runners is. `background` stands in for the rest
/// of the colors: it is what makes "the same name from a different runner" a
/// value a test can tell apart.
private struct Theme: Identifiable, Equatable {
    var id: String
    var background: UInt32 = 0
}

struct ThemeCatalogTests {
    private let builtIn = [
        Theme(id: "Nord", background: 1), Theme(id: "Solarized", background: 2),
    ]

    // MARK: - The catalog

    /// A phone with no runner still has a picker. This is the train.
    @Test func noRunnerLeavesTheBuiltInsAlone() {
        let merged = ThemeCatalog.merged(builtIn: builtIn, hostThemes: [String: [Theme]]())
        #expect(merged.map(\.id) == ["Nord", "Solarized"])
    }

    /// One runner's own theme is appended, and the built-ins are still there —
    /// the additive rule `Themes.merge` always had.
    @Test func oneRunnersThemeIsAddedToTheBuiltIns() {
        let merged = ThemeCatalog.merged(
            builtIn: builtIn, hostThemes: ["a": [Theme(id: "Mine", background: 9)]])
        #expect(merged.map(\.id) == ["Nord", "Solarized", "Mine"])
    }

    /// **The regression.** Two runners, two themes, and BOTH of them in the
    /// picker. The port's merge produced only the second runner's, because it
    /// rebuilt the catalog from `builtIn` on every call.
    @Test func everyRunnersThemesAreInTheCatalogAtOnce() {
        let merged = ThemeCatalog.merged(
            builtIn: builtIn,
            hostThemes: [
                "a": [Theme(id: "FromA", background: 10)],
                "b": [Theme(id: "FromB", background: 11)],
            ])
        #expect(merged.map(\.id) == ["Nord", "Solarized", "FromA", "FromB"])
    }

    /// Three runners, and the third answering does not cost the first two their
    /// themes. The shape of the bug on a real fleet: every poll was a rebuild.
    @Test func aThirdRunnerAnsweringKeepsTheFirstTwosThemes() {
        var hostThemes = [
            "a": [Theme(id: "FromA")], "b": [Theme(id: "FromB")],
        ]
        hostThemes["c"] = [Theme(id: "FromC")]
        let merged = ThemeCatalog.merged(builtIn: builtIn, hostThemes: hostThemes)
        #expect(merged.map(\.id) == ["Nord", "Solarized", "FromA", "FromB", "FromC"])
    }

    /// The host wins a name collision with a built-in, and wins it IN PLACE —
    /// the picker does not reshuffle because a runner happens to define "Nord".
    @Test func aHostThemeReplacesTheBuiltInOfTheSameNameInPlace() {
        let merged = ThemeCatalog.merged(
            builtIn: builtIn, hostThemes: ["a": [Theme(id: "Nord", background: 99)]])
        #expect(merged.map(\.id) == ["Nord", "Solarized"])
        #expect(merged.first?.background == 99)
    }

    /// Two runners defining one name has no right answer, so the rule is that
    /// there is only ONE answer: sorted runner order, last one wins. This is
    /// what "at random" meant — the same fleet used to resolve differently
    /// depending on which runner's poll landed last.
    @Test func twoRunnersDefiningOneNameResolveTheSameWayEveryTime() {
        let themes = [
            "a": [Theme(id: "Shared", background: 1)],
            "b": [Theme(id: "Shared", background: 2)],
        ]
        let merged = ThemeCatalog.merged(builtIn: builtIn, hostThemes: themes)
        #expect(merged.map(\.id) == ["Nord", "Solarized", "Shared"])
        #expect(merged.last?.background == 2)
    }

    /// The same fleet, assembled in the other order, is the same catalog. The
    /// dictionary makes this true today; the test is what stops a later
    /// signature taking an array of arrivals from quietly making it false
    /// again.
    @Test func theCatalogDoesNotDependOnWhichRunnerAnsweredFirst() {
        var oneWay: [String: [Theme]] = [:]
        oneWay["a"] = [Theme(id: "FromA", background: 1)]
        oneWay["b"] = [Theme(id: "FromB", background: 2)]

        var theOther: [String: [Theme]] = [:]
        theOther["b"] = [Theme(id: "FromB", background: 2)]
        theOther["a"] = [Theme(id: "FromA", background: 1)]

        #expect(
            ThemeCatalog.merged(builtIn: builtIn, hostThemes: oneWay)
                == ThemeCatalog.merged(builtIn: builtIn, hostThemes: theOther))
    }

    /// A runner that went away takes its themes with it and nothing else's.
    /// The tear-down half of the same rule — the port had no way to express it
    /// at all, because the catalog was never anybody's in particular.
    @Test func retiringOneRunnerLeavesTheOthersThemes() {
        var hostThemes = [
            "a": [Theme(id: "FromA")], "b": [Theme(id: "FromB")],
        ]
        hostThemes["a"] = nil
        let merged = ThemeCatalog.merged(builtIn: builtIn, hostThemes: hostThemes)
        #expect(merged.map(\.id) == ["Nord", "Solarized", "FromB"])
    }

    // MARK: - Which one is in force

    /// The user-visible half. A theme belonging to runner A still resolves
    /// after runner B answers — which is exactly what stopped happening, and
    /// the fallback below is why the symptom was "everything went Nord".
    @Test func aThemeFromOneRunnerStillResolvesAfterAnotherAnswers() {
        let merged = ThemeCatalog.merged(
            builtIn: builtIn,
            hostThemes: [
                "a": [Theme(id: "FromA", background: 10)],
                "b": [Theme(id: "FromB", background: 11)],
            ])
        let inForce = ThemeCatalog.inForce(
            named: "FromA", in: merged, fallback: Theme(id: "Nord", background: 1))
        #expect(inForce.id == "FromA")
    }

    /// A name that genuinely no longer exists costs you your colors and not
    /// your terminal.
    @Test func aVanishedThemeFallsBackRatherThanToNothing() {
        let inForce = ThemeCatalog.inForce(
            named: "Gone", in: builtIn, fallback: Theme(id: "Nord", background: 1))
        #expect(inForce.id == "Nord")
    }
}
