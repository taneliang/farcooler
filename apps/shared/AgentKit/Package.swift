// swift-tools-version: 6.0
import PackageDescription

// Shared because both apps must agree about one session.
//
// The two apps already duplicate Model.swift and VTCore.swift by copy. This
// body of logic is larger than either, and two copies would drift in exactly
// the way that makes a phone and a Mac disagree about the same terminal —
// which is the failure the whole derivation model exists to prevent.
let package = Package(
    name: "AgentKit",
    // iOS 26 is this app's minimum, so the shared code can use what the phone
    // app uses without straddling a version line that no longer exists. macOS
    // stays at 14: the Mac app has its own floor and its own glass fallback.
    // String versions rather than `.v26`, which this PackageDescription does not
    // have yet. Only `swift build`/`swift test` of this package read these — the
    // iOS app compiles these sources directly (see `apps/ios/generate-project.py`)
    // and takes its floor from IPHONEOS_DEPLOYMENT_TARGET; the Mac app takes its
    // own from `apps/macos/Package.swift` and `Info.plist`.
    platforms: [.macOS("26.0"), .iOS("26.0")],
    products: [.library(name: "AgentKit", targets: ["AgentKit"])],
    targets: [
        .target(name: "AgentKit"),
        // The captured live events are a resource rather than a string literal
        // so they stay byte-for-byte what the daemon emitted. A fixture
        // reformatted by hand stops testing the thing it exists to test.
        .testTarget(
            name: "AgentKitTests",
            dependencies: ["AgentKit"],
            resources: [.copy("live_events.jsonl")]),
    ]
)
