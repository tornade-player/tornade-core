/// Copy `src` to `dst` only when the content differs (or `dst` doesn't exist).
/// This avoids unnecessary writes — and sandbox violations — when the build
/// script re-runs but `ffi.rs` hasn't actually changed.
fn copy_if_changed(src: &str, dst: &str) {
    let src_bytes = match std::fs::read(src) {
        Ok(b) => b,
        Err(_) => return,
    };
    if let Ok(dst_bytes) = std::fs::read(dst)
        && src_bytes == dst_bytes
    {
        return;
    }
    let _ = std::fs::copy(src, dst);
}

fn main() {
    let out_dir = "./include";
    let pkg = env!("CARGO_PKG_NAME"); // "tornade-core"

    let bridges = vec!["src/ffi.rs"];

    for bridge in &bridges {
        println!("cargo:rerun-if-changed={bridge}");
    }

    swift_bridge_build::parse_bridges(bridges).write_all_concatenated(out_dir, pkg);

    // Sync the generated Swift bindings and C headers to every location that
    // the Xcode project expects them, so that a plain `cargo build` keeps
    // everything in sync without requiring a separate copy step or an Xcode
    // build phase to run first.
    //
    // Paths are relative to the tornade-core/ package root.
    // Failures are silently ignored so that the crate can still be built in
    // environments where the Xcode project tree is absent (CI, etc.).
    let xcode_src = format!("{out_dir}/{pkg}");

    let destinations = [
        // Xcode project source tree (tracked in git, used by Swift compiler)
        "../TornadeUI-macOS/Tornade/Tornade/tornade-core",
        // Libraries staging area (used for HEADER_SEARCH_PATHS)
        "../TornadeUI-macOS/Libraries/tornade-core",
    ];

    for dest in &destinations {
        let _ = std::fs::create_dir_all(dest);
        copy_if_changed(
            &format!("{xcode_src}/{pkg}.swift"),
            &format!("{dest}/{pkg}.swift"),
        );
        copy_if_changed(&format!("{xcode_src}/{pkg}.h"), &format!("{dest}/{pkg}.h"));
        copy_if_changed(
            &format!("{out_dir}/SwiftBridgeCore.swift"),
            &format!("{dest}/SwiftBridgeCore.swift"),
        );
        copy_if_changed(
            &format!("{out_dir}/SwiftBridgeCore.h"),
            &format!("{dest}/SwiftBridgeCore.h"),
        );
    }

    // SwiftBridgeCore also lives at the Libraries root for the bridging header
    copy_if_changed(
        &format!("{out_dir}/SwiftBridgeCore.swift"),
        "../TornadeUI-macOS/Libraries/SwiftBridgeCore.swift",
    );
    copy_if_changed(
        &format!("{out_dir}/SwiftBridgeCore.h"),
        "../TornadeUI-macOS/Libraries/SwiftBridgeCore.h",
    );
}
