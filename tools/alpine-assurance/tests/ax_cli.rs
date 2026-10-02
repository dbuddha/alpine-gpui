//! Exercises the AX capture command through the compiled CLI.

use std::{fs, path::Path, process::Command};

fn repository_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap_or_else(|| Path::new("."))
}

#[test]
fn capture_command_rejects_invalid_arguments_and_impossible_attachment() -> Result<(), String> {
    let root = repository_root();
    let binary = env!("CARGO_BIN_EXE_alpine-assurance");
    let output = root.join(format!("target/ax-cli-capture-{}", std::process::id()));
    let _ = fs::remove_dir_all(&output);
    let output_text = output.display().to_string();
    let missing = Command::new(binary)
        .current_dir(root)
        .arg("capture-ax-client")
        .output()
        .map_err(|error| error.to_string())?;
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stderr).contains(
        "capture-ax-client requires PID, generation, pre-action milliseconds, post-action milliseconds, and an output directory"
    ));
    let cases = [
        vec![
            "capture-ax-client",
            "not-a-pid",
            "1",
            "1",
            "1",
            &output_text,
        ],
        vec![
            "capture-ax-client",
            "1",
            "not-a-generation",
            "1",
            "1",
            &output_text,
        ],
        vec![
            "capture-ax-client",
            "1",
            "1",
            "not-a-duration",
            "1",
            &output_text,
        ],
        vec![
            "capture-ax-client",
            "1",
            "1",
            "1",
            "not-a-duration",
            &output_text,
        ],
        vec![
            "capture-ax-client",
            "2147483647",
            "1",
            "1",
            "1",
            &output_text,
        ],
    ];
    for arguments in cases {
        let result = Command::new(binary)
            .current_dir(root)
            .args(arguments)
            .output()
            .map_err(|error| error.to_string())?;
        assert!(!result.status.success());
        assert!(!result.stderr.is_empty());
    }
    assert!(!output.exists());
    Ok(())
}
