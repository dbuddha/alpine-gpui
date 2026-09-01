//! Exercises Task #273 structural AX evidence through the compiled CLI.

use std::{fs, path::Path, process::Command};

fn repository_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap_or_else(|| Path::new("."))
}

#[test]
fn validates_fixture_without_admitting_physical_evidence() {
    let binary = env!("CARGO_BIN_EXE_alpine-assurance");
    let bundle = "assurance/ax/v1/fixture";
    let fixture = Command::new(binary)
        .current_dir(repository_root())
        .args(["validate-ax-fixture", bundle])
        .output();
    assert!(fixture.is_ok());
    if let Ok(fixture) = fixture {
        assert!(fixture.status.success());
        let stdout = String::from_utf8_lossy(&fixture.stdout);
        assert!(stdout.contains("validated task #273 AX fixture"));
        assert!(stdout.contains("no physical or performance claim"));
    }

    for command in ["validate-ax-evidence", "ax-evidence-report"] {
        let physical = Command::new(binary)
            .current_dir(repository_root())
            .args([command, bundle])
            .output();
        assert!(physical.is_ok());
        if let Ok(physical) = physical {
            assert!(!physical.status.success());
            assert!(
                String::from_utf8_lossy(&physical.stderr)
                    .contains("physical AX commands reject fixture-only bundles")
            );
        }
    }
}

#[test]
fn rejects_tampered_fixture_through_the_binary_boundary() -> Result<(), String> {
    let root = repository_root();
    let source = root.join("assurance/ax/v1/fixture");
    let destination = root.join(format!("target/ax-cli-invalid-{}", std::process::id()));
    let _ = fs::remove_dir_all(&destination);
    fs::create_dir_all(&destination).map_err(|error| error.to_string())?;
    for entry in fs::read_dir(&source).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        fs::copy(entry.path(), destination.join(entry.file_name()))
            .map_err(|error| error.to_string())?;
    }
    fs::write(
        destination.join("tree.jsonl"),
        "{\"sequence\":1,\"tampered\":true}\n",
    )
    .map_err(|error| error.to_string())?;

    let output = Command::new(env!("CARGO_BIN_EXE_alpine-assurance"))
        .current_dir(root)
        .arg("validate-ax-fixture")
        .arg(&destination)
        .output()
        .map_err(|error| error.to_string())?;
    let removal = fs::remove_dir_all(&destination);
    assert!(removal.is_ok(), "cannot remove AX fixture: {removal:#?}");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("AX tree"));
    assert!(String::from_utf8_lossy(&output.stderr).contains("hash mismatch"));
    Ok(())
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
