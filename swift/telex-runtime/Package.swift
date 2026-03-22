// swift-tools-version: 6.0
import PackageDescription

// telex-shm-ffi is a Rust staticlib built by `cargo build --release -p telex-shm-ffi`.
// It lives at target/release/libtelex_shm_ffi.a relative to the telex workspace root,
// which is two directories up from this Package.swift.
let telexRoot = "../.."
let rustLibDir = "\(telexRoot)/target/release"

let package = Package(
    name: "telex-runtime",
    platforms: [
        .macOS(.v13)
    ],
    products: [
        .library(name: "TelexRuntime", targets: ["TelexRuntime"]),
        .executable(name: "shm-bootstrap-client", targets: ["shm-bootstrap-client"]),
        .executable(name: "shm-guest-client", targets: ["shm-guest-client"]),
    ],
    dependencies: [
        .package(url: "https://github.com/apple/swift-nio.git", from: "2.92.0")
    ],
    targets: [
        .target(
            name: "TelexRuntime",
            dependencies: [
                "CTelexShm",
                "CTelexShmFfi",
                .product(name: "NIO", package: "swift-nio"),
                .product(name: "NIOCore", package: "swift-nio"),
                .product(name: "NIOPosix", package: "swift-nio"),
            ],
            path: "Sources/TelexRuntime"
        ),
        .target(
            name: "CTelexShm",
            path: "Sources/CTelexShm",
            publicHeadersPath: "include"
        ),
        .target(
            name: "CTelexShmFfi",
            path: "Sources/CTelexShmFfi",
            publicHeadersPath: "include",
            linkerSettings: [
                .unsafeFlags(["-L\(rustLibDir)", "-ltelex_shm_ffi"]),
            ]
        ),
        .executableTarget(
            name: "shm-bootstrap-client",
            dependencies: ["TelexRuntime"],
            path: "Sources/shm-bootstrap-client"
        ),
        .executableTarget(
            name: "shm-guest-client",
            dependencies: ["TelexRuntime"],
            path: "Sources/shm-guest-client"
        ),
        .testTarget(
            name: "TelexRuntimeTests",
            dependencies: ["TelexRuntime"],
            path: "Tests/TelexRuntimeTests"
        ),
    ]
)
