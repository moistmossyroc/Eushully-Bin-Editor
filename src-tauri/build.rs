fn main() {
    // The fanpatch loader (version.dll) is a prebuilt CreateFile-hijack binary
    // bundled into the editor via include_bytes! (see commands.rs). It is
    // deliberately NOT committed to the public source repo. Detect it here:
    //   present (local/dev builds) -> enable the bundled-loader code path;
    //   absent (a clone of the public repo) -> the crate still compiles and the
    //   loader-write step degrades gracefully.
    println!("cargo::rustc-check-cfg=cfg(has_bundled_loader)");
    println!("cargo::rerun-if-changed=assets/version.dll");
    if std::path::Path::new("assets/version.dll").exists() {
        println!("cargo::rustc-cfg=has_bundled_loader");
    }
    tauri_build::build()
}
