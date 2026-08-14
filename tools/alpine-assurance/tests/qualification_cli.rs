//! Exercises qualification validation through the compiled CLI boundary.

use std::{path::Path, process::Command};

fn repository_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap_or_else(|| Path::new("."))
}

#[test]
fn validates_and_reports_through_the_binary_boundary() {
    let binary = env!("CARGO_BIN_EXE_alpine-assurance");
    let validation = Command::new(binary)
        .current_dir(repository_root())
        .args([
            "validate-qualification",
            "assurance/qualification/v1/valid.toml",
        ])
        .output();
    assert!(validation.is_ok());
    if let Ok(validation) = validation {
        assert!(validation.status.success());
        let stdout = String::from_utf8_lossy(&validation.stdout);
        assert!(stdout.contains("validated qualification renderer-foundation-fixture"));
    }

    let report = Command::new(binary)
        .current_dir(repository_root())
        .args([
            "qualification-report",
            "assurance/qualification/v1/valid.toml",
        ])
        .output();
    assert!(report.is_ok());
    if let Ok(report) = report {
        assert!(report.status.success());
        let stdout = String::from_utf8_lossy(&report.stdout);
        assert!(stdout.contains("# Alpine qualification report"));
        assert!(stdout.contains("Comparison level: renderer-only"));
    }
}

#[test]
fn rejects_an_invalid_fixture_through_the_binary_boundary() {
    let output = Command::new(env!("CARGO_BIN_EXE_alpine-assurance"))
        .current_dir(repository_root())
        .args([
            "validate-qualification",
            "assurance/qualification/v1/performance-before-correctness.toml",
        ])
        .output();
    assert!(output.is_ok());
    if let Ok(output) = output {
        assert!(!output.status.success());
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("equivalence gate visual did not pass"));
    }
}
