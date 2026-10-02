//! How each app is launched: always a fresh process from its binary, with a
//! disposable HOME and the minimal environment a Dock launch provides.

use std::ffi::OsString;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum App {
    Alpine,
    Zed,
    #[allow(
        clippy::enum_variant_names,
        reason = "AppKit is the framework the reference editor is built on"
    )]
    AppKit,
}

impl App {
    pub fn parse(name: &str) -> Result<Self, String> {
        match name {
            "alpine" => Ok(Self::Alpine),
            "zed" => Ok(Self::Zed),
            "appkit" => Ok(Self::AppKit),
            other => Err(format!(
                "unknown app {other:?}; expected alpine, zed or appkit"
            )),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Alpine => "alpine",
            Self::Zed => "zed",
            Self::AppKit => "appkit",
        }
    }
}

/// The PATH a Dock launch gets from launchd.
pub const DOCK_PATH: &str = "/usr/bin:/bin:/usr/sbin:/sbin";

/// Alpine's default content size, which the reference app copies.
pub const WINDOW_WIDTH: u32 = 960;
pub const WINDOW_HEIGHT: u32 = 540;
/// Alpine's measured frame height (content plus a 32 pt title bar). Zed
/// draws its title bar inside its content, so it gets the whole frame.
pub const FRAME_HEIGHT: u32 = 572;

/// macOS caps a Unix socket path at 104 bytes. Zed binds its crash-handler
/// socket at `$HOME/Library/Caches/Zed/zed-crash-handler-<pid>`.
const ZED_SOCKET_SUFFIX: &str = "/Library/Caches/Zed/zed-crash-handler-99999";
const SOCKET_PATH_MAX: usize = 103;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RustAnalyzer {
    pub binary: PathBuf,
    pub rustup_home: PathBuf,
    pub cargo_home: PathBuf,
}

/// Values copied from the bench's own environment into the app's.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Identity {
    pub user: String,
    pub shell: String,
    pub text_encoding: Option<String>,
}

pub struct LaunchInputs<'a> {
    pub app: App,
    pub home: &'a Path,
    pub tmpdir: &'a Path,
    pub documents: &'a [PathBuf],
    pub identity: &'a Identity,
    pub rust_analyzer: Option<&'a RustAnalyzer>,
    pub alpine_bundle: &'a Path,
    pub zed_app: &'a Path,
    pub reference_binary: &'a Path,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaunchSpec {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    pub env: Vec<(String, OsString)>,
    /// Files written under the disposable home before launch.
    pub seed_files: Vec<(PathBuf, String)>,
}

pub fn dock_env(home: &Path, tmpdir: &Path, identity: &Identity) -> Vec<(String, OsString)> {
    let mut env = vec![
        ("HOME".to_owned(), home.as_os_str().to_owned()),
        ("TMPDIR".to_owned(), tmpdir.as_os_str().to_owned()),
        ("PATH".to_owned(), OsString::from(DOCK_PATH)),
        ("USER".to_owned(), OsString::from(&identity.user)),
        ("LOGNAME".to_owned(), OsString::from(&identity.user)),
        ("SHELL".to_owned(), OsString::from(&identity.shell)),
    ];
    if let Some(encoding) = &identity.text_encoding {
        env.push((
            "__CF_USER_TEXT_ENCODING".to_owned(),
            OsString::from(encoding),
        ));
    }
    env
}

pub fn zed_data_dir(home: &Path) -> PathBuf {
    home.join("zed-data")
}

fn json_string(value: &str) -> String {
    let mut quoted = String::from("\"");
    for character in value.chars() {
        match character {
            '"' => quoted.push_str("\\\""),
            '\\' => quoted.push_str("\\\\"),
            '\n' => quoted.push_str("\\n"),
            '\t' => quoted.push_str("\\t"),
            control if u32::from(control) < 0x20 => {
                // Writing into a String cannot fail.
                let _ = write!(quoted, "\\u{:04x}", u32::from(control));
            }
            other => quoted.push(other),
        }
    }
    quoted.push('"');
    quoted
}

/// Zed's isolated settings: no updates, telemetry, AI or sign-in server, a
/// steady caret like Alpine's, and Alpine's font metrics (Menlo 15/22).
pub fn zed_settings(rust_analyzer: Option<&Path>) -> String {
    let mut lines = vec![
        "  \"auto_update\": false".to_owned(),
        "  \"telemetry\": { \"diagnostics\": false, \"metrics\": false }".to_owned(),
        "  \"disable_ai\": true".to_owned(),
        "  \"server_url\": \"http://127.0.0.1:9\"".to_owned(),
        "  \"cursor_blink\": false".to_owned(),
        "  \"buffer_font_family\": \"Menlo\"".to_owned(),
        "  \"buffer_font_size\": 15".to_owned(),
        "  \"buffer_line_height\": { \"custom\": 1.4667 }".to_owned(),
    ];
    if let Some(binary) = rust_analyzer {
        lines.push(format!(
            "  \"lsp\": {{ \"rust-analyzer\": {{ \"binary\": {{ \"path\": {} }} }} }}",
            json_string(&binary.to_string_lossy())
        ));
    }
    format!("{{\n{}\n}}\n", lines.join(",\n"))
}

pub fn check_zed_home(home: &Path) -> Result<(), String> {
    let length = home.as_os_str().len() + ZED_SOCKET_SUFFIX.len();
    if length > SOCKET_PATH_MAX {
        return Err(format!(
            "disposable home {} is too long for Zed's crash-handler socket ({length} > {SOCKET_PATH_MAX} bytes); set BENCH_HOME_ROOT to a shorter directory",
            home.display()
        ));
    }
    Ok(())
}

fn rust_analyzer_env(env: &mut Vec<(String, OsString)>, rust_analyzer: &RustAnalyzer) {
    env.push((
        "RUSTUP_HOME".to_owned(),
        rust_analyzer.rustup_home.as_os_str().to_owned(),
    ));
    env.push((
        "CARGO_HOME".to_owned(),
        rust_analyzer.cargo_home.as_os_str().to_owned(),
    ));
}

/// Workloads always pass documents; the Zed isolation probe passes none so
/// a probe can never add a recent document.
pub fn launch_spec(inputs: &LaunchInputs<'_>) -> Result<LaunchSpec, String> {
    let mut env = dock_env(inputs.home, inputs.tmpdir, inputs.identity);
    let mut args: Vec<OsString> = Vec::new();
    let mut seed_files = Vec::new();
    let program = match inputs.app {
        App::Alpine => {
            if let Some(rust_analyzer) = inputs.rust_analyzer {
                rust_analyzer_env(&mut env, rust_analyzer);
                env.push((
                    "ALPINE_RUST_ANALYZER".to_owned(),
                    rust_analyzer.binary.as_os_str().to_owned(),
                ));
            }
            // Alpine opens exactly one path: the last document is the file.
            args.extend(
                inputs
                    .documents
                    .last()
                    .map(|path| path.as_os_str().to_owned()),
            );
            inputs.alpine_bundle.join("Contents/MacOS/alpine-editor")
        }
        App::Zed => {
            check_zed_home(inputs.home)?;
            if let Some(rust_analyzer) = inputs.rust_analyzer {
                rust_analyzer_env(&mut env, rust_analyzer);
            }
            env.push((
                "ZED_UPDATE_EXPLANATION".to_owned(),
                OsString::from("updates are disabled for the Alpine bench"),
            ));
            env.push((
                "ZED_SERVER_URL".to_owned(),
                OsString::from("http://127.0.0.1:9"),
            ));
            env.push((
                "ZED_WINDOW_SIZE".to_owned(),
                OsString::from(format!("{WINDOW_WIDTH},{FRAME_HEIGHT}")),
            ));
            let data = zed_data_dir(inputs.home);
            seed_files.push((
                data.join("config/settings.json"),
                zed_settings(inputs.rust_analyzer.map(|ra| ra.binary.as_path())),
            ));
            args.push(OsString::from("--user-data-dir"));
            args.push(data.into_os_string());
            args.extend(
                inputs
                    .documents
                    .iter()
                    .map(|path| path.as_os_str().to_owned()),
            );
            inputs.zed_app.join("Contents/MacOS/zed")
        }
        App::AppKit => {
            args.push(OsString::from("--width"));
            args.push(OsString::from(WINDOW_WIDTH.to_string()));
            args.push(OsString::from("--height"));
            args.push(OsString::from(WINDOW_HEIGHT.to_string()));
            args.extend(
                inputs
                    .documents
                    .last()
                    .map(|path| path.as_os_str().to_owned()),
            );
            inputs.reference_binary.to_path_buf()
        }
    };
    Ok(LaunchSpec {
        program,
        args,
        env,
        seed_files,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        App, DOCK_PATH, Identity, LaunchInputs, LaunchSpec, RustAnalyzer, check_zed_home,
        launch_spec, zed_settings,
    };
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};

    fn identity() -> Identity {
        Identity {
            user: "me".to_owned(),
            shell: "/bin/zsh".to_owned(),
            text_encoding: Some("0x1F5:0x0:0x0".to_owned()),
        }
    }

    fn spec(
        app: App,
        documents: &[PathBuf],
        ra: Option<&RustAnalyzer>,
    ) -> Result<LaunchSpec, String> {
        let identity = identity();
        launch_spec(&LaunchInputs {
            app,
            home: Path::new("/tmp/alpine-bench/r1-01/home"),
            tmpdir: Path::new("/tmp/alpine-bench/r1-01/tmp"),
            documents,
            identity: &identity,
            rust_analyzer: ra,
            alpine_bundle: Path::new("/Users/me/Applications/Alpine Editor.app"),
            zed_app: Path::new("/Applications/Zed.app"),
            reference_binary: Path::new("/bench/target/helpers/bench-reference-appkit"),
        })
    }

    fn env_value<'a>(spec: &'a LaunchSpec, key: &str) -> Option<&'a OsString> {
        spec.env
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value)
    }

    #[test]
    fn every_app_gets_the_same_dock_environment() -> Result<(), String> {
        let documents = vec![PathBuf::from("/fixtures/idle-200.txt")];
        for app in [App::Alpine, App::Zed, App::AppKit] {
            let spec = spec(app, &documents, None)?;
            assert_eq!(
                env_value(&spec, "HOME"),
                Some(&OsString::from("/tmp/alpine-bench/r1-01/home"))
            );
            assert_eq!(env_value(&spec, "PATH"), Some(&OsString::from(DOCK_PATH)));
            assert_eq!(
                env_value(&spec, "TMPDIR"),
                Some(&OsString::from("/tmp/alpine-bench/r1-01/tmp"))
            );
            assert_eq!(env_value(&spec, "USER"), Some(&OsString::from("me")));
            assert!(env_value(&spec, "RUSTUP_HOME").is_none(), "{}", app.name());
            assert_eq!(
                spec.args.last(),
                Some(&OsString::from("/fixtures/idle-200.txt"))
            );
        }
        Ok(())
    }

    #[test]
    fn alpine_runs_the_installed_bundle_binary_with_one_path() -> Result<(), String> {
        let documents = vec![PathBuf::from("/repo"), PathBuf::from("/repo/src/lib.rs")];
        let ra = RustAnalyzer {
            binary: PathBuf::from("/rustup/toolchains/1.97.1/bin/rust-analyzer"),
            rustup_home: PathBuf::from("/Users/me/.rustup"),
            cargo_home: PathBuf::from("/Users/me/.cargo"),
        };
        let spec = spec(App::Alpine, &documents, Some(&ra))?;
        assert_eq!(
            spec.program,
            PathBuf::from("/Users/me/Applications/Alpine Editor.app/Contents/MacOS/alpine-editor")
        );
        assert_eq!(spec.args, vec![OsString::from("/repo/src/lib.rs")]);
        assert_eq!(
            env_value(&spec, "ALPINE_RUST_ANALYZER"),
            Some(&OsString::from(
                "/rustup/toolchains/1.97.1/bin/rust-analyzer"
            ))
        );
        assert_eq!(
            env_value(&spec, "RUSTUP_HOME"),
            Some(&OsString::from("/Users/me/.rustup"))
        );
        assert!(spec.seed_files.is_empty());
        Ok(())
    }

    #[test]
    fn zed_is_isolated_by_home_data_dir_and_seeded_settings() -> Result<(), String> {
        let documents = vec![PathBuf::from("/repo"), PathBuf::from("/repo/src/lib.rs")];
        let spec = spec(App::Zed, &documents, None)?;
        assert_eq!(
            spec.program,
            PathBuf::from("/Applications/Zed.app/Contents/MacOS/zed")
        );
        assert_eq!(
            spec.args,
            vec![
                OsString::from("--user-data-dir"),
                OsString::from("/tmp/alpine-bench/r1-01/home/zed-data"),
                OsString::from("/repo"),
                OsString::from("/repo/src/lib.rs"),
            ]
        );
        assert!(env_value(&spec, "ZED_UPDATE_EXPLANATION").is_some());
        assert_eq!(
            env_value(&spec, "ZED_WINDOW_SIZE"),
            Some(&OsString::from("960,572"))
        );
        let (path, settings) = spec.seed_files.first().ok_or("no settings seeded")?;
        assert_eq!(
            path,
            &PathBuf::from("/tmp/alpine-bench/r1-01/home/zed-data/config/settings.json")
        );
        assert!(settings.contains("\"auto_update\": false"));
        assert!(settings.contains("\"cursor_blink\": false"));
        Ok(())
    }

    #[test]
    fn appkit_gets_alpines_window_size() -> Result<(), String> {
        let spec = spec(App::AppKit, &[PathBuf::from("/f.txt")], None)?;
        let args: Vec<String> = spec
            .args
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args, ["--width", "960", "--height", "540", "/f.txt"]);
        Ok(())
    }

    #[test]
    fn zed_settings_are_valid_minimal_json_and_escape_paths() {
        let plain = zed_settings(None);
        assert!(plain.starts_with("{\n") && plain.ends_with("}\n"));
        assert!(!plain.contains("lsp"));
        let with_ra = zed_settings(Some(Path::new("/odd \"dir\"/rust-analyzer")));
        assert!(with_ra.contains("\"path\": \"/odd \\\"dir\\\"/rust-analyzer\""));
        assert_eq!(with_ra.matches('{').count(), with_ra.matches('}').count());
    }

    #[test]
    fn long_homes_are_refused_for_zed() {
        assert!(
            check_zed_home(Path::new(
                "/tmp/alpine-bench/20261002T183005Z-idle-zed/01/home"
            ))
            .is_ok()
        );
        let long = format!("/Users/me/{}/home", "x".repeat(80));
        assert!(check_zed_home(Path::new(&long)).is_err());
    }

    #[test]
    fn a_probe_launch_opens_no_document() -> Result<(), String> {
        let spec = spec(App::Zed, &[], None)?;
        assert_eq!(spec.args.len(), 2, "only --user-data-dir and its path");
        Ok(())
    }

    #[test]
    fn app_names_round_trip() -> Result<(), String> {
        for app in [App::Alpine, App::Zed, App::AppKit] {
            assert_eq!(App::parse(app.name())?, app);
        }
        assert!(App::parse("vscode").is_err());
        Ok(())
    }
}
