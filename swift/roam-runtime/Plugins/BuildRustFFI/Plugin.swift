import PackagePlugin
import Foundation

@main
struct BuildRustFFI: BuildToolPlugin {
    func createBuildCommands(
        context: PluginContext,
        target: Target
    ) async throws -> [Command] {
        // Find the roam workspace root (Package.swift is in swift/roam-runtime/)
        let packageDir = context.package.directoryURL
        let roamRoot = packageDir
            .deletingLastPathComponent()  // swift/
            .deletingLastPathComponent()  // roam/

        let cargoManifest = roamRoot.appending(path: "Cargo.toml")
        // Use the plugin work directory as CARGO_TARGET_DIR so cargo can write
        // inside the sandbox without needing --disable-sandbox.
        let cargoTargetDir = context.pluginWorkDirectoryURL.appending(path: "cargo-target")
        let staticLib = cargoTargetDir.appending(path: "release/libroam_shm_ffi.a")

        let outputDir = context.pluginWorkDirectoryURL

        return [
            .prebuildCommand(
                displayName: "Build Rust FFI (roam-shm-ffi)",
                executable: URL(fileURLWithPath: "/usr/bin/env"),
                arguments: [
                    "sh", "-c",
                    """
                    set -e
                    export PATH="\(NSHomeDirectory())/.cargo/bin:$PATH"
                    export CARGO_TARGET_DIR='\(cargoTargetDir.path)'
                    cargo build --release --manifest-path '\(cargoManifest.path)' -p roam-shm-ffi
                    cp '\(staticLib.path)' '\(outputDir.path)/libroam_shm_ffi.a'
                    """,
                ],
                outputFilesDirectory: outputDir
            ),
        ]
    }
}
