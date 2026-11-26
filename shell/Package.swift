// swift-tools-version:5.9
// The Pane menu bar app. Build with `make app` (needs the Rust static
// library in ../target/aarch64-apple-darwin/release first).
import PackageDescription

let rustLib = Context.packageDirectory + "/../target/aarch64-apple-darwin/release"

let package = Package(
    name: "PaneShell",
    platforms: [.macOS(.v14)],
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
            ]
        ),
    ]
)
