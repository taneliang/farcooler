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
    name: "Far Cooler",
    platforms: [.macOS("26.0")],
    dependencies: [
        .package(path: "../shared/AgentKit"),
        // Sparkle, for the reason docs/superpowers/specs/2026-08-16-sparkle-auto-update-design.md
        // gives: a Developer ID app has no App Store to update it. The FIRST remote
        // dependency this package has ever had — everything else here is a path
        // dependency or a system library.
        .package(url: "https://github.com/sparkle-project/Sparkle", from: "2.6.0"),
    ],
    targets: [
        // The Rust terminal core. One emulator, in the language the daemon
        // already speaks, behind a Far Cooler-owned C ABI — so the same core
        // serves Mac, iOS and Android, and each platform writes only a
        // renderer. See crates/vt.
        .systemLibrary(name: "CFarCoolerVT", path: "Sources/CFarCoolerVT"),
        .executableTarget(
            name: "Far Cooler",
            dependencies: [
                "CFarCoolerVT",
                .product(name: "AgentKit", package: "AgentKit"),
                .product(name: "Sparkle", package: "Sparkle"),
            ],
            path: "Sources/FarCooler",
            linkerSettings: [
                .unsafeFlags(["-L\(rustLibDir)"]),
                .linkedLibrary("farcooler_vt"),
                // Where the framework will live once build-app.sh assembles the
                // bundle. SwiftPM links against its own artifact directory and has
                // no reason to know about a bundle it did not create, so without
                // this the app builds and then dies at launch with "Library not
                // loaded: @rpath/Sparkle.framework/Versions/B/Sparkle".
                .unsafeFlags(["-Xlinker", "-rpath", "-Xlinker", "@executable_path/../Frameworks"]),
            ]
        ),
    ]
)
