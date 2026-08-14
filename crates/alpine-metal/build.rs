//! Selects the reviewed offline Metal library embedded by the native backend.

use std::{env, path::PathBuf};

fn main() {
    let default_library =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../shaders/offscreen.metallib");
    let library = env::var_os("ALPINE_METALLIB_PATH").map_or(default_library, PathBuf::from);

    println!("cargo:rerun-if-env-changed=ALPINE_METALLIB_PATH");
    println!("cargo:rerun-if-changed={}", library.display());
    println!("cargo:rustc-env=ALPINE_METALLIB_PATH={}", library.display());
}
