//! Exercises onscreen SDR artifact validation through the compiled CLI.

use std::{fs, path::Path, process::Command};

fn repository_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap_or_else(|| Path::new("."))
}

#[test]
fn validates_and_reports_complete_physical_fixture() {
    let binary = env!("CARGO_BIN_EXE_alpine-assurance");
    let bundle = "assurance/onscreen-sdr/v1/valid";
    let validation = Command::new(binary)
        .current_dir(repository_root())
        .args(["validate-onscreen-sdr", bundle])
        .output();
    assert!(validation.is_ok());
    if let Ok(validation) = validation {
        assert!(validation.status.success());
        assert!(
            String::from_utf8_lossy(&validation.stdout)
                .contains("validated task #234 onscreen SDR bundle")
        );
    }

    let report = Command::new(binary)
        .current_dir(repository_root())
        .args(["onscreen-sdr-report", bundle])
        .output();
    assert!(report.is_ok());
    if let Ok(report) = report {
        assert!(report.status.success());
        let stdout = String::from_utf8_lossy(&report.stdout);
        assert!(stdout.contains("Deliberate wrong-transfer control: rejected"));
        assert!(stdout.contains("Performance claim: none"));
    }
}

#[test]
fn rejects_a_synthetic_or_nondiscriminating_display_move() -> Result<(), String> {
    let root = repository_root();
    let source = root.join("assurance/onscreen-sdr/v1/valid");
    let destination = root.join(format!(
        "target/onscreen-sdr-invalid-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&destination);
    fs::create_dir_all(&destination).map_err(|error| error.to_string())?;
    for entry in fs::read_dir(&source).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        fs::copy(entry.path(), destination.join(entry.file_name()))
            .map_err(|error| error.to_string())?;
    }
    let moved = destination.join("display-move.toml");
    let invalid = fs::read_to_string(&moved)
        .map_err(|error| error.to_string())?
        .replace("display_id = 200", "display_id = 100")
        .replace("backing_scale = 1.0", "backing_scale = 2.0");
    fs::write(&moved, invalid).map_err(|error| error.to_string())?;
    let output = Command::new(env!("CARGO_BIN_EXE_alpine-assurance"))
        .current_dir(root)
        .arg("validate-onscreen-sdr")
        .arg(&destination)
        .output()
        .map_err(|error| error.to_string())?;
    let rejects_move = !output.status.success()
        && String::from_utf8_lossy(&output.stderr)
            .contains("display move must change physical display identity");

    let launch = destination.join("launch.toml");
    let unknown = fs::read_to_string(&launch).map_err(|error| error.to_string())?
        + "\nunapproved_field = true\n";
    fs::write(&launch, unknown).map_err(|error| error.to_string())?;
    let output = Command::new(env!("CARGO_BIN_EXE_alpine-assurance"))
        .current_dir(root)
        .arg("validate-onscreen-sdr")
        .arg(&destination)
        .output()
        .map_err(|error| error.to_string())?;
    let rejects_unknown = !output.status.success()
        && String::from_utf8_lossy(&output.stderr).contains("unknown field");
    let removal = fs::remove_dir_all(&destination);
    assert!(removal.is_ok());
    assert!(rejects_move);
    assert!(rejects_unknown);
    Ok(())
}
