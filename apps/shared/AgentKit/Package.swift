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
    platforms: [.macOS(.v14), .iOS(.v17)],
    products: [.library(name: "AgentKit", targets: ["AgentKit"])],
    targets: [
        .target(name: "AgentKit"),
        .testTarget(name: "AgentKitTests", dependencies: ["AgentKit"]),
    ]
)
