// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "Overnight",
    platforms: [.macOS(.v14)],
    dependencies: [
        // A real VT emulator. Overnight's accepted long-term core is
        // libghostty-vt behind an Overnight-owned C ABI shared by Mac, iOS and
        // Android. SwiftTerm is the Mac-only interim: it gives correct VT
        // parsing, mouse reporting, selection and scrollback today, which is
        // what makes a coding agent usable, without hand-rolling an emulator.
        .package(url: "https://github.com/migueldeicaza/SwiftTerm", from: "1.2.0")
    ],
    targets: [
        .executableTarget(
            name: "Overnight",
            dependencies: ["SwiftTerm"],
            path: "Sources/Overnight"
        )
    ]
)
