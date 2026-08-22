//! Compiles the public Apple signpost macro shim on the shipping target.

fn main() {
    println!("cargo:rerun-if-changed=src/studio_signposts.c");

    let apple_silicon_macos = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos")
        && std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("aarch64");
    if apple_silicon_macos {
        cc::Build::new()
            .file("src/studio_signposts.c")
            .warnings(true)
            .flag_if_supported("-Werror")
            .compile("alpine_studio_signposts");
    }
}
