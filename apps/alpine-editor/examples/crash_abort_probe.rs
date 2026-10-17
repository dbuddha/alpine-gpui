//! Release-mode probe: the crash hook must run under `panic = "abort"`.
//! Needs `--cfg alpine_native_validation`; usage: <disposable HOME> <file>.
//! Expect SIGABRT, a crash log and a recovery journal under that HOME.

#[cfg(all(alpine_native_validation, target_os = "macos", target_arch = "aarch64"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = std::env::args_os().skip(1);
    let (Some(home), Some(document)) = (arguments.next(), arguments.next()) else {
        return Err("usage: crash_abort_probe <home> <file>".into());
    };
    alpine_editor::native_validation::crash_probe(
        std::path::Path::new(&home),
        std::path::Path::new(&document),
        "edited by the probe\n",
    )?;
    Ok(())
}

#[cfg(not(all(alpine_native_validation, target_os = "macos", target_arch = "aarch64")))]
fn main() {
    eprintln!("skipped: needs --cfg alpine_native_validation on Apple Silicon macOS");
}
