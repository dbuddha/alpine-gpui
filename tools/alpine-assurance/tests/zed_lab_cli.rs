//! Exercises accepted Zed lab evidence through the compiled CLI boundary.

use std::{path::Path, process::Command};

fn repository_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap_or_else(|| Path::new("."))
}

#[test]
fn validates_and_reports_composed_renderer_evidence() {
    let binary = env!("CARGO_BIN_EXE_alpine-assurance");
    let manifest = "assurance/lab/v1/task-61-solid-quad.toml";
    let validation = Command::new(binary)
        .current_dir(repository_root())
        .args(["validate-zed-lab-evidence", manifest])
        .output();
    assert!(validation.is_ok());
    if let Ok(validation) = validation {
        assert!(validation.status.success());
        let stdout = String::from_utf8_lossy(&validation.stdout);
        assert!(stdout.contains("task #61 with hosted offline GPUI and physical Direct Metal"));
    }

    let report = Command::new(binary)
        .current_dir(repository_root())
        .args(["zed-lab-evidence-report", manifest])
        .output();
    assert!(report.is_ok());
    if let Ok(report) = report {
        assert!(report.status.success());
        let stdout = String::from_utf8_lossy(&report.stdout);
        assert!(stdout.contains("Hosted offline evidence"));
        assert!(stdout.contains("retained for 90 days through"));
        assert!(stdout.contains("No timing or performance claim is present"));
    }
}
