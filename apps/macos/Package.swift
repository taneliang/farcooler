// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "Overnight",
    platforms: [.macOS(.v14)],
    targets: [
        .executableTarget(
            name: "Overnight",
            path: "Sources/Overnight"
        )
    ]
)
