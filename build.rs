fn main() {
    println!("cargo:rerun-if-changed=native/macos_focus.m");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos")
        && std::env::var_os("CARGO_FEATURE_VIEWER").is_some()
    {
        cc::Build::new()
            .file("native/macos_focus.m")
            .compile("waytransfer_macos_focus");
        println!("cargo:rustc-link-lib=framework=AppKit");
    }
}
