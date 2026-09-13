//! Compiles the public Apple signpost macro shim on the shipping target.

fn main() {
    println!("cargo:rerun-if-changed=src/editor_signposts.c");

    let apple_silicon_macos = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos")
        && std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("aarch64");
    if apple_silicon_macos {
        cc::Build::new()
            .file("src/editor_signposts.c")
            .warnings(true)
            .flag_if_supported("-Werror")
            .compile("alpine_editor_signposts");
    }
}
