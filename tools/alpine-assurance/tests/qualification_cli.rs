//! Exercises qualification validation through the compiled CLI boundary.

use std::{fs, path::Path, process::Command};

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
fn decodes_and_renders_the_committed_scene_trace() {
    let output_path = repository_root().join("target/qualification-cli-reference.bgra");
    let invalid_output = repository_root().join("target/qualification-cli-output-directory");
    assert!(fs::create_dir_all(repository_root().join("target")).is_ok());
    assert!(fs::create_dir_all(&invalid_output).is_ok());
    let invalid_output_result = Command::new(env!("CARGO_BIN_EXE_alpine-assurance"))
        .current_dir(repository_root())
        .args([
            "render-scene-reference",
            "assurance/qualification/v1/scene.toml",
        ])
        .arg(&invalid_output)
        .output();
    assert!(invalid_output_result.is_ok());
    if let Ok(invalid_output_result) = invalid_output_result {
        assert!(!invalid_output_result.status.success());
        let stderr = String::from_utf8_lossy(&invalid_output_result.stderr);
        assert!(stderr.contains("cannot invalidate prior output"));
    }
    assert!(fs::write(&output_path, b"stale evidence").is_ok());
    let rejected_render = Command::new(env!("CARGO_BIN_EXE_alpine-assurance"))
        .current_dir(repository_root())
        .args([
            "render-scene-reference",
            "assurance/qualification/v1/unsupported-scene.toml",
        ])
        .arg(&output_path)
        .output();
    assert!(rejected_render.is_ok());
    if let Ok(rejected_render) = rejected_render {
        assert!(!rejected_render.status.success());
        assert!(!output_path.exists());
    }
    let validation = Command::new(env!("CARGO_BIN_EXE_alpine-assurance"))
        .current_dir(repository_root())
        .args([
            "validate-scene-trace",
            "assurance/qualification/v1/scene.toml",
        ])
        .output();
    assert!(validation.is_ok());
    if let Ok(validation) = validation {
        assert!(validation.status.success());
        let stdout = String::from_utf8_lossy(&validation.stdout);
        assert!(stdout.contains("with 3 operations and 8x4 reference pixels"));
    }

    let render = Command::new(env!("CARGO_BIN_EXE_alpine-assurance"))
        .current_dir(repository_root())
        .args([
            "render-scene-reference",
            "assurance/qualification/v1/scene.toml",
        ])
        .arg(&output_path)
        .output();
    assert!(render.is_ok());
    if let Ok(render) = render {
        assert!(render.status.success());
        let stdout = String::from_utf8_lossy(&render.stdout);
        assert!(stdout.contains("through cpu-oracle"));
        let bytes = fs::read(&output_path);
        assert!(bytes.is_ok());
        if let Ok(bytes) = bytes {
            assert_eq!(bytes.len(), 8 * 4 * 4);
            assert_eq!(&bytes[0..4], &[128, 128, 0, 255]);
            assert_eq!(&bytes[8..12], &[64, 128, 64, 255]);
            assert_eq!(&bytes[16..20], &[128, 0, 128, 255]);
            assert_eq!(&bytes[64..68], &[255, 0, 0, 255]);
        }
    }
}

#[test]
fn validates_and_renders_every_realistic_prepared_scene_fixture() {
    let binary = env!("CARGO_BIN_EXE_alpine-assurance");
    assert!(fs::create_dir_all(repository_root().join("target")).is_ok());
    for fixture in [
        "clipped-grid.toml",
        "glyph-grid.toml",
        "code-viewport.toml",
        "scroll-before.toml",
        "scroll-after.toml",
        "resize-before.toml",
        "resize-after.toml",
    ] {
        let manifest = format!("assurance/qualification/v2/{fixture}");
        let validation = Command::new(binary)
            .current_dir(repository_root())
            .args(["validate-scene-trace", &manifest])
            .output();
        assert!(validation.is_ok(), "{fixture}");
        if let Ok(validation) = validation {
            assert!(
                validation.status.success(),
                "{fixture}: {}",
                String::from_utf8_lossy(&validation.stderr)
            );
        }
        let output_path = repository_root()
            .join("target")
            .join(format!("{fixture}.bgra"));
        let render = Command::new(binary)
            .current_dir(repository_root())
            .args(["render-scene-reference", &manifest])
            .arg(&output_path)
            .output();
        assert!(render.is_ok(), "{fixture}");
        if let Ok(render) = render {
            assert!(
                render.status.success(),
                "{fixture}: {}",
                String::from_utf8_lossy(&render.stderr)
            );
            assert!(fs::metadata(output_path).is_ok_and(|metadata| metadata.len() > 0));
        }
    }
}

#[test]
fn validates_the_atlas_lifecycle_sequence_through_the_binary_boundary() {
    let output = Command::new(env!("CARGO_BIN_EXE_alpine-assurance"))
        .current_dir(repository_root())
        .args([
            "validate-trace-sequence",
            "assurance/qualification/sequences/atlas-lifecycle-v1.toml",
        ])
        .output();
    assert!(output.is_ok());
    if let Ok(output) = output {
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("5 visible steps, 2 renderer generations"));
        assert!(stdout.contains("24 atlas upload bytes"));
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
