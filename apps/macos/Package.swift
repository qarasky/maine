// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "maine",
    platforms: [
        .macOS(.v14)
    ],
    products: [
        .executable(name: "Maine", targets: ["Maine"])
    ],
    dependencies: [
        .package(url: "https://github.com/migueldeicaza/SwiftTerm.git", from: "1.99.0")
    ],
    targets: [
        .executableTarget(
            name: "Maine",
            dependencies: [
                .product(name: "SwiftTerm", package: "SwiftTerm")
            ],
            path: "Sources/Maine"
        ),
        .testTarget(
            name: "MaineTests",
            dependencies: ["Maine"],
            path: "Tests/MaineTests"
        )
    ]
)
