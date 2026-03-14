fn main() {
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();

    // Force the linker to include every object file from every statically-linked
    // archive.  This is a belt-and-suspenders measure alongside the `pub use *`
    // re-export in src/lib.rs; together they ensure no engine symbol is lost.
    match target_os.as_str() {
        "macos" | "ios" => {
            println!("cargo:rustc-link-arg-cdylib=-Wl,-all_load");
        }
        "linux" | "android" => {
            println!("cargo:rustc-link-arg-cdylib=-Wl,--whole-archive");
            println!("cargo:rustc-link-arg-cdylib=-Wl,--no-whole-archive");
        }
        _ => {}
    }
}
