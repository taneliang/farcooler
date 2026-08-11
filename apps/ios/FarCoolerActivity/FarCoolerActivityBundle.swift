import SwiftUI
import WidgetKit

/// The extension's entry point.
///
/// A `WidgetBundle` rather than declaring `AgentActivityWidget` itself `@main`,
/// because this extension will gain home screen widgets and there is no way to
/// add a second one to a `@main` widget without rewriting this file into exactly
/// what it already is.
@main
struct FarCoolerActivityBundle: WidgetBundle {
    var body: some Widget {
        AgentActivityWidget()
    }
}
