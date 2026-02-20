fn main() {
    let out_dir = "./include";
    let pkg = env!("CARGO_PKG_NAME"); // "tornade-core"

    let bridges = vec!["src/ffi.rs"];

    for bridge in &bridges {
        println!("cargo:rerun-if-changed={}", bridge);
    }

    swift_bridge_build::parse_bridges(bridges)
        .write_all_concatenated(out_dir, pkg);

    // Sync the generated Swift bindings and C headers to every location that
    // the Xcode project expects them, so that a plain `cargo build` keeps
    // everything in sync without requiring a separate copy step or an Xcode
    // build phase to run first.
    //
    // Paths are relative to the tornade-core/ package root.
    // Failures are silently ignored so that the crate can still be built in
    // environments where the Xcode project tree is absent (CI, etc.).
    let xcode_src = format!("{}/{}", out_dir, pkg);

    let destinations = [
        // Xcode project source tree (tracked in git, used by Swift compiler)
        "../TornadeUI-macOS/Tornade/Tornade/tornade-core",
        // Libraries staging area (used for HEADER_SEARCH_PATHS)
        "../TornadeUI-macOS/Libraries/tornade-core",
    ];

    for dest in &destinations {
        let _ = std::fs::create_dir_all(dest);
        let _ = std::fs::copy(
            format!("{}/{}.swift", xcode_src, pkg),
            format!("{}/{}.swift", dest, pkg),
        );
        let _ = std::fs::copy(
            format!("{}/{}.h", xcode_src, pkg),
            format!("{}/{}.h", dest, pkg),
        );
        let _ = std::fs::copy(
            format!("{}/SwiftBridgeCore.swift", out_dir),
            format!("{}/SwiftBridgeCore.swift", dest),
        );
        let _ = std::fs::copy(
            format!("{}/SwiftBridgeCore.h", out_dir),
            format!("{}/SwiftBridgeCore.h", dest),
        );
    }

    // SwiftBridgeCore also lives at the Libraries root for the bridging header
    let _ = std::fs::copy(
        format!("{}/SwiftBridgeCore.swift", out_dir),
        "../TornadeUI-macOS/Libraries/SwiftBridgeCore.swift",
    );
    let _ = std::fs::copy(
        format!("{}/SwiftBridgeCore.h", out_dir),
        "../TornadeUI-macOS/Libraries/SwiftBridgeCore.h",
    );
}
