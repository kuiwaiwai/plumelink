fn main() {
    if std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("wasm32") {
        // Older WebViews require the canonical zero table index for call_indirect.
        println!("cargo:rustc-link-arg-cdylib=--compress-relocations");
        println!("cargo:rustc-link-arg-cdylib=--strip-debug");
    }
}
