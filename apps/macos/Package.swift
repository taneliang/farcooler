// swift-tools-version: 6.0
import Foundation
import PackageDescription

// Where the Rust terminal core's static library lands.
//
// SwiftPM has no notion of "build the Rust first", and a relative -L is
// resolved against a working directory SwiftPM does not promise. So the path is
// derived from this file's own location, which is the one thing that is always
// known. build-vt.sh puts the library there.
let repoRoot = URL(fileURLWithPath: #filePath)
    .deletingLastPathComponent()  // apps/macos
    .deletingLastPathComponent()  // apps
    .deletingLastPathComponent()  // repo root
let rustLibDir = repoRoot.appendingPathComponent("target/release").path

let package = Package(
    name: "Overnight",
    platforms: [.macOS(.v14)],
    dependencies: [.package(path: "../shared/AgentKit")],
    targets: [
        // The Rust terminal core. One emulator, in the language the daemon
        // already speaks, behind an Overnight-owned C ABI — so the same core
        // serves Mac, iOS and Android, and each platform writes only a
        // renderer. See crates/vt.
        .systemLibrary(name: "COvernightVT", path: "Sources/COvernightVT"),
        .executableTarget(
            name: "Overnight",
            dependencies: ["COvernightVT", .product(name: "AgentKit", package: "AgentKit")],
            path: "Sources/Overnight",
            linkerSettings: [
                .unsafeFlags(["-L\(rustLibDir)"]),
                .linkedLibrary("overnight_vt"),
            ]
        ),
    ]
)
