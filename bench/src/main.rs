//! Local black-box benchmark for Alpine Editor, Zed and a reference editor
//! built on `AppKit`, measured the same way. Never shipped: it needs a GUI
//! session, and input and capture need Accessibility and Screen Recording.

mod analysis;
mod apps;
mod baseline;
mod cli;
mod fixtures;
mod helpers;
mod isolation;
mod paths;
mod procs;
mod stamp;
mod stats;
mod trial;
mod tsv;
mod workload;

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match cli::parse(&args).and_then(cli::execute) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("bench: {error}");
            ExitCode::FAILURE
        }
    }
}

/// A fresh scratch directory under `bench/target/test-tmp` for one test.
#[cfg(test)]
fn test_dir(name: &str) -> Result<std::path::PathBuf, String> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target/test-tmp")
        .join(format!("{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|error| format!("{}: {error}", dir.display()))?;
    Ok(dir)
}
