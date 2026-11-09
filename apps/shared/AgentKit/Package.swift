// swift-tools-version: 6.0
import Foundation
import PackageDescription

// Shared because both apps must agree about one session.
//
// The two apps still duplicate VTCore.swift by copy. This body of logic is
// larger than that, and two copies would drift in exactly the way that makes a
// phone and a Mac disagree about the same terminal — which is the failure the
// whole derivation model exists to prevent.
//
// This used to say Model.swift was duplicated by copy too, and that was the
// wrong word for it even then: the two models were never copies, they read
// DIFFERENT producers. The phone's is `CoreModel.swift` in this package now,
// and it is here to be tested rather than to be shared — every type in it is
// internal, so the Mac still cannot see it. Its header has the argument.
// Where the Rust client core's static library lands (`apps/macos/build-vt.sh`,
// or `cargo build --release -p farcooler-client`). Only the TESTS link it: the
// Plan's view is the Rust core's (`PlanView.swift`, ov-1016), and AgentKit's
// tests of what reads it call the real thing. AgentKit itself never links it,
// since the watch and the widgets compile its sources and can't; each app
// installs its own call (`PlanViewPresenter.install`), and the tests reach the
// symbol the `-u` below keeps in their link.
let rustLibDir = URL(fileURLWithPath: #filePath)
    .deletingLastPathComponent()  // apps/shared/AgentKit
    .deletingLastPathComponent()  // apps/shared
    .deletingLastPathComponent()  // apps
    .deletingLastPathComponent()  // repo root
    .appendingPathComponent("target/release").path

let package = Package(
    name: "AgentKit",
    // iOS 26 is this app's minimum, so the shared code can use what the phone
    // app uses without straddling a version line that no longer exists. macOS
    // is the same number and for the same reason — the Mac app's own floor in
    // `apps/macos/Package.swift` is 26. watchOS is here because the watch
    // renders the same `FleetSnapshot` the phone does, out of this same file:
    // a second projection written for the watch would be a second thing that
    // can disagree with the daemon about one pane, and the staleness rules are
    // exactly where such a disagreement goes unseen — both surfaces look
    // confident while one of them is wrong.
    // String versions rather than `.v26`, which this PackageDescription does not
    // have yet. Only `swift build`/`swift test` of this package read these — the
    // iOS app compiles these sources directly (see `apps/ios/generate-project.py`)
    // and takes its floor from IPHONEOS_DEPLOYMENT_TARGET, as the watch targets
    // will from WATCHOS_DEPLOYMENT_TARGET; the Mac app takes its own from
    // `apps/macos/Package.swift` and `Info.plist`.
    platforms: [.macOS("26.0"), .iOS("26.0"), .watchOS("26.0")],
    products: [.library(name: "AgentKit", targets: ["AgentKit"])],
    targets: [
        .target(name: "AgentKit"),
        // The captured live events are a resource rather than a string literal
        // so they stay byte-for-byte what the daemon emitted. A fixture
        // reformatted by hand stops testing the thing it exists to test.
        .testTarget(
            name: "AgentKitTests",
            dependencies: ["AgentKit"],
            exclude: ["task_start_lines.json"],  // read from the source tree, not the bundle (ov-237)
            resources: [.copy("live_events.jsonl")],
            linkerSettings: [
                .unsafeFlags(["-L\(rustLibDir)", "-Xlinker", "-u", "-Xlinker", "_farcooler_client_plan_view"]),
                .linkedLibrary("farcooler_client"),
            ]),
    ]
)
