//! Exercises A/A calibration validation through the compiled CLI boundary.

use std::{fs, path::Path, process::Command};

fn repository_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap_or_else(|| Path::new("."))
}

#[test]
fn validates_and_reports_fixture_without_a_performance_claim() {
    let binary = env!("CARGO_BIN_EXE_alpine-assurance");
    let validation = Command::new(binary)
        .current_dir(repository_root())
        .args([
            "validate-aa-calibration",
            "assurance/calibration/v1/valid.toml",
        ])
        .output();
    assert!(validation.is_ok());
    if let Ok(validation) = validation {
        assert!(validation.status.success());
        let stdout = String::from_utf8_lossy(&validation.stdout);
        assert!(stdout.contains("20 runs, 4 windows, and 40 pairs"));
        assert!(stdout.contains("no performance claim"));
    }

    let report = Command::new(binary)
        .current_dir(repository_root())
        .args([
            "aa-calibration-report",
            "assurance/calibration/v1/valid.toml",
        ])
        .output();
    assert!(report.is_ok());
    if let Ok(report) = report {
        assert!(report.status.success());
        let stdout = String::from_utf8_lossy(&report.stdout);
        assert!(stdout.contains("# Alpine renderer A/A calibration report"));
        assert!(stdout.contains("Status: fixture-only"));
        assert!(stdout.contains("Performance claim: none"));
    }
}

#[test]
fn rejects_revision_mismatch_through_the_binary_boundary() -> Result<(), String> {
    let root = repository_root();
    let source = fs::read_to_string(root.join("assurance/calibration/v1/valid.toml"))
        .map_err(|error| format!("cannot read fixture: {error}"))?;
    let invalid = source.replace(
        "candidate_revision = \"b567e8f29c3c6c6bcdf98c02bc1958e59f044157\"",
        "candidate_revision = \"2222222222222222222222222222222222222222\"",
    );
    let directory = root.join("target/calibration-cli-tests");
    fs::create_dir_all(&directory)
        .map_err(|error| format!("cannot create test directory: {error}"))?;
    let manifest = directory.join(format!("revision-mismatch-{}.toml", std::process::id()));
    fs::write(&manifest, invalid).map_err(|error| format!("cannot write test fixture: {error}"))?;

    let output = Command::new(env!("CARGO_BIN_EXE_alpine-assurance"))
        .current_dir(root)
        .arg("validate-aa-calibration")
        .arg(&manifest)
        .output()
        .map_err(|error| format!("cannot execute assurance tool: {error}"))?;
    let removal = fs::remove_file(&manifest);
    assert!(removal.is_ok(), "cannot remove test fixture: {removal:#?}");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("A/A base and candidate revisions must match"));
    Ok(())
}

#[test]
fn rejects_raw_artifact_hash_mismatch_through_the_binary_boundary() -> Result<(), String> {
    let root = repository_root();
    let source = fs::read_to_string(root.join("assurance/calibration/v1/valid.toml"))
        .map_err(|error| format!("cannot read fixture: {error}"))?;
    let invalid = source.replace(
        "raw_samples_sha256 = \"694f83c3ff56e26b2198a9534c34a01b8840065af0b7b32426b74f5702fe183e\"",
        "raw_samples_sha256 = \"000083c3ff56e26b2198a9534c34a01b8840065af0b7b32426b74f5702fe183e\"",
    );
    let directory = root.join("target/calibration-cli-tests");
    fs::create_dir_all(&directory)
        .map_err(|error| format!("cannot create test directory: {error}"))?;
    let manifest = directory.join(format!("hash-mismatch-{}.toml", std::process::id()));
    fs::write(&manifest, invalid).map_err(|error| format!("cannot write test fixture: {error}"))?;

    let output = Command::new(env!("CARGO_BIN_EXE_alpine-assurance"))
        .current_dir(root)
        .arg("validate-aa-calibration")
        .arg(&manifest)
        .output()
        .map_err(|error| format!("cannot execute assurance tool: {error}"))?;
    let removal = fs::remove_file(&manifest);
    assert!(removal.is_ok(), "cannot remove test fixture: {removal:#?}");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("raw sample artifact SHA-256 does not match the manifest"));
    Ok(())
}
