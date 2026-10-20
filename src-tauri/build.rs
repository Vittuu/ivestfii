fn main() {
    tauri_build::build();
    // Tauri links its Windows manifest/resources only into package binaries.
    // The alternate preview executable needs the same Common Controls v6
    // manifest (TaskDialogIndirect is unavailable without it).
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let output = std::env::var("OUT_DIR").expect("Cargo supplies OUT_DIR");
        println!("cargo:rustc-link-arg-examples={output}/resource.lib");
    }
}
