// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "telex",
    platforms: [
        .macOS(.v13)
    ],
    products: [
        .library(name: "TelexRuntime", targets: ["TelexRuntime"])
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
            path: "swift/telex-runtime/Sources/TelexRuntime"
        ),
        .target(
            name: "CTelexShm",
            path: "swift/telex-runtime/Sources/CTelexShm",
            publicHeadersPath: "include"
        ),
        .target(
            name: "CTelexShmFfi",
            path: "swift/telex-runtime/Sources/CTelexShmFfi",
            publicHeadersPath: "include",
            linkerSettings: [
                // Consumer must build libtelex_shm_ffi.a (cargo build --release -p telex-shm-ffi)
                // and add its directory to LIBRARY_SEARCH_PATHS or pass -Xlinker -L<path>.
                .linkedLibrary("telex_shm_ffi"),
            ]
        ),
        .testTarget(
            name: "TelexRuntimeTests",
            dependencies: ["TelexRuntime"],
            path: "swift/telex-runtime/Tests/TelexRuntimeTests"
        ),
    ]
)
