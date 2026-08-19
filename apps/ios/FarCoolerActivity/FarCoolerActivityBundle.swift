import SwiftUI
import WidgetKit

/// The extension's entry point.
///
/// A `WidgetBundle` rather than declaring `AgentActivityWidget` itself `@main`,
/// because this extension has gained home screen widgets and there is no way to
/// add a second one to a `@main` widget without rewriting this file into exactly
/// what it already is.
///
/// This list is the only thing that makes a widget exist. WidgetKit discovers
/// widgets by asking the `@main` bundle for them, so a `Widget` that compiles
/// but is not named here is absent from the gallery with no error anywhere —
/// no warning, no log, just a size the person cannot add.
@main
struct FarCoolerActivityBundle: WidgetBundle {
    var body: some Widget {
        AgentActivityWidget()
        FleetWidget()
    }
}
