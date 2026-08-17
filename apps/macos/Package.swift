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
        //
        // Which is why `Package.resolved` is now tracked, and why that file is
        // more load-bearing than it looks. It pins the resolution so a fresh
        // checkout cannot quietly take a newer Sparkle — but it is also
        // GENERATED, and a `swift build` that rewrites it (a different toolchain,
        // a changed format) DIRTIES THE TREE. A dirty tree makes
        // `scripts/version.sh channel` answer `local`, which is how a canary
        // silently becomes a local build. Every caller resolves its per-channel
        // values before building for exactly this reason; if you ever see a
        // build report the wrong channel, `git status` on this file first.
        .package(url: "https://github.com/sparkle-project/Sparkle", from: "2.6.0"),
    ],
    targets: [
        // The Rust terminal core. One emulator, in the language the daemon
        // already speaks, behind a Far Cooler-owned C ABI — so the same core
        // serves Mac, iOS and Android, and each platform writes only a
        // renderer. See crates/vt.
        .systemLibrary(name: "CFarCoolerVT", path: "Sources/CFarCoolerVT"),
        // The client core, for the enrollment ceremony and nothing else.
        //
        // The Mac reaches runners by running `ssh`, which is why this library
        // was a phone's concern until now. What changed is that every rule
        // deciding whether a scanned code is acceptable lives in
        // crates/client/src/ceremony.rs, and a Mac that could not call it would
        // be a third implementation of those rules — which is the duplication
        // the Rust core exists to prevent.
        .systemLibrary(name: "CFarCoolerClient", path: "Sources/CFarCoolerClient"),
        .executableTarget(
            name: "Far Cooler",
            dependencies: [
                "CFarCoolerVT",
                "CFarCoolerClient",
                .product(name: "AgentKit", package: "AgentKit"),
                .product(name: "Sparkle", package: "Sparkle"),
            ],
            path: "Sources/FarCooler",
            linkerSettings: [
                .unsafeFlags(["-L\(rustLibDir)"]),
                .linkedLibrary("farcooler_vt"),
                .linkedLibrary("farcooler_client"),
                // Where the framework will live once build-app.sh assembles the
                // bundle. SwiftPM links against its own artifact directory and has
                // no reason to know about a bundle it did not create, so without
                // this the app builds and then dies at launch with "Library not
                // loaded: @rpath/Sparkle.framework/Versions/B/Sparkle".
                .unsafeFlags(["-Xlinker", "-rpath", "-Xlinker", "@executable_path/../Frameworks"]),
            ]
        ),
        // The rules with teeth, and only those.
        //
        // This package has never had a test target, and most of it does not
        // want one: a SwiftUI view is verified by looking at it. What is here
        // is the `~/.ssh/config` alias logic, whose failure modes are a runner
        // taking over `github.com` for every push on this Mac, and Key A
        // landing in a file whose deletion would then break Far Cooler rather
        // than only Zed. Neither is visible by looking.
        //
        // The byte MECHANICS are deliberately not here — the lock, the two
        // `fsync`s, the checksummed backup belong to `crates/daemon/src/fence.rs`
        // and are tested in Rust beside the `authorized_keys` fixtures, so that
        // one routine has one test suite. What IS here about the write is the
        // boundary: that Swift reaches that routine at all, that the block lands
        // above an `Include` rather than below it, and that each word the FFI can
        // answer becomes the right sentence. Those tests aim at a scratch path,
        // never `~/.ssh/config` — a suite that rewrote the config of whoever ran
        // it would be worse than no suite, which is why `SshConfig.write` takes
        // its path as a defaulted parameter.
        .testTarget(
            name: "CeremonyTests",
            dependencies: ["Far Cooler"],
            path: "Tests/CeremonyTests"
        ),
    ]
)
