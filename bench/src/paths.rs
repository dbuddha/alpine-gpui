//! Where the bench finds its helpers, apps and output, and where it puts
//! disposable homes. Environment overrides exist for the few machine paths.

use crate::apps::{Identity, RustAnalyzer};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

pub struct BenchPaths {
    pub repo_root: PathBuf,
    pub helpers_dir: PathBuf,
    pub results: PathBuf,
    pub baseline: PathBuf,
    pub home_root: PathBuf,
    pub real_home: PathBuf,
    pub alpine_bundle: PathBuf,
    pub zed_app: PathBuf,
}

/// Short on purpose: Zed's crash-handler socket lives under `$HOME`.
const DEFAULT_HOME_ROOT: &str = "/tmp/alpine-bench";

impl BenchPaths {
    pub fn discover() -> Result<Self, String> {
        let bench_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let repo_root = bench_root
            .parent()
            .ok_or("bench/ has no parent directory")?
            .to_path_buf();
        let real_home = env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or("HOME is not set")?;
        let override_path =
            |name: &str, fallback: PathBuf| env::var_os(name).map_or(fallback, PathBuf::from);
        Ok(Self {
            helpers_dir: bench_root.join("target/helpers"),
            results: bench_root.join("results"),
            baseline: bench_root.join("baseline.tsv"),
            home_root: override_path("BENCH_HOME_ROOT", PathBuf::from(DEFAULT_HOME_ROOT)),
            alpine_bundle: override_path(
                "BENCH_ALPINE_APP",
                real_home.join("Applications/Alpine Editor.app"),
            ),
            zed_app: override_path("BENCH_ZED_APP", PathBuf::from("/Applications/Zed.app")),
            repo_root,
            real_home,
        })
    }

    /// Outside any git checkout, so no editor's git integration sees the
    /// fixtures; next to the disposable homes.
    pub fn fixtures_dir(&self) -> PathBuf {
        self.home_root.join("fixtures/v1")
    }

    /// rust-analyzer's cargo output for both editors, so `open-repo` never
    /// writes the measured checkout's own `target/`.
    pub fn rust_analyzer_target(&self) -> PathBuf {
        self.helpers_dir.with_file_name("rust-analyzer")
    }
}

pub fn identity() -> Identity {
    let user = env::var("USER")
        .or_else(|_| env::var("LOGNAME"))
        .unwrap_or_default();
    Identity {
        user,
        shell: env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_owned()),
        text_encoding: env::var("__CF_USER_TEXT_ENCODING").ok(),
    }
}

/// `channel = "1.97.1"` from `rust-toolchain.toml`.
pub fn toolchain_channel(text: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let (key, value) = line.split_once('=')?;
        (key.trim() == "channel").then(|| value.trim().trim_matches('"').to_owned())
    })
}

/// The rust-analyzer both editors get: an explicit path, or the one in the
/// checkout's pinned toolchain, run with the real rustup and cargo homes.
pub fn rust_analyzer(paths: &BenchPaths, explicit: Option<&Path>) -> Result<RustAnalyzer, String> {
    let rustup_home =
        env::var_os("RUSTUP_HOME").map_or_else(|| paths.real_home.join(".rustup"), PathBuf::from);
    let cargo_home =
        env::var_os("CARGO_HOME").map_or_else(|| paths.real_home.join(".cargo"), PathBuf::from);
    let binary = if let Some(path) = explicit {
        path.to_path_buf()
    } else if let Some(path) = env::var_os("BENCH_RUST_ANALYZER") {
        PathBuf::from(path)
    } else {
        let toolchain_file = paths.repo_root.join("rust-toolchain.toml");
        let text = fs::read_to_string(&toolchain_file)
            .map_err(|error| format!("read {}: {error}", toolchain_file.display()))?;
        let channel = toolchain_channel(&text)
            .ok_or_else(|| format!("{} has no channel", toolchain_file.display()))?;
        rustup_home.join(format!(
            "toolchains/{channel}-aarch64-apple-darwin/bin/rust-analyzer"
        ))
    };
    if !binary.is_file() {
        return Err(format!(
            "rust-analyzer not found at {}; pass --rust-analyzer PATH",
            binary.display()
        ));
    }
    Ok(RustAnalyzer {
        binary,
        rustup_home,
        cargo_home,
        target_dir: paths.rust_analyzer_target(),
    })
}

#[cfg(test)]
mod tests {
    use super::{BenchPaths, toolchain_channel};

    #[test]
    fn toolchain_channel_reads_the_pinned_release() {
        let text = "[toolchain]\nchannel = \"1.97.1\"\ncomponents = [\"clippy\"]\n";
        assert_eq!(toolchain_channel(text).as_deref(), Some("1.97.1"));
        assert_eq!(toolchain_channel("[toolchain]\n"), None);
    }

    #[test]
    fn discovered_paths_hang_off_the_bench_directory() -> Result<(), String> {
        let paths = BenchPaths::discover()?;
        let bench_root = paths.repo_root.join("bench");
        assert!(paths.helpers_dir.starts_with(&bench_root));
        assert!(paths.results.starts_with(&bench_root));
        assert_eq!(paths.baseline, bench_root.join("baseline.tsv"));
        assert!(paths.fixtures_dir().starts_with(&paths.home_root));
        assert_eq!(
            paths.rust_analyzer_target(),
            bench_root.join("target/rust-analyzer")
        );
        Ok(())
    }
}
