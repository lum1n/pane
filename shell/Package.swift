// swift-tools-version:6.2
// The Pane menu bar app. Build with `make app` (needs the Rust static
// library in ../target/aarch64-apple-darwin/release first).
import PackageDescription

let rustLib = Context.packageDirectory + "/../target/aarch64-apple-darwin/release"

let package = Package(
    name: "PaneShell",
    platforms: [.macOS(.v26)],
    targets: [
        .systemLibrary(name: "CPane", path: "Sources/CPane"),
        .executableTarget(
            name: "Pane",
            dependencies: ["CPane"],
            path: "Sources/PaneShell",
            linkerSettings: [
                .unsafeFlags(["-L", rustLib]),
                .linkedLibrary("pane_ffi"),
                .linkedLibrary("objc"),
                .linkedLibrary("iconv"),
                .linkedFramework("AppKit"),
                .linkedFramework("ApplicationServices"),
                .linkedFramework("CoreFoundation"),
                .linkedFramework("CoreGraphics"),
                .linkedFramework("ColorSync"),
                .linkedFramework("Foundation"),
                .linkedFramework("Security"),
                .linkedFramework("ServiceManagement"),
            ]
        ),
    ],
    // Swift 5 mode: the code already isolates to the main actor by hand
    // (`@MainActor`, `assumeIsolated`), so strict concurrency would only add churn.
    swiftLanguageModes: [.v5]
)
