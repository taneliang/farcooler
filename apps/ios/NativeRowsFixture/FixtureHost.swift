import SwiftUI

@main
struct NativeRowsFixtureHost: App {
    var body: some Scene { WindowGroup { FixtureScreen() } }
}

@MainActor
private struct FixtureScreen: View {
    @State private var state: FixtureState = .loading

    var body: some View {
        Group {
            switch state {
            case .loading: Text("Loading fixture")
            case .failed(let failure): Text("Fixture error: \(failure.rawValue)").accessibilityIdentifier("fixture-error")
            case .ready(let fixture):
                let boxes = fixture.tools.map(AgentRowBox.init)
                VStack {
                    Text("Verified fixture: \(FixtureLoader.digest)")
                        .accessibilityIdentifier("fixture-ready-\(FixtureLoader.digest)")
                    NativeToolGroupRow(id: "tools:\(FixtureLoader.ids[1])", boxes: boxes)
                }
            }
        }
        .environment(\.selectsRanges, false)
        .environment(\.openURL, OpenURLAction { _ in .discarded })
        .task {
            state = .resolve { try FixtureLoader.load() }
        }
    }
}
