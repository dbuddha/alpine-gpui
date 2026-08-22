//! Native process composition for Studio clipboard and dirty-close behavior.

#[cfg(all(alpine_native_validation, target_os = "macos", target_arch = "aarch64"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let initial = alpine_studio::initial_scene()?;
    assert_eq!(initial.revision().get(), 1);
    assert!(!initial.operations().is_empty());
    assert!(!initial.clips().is_empty());
    assert!(!initial.quads().is_empty());
    assert!(!initial.glyphs().is_empty());
    qualify_shipping_executable()?;
    let evidence = alpine_studio::native_validation::qualify_clipboard_and_close_process()?;
    assert_eq!(evidence.input_events(), 12);
    assert_eq!(evidence.input_frames(), 10);
    assert!(evidence.persisted_bytes() > 1_000);
    assert_eq!(evidence.released_owner_classes(), 10);
    let tree = alpine_studio::native_validation::qualify_file_tree_process()?;
    assert_eq!(tree.keyboard_events(), 6);
    assert_eq!(tree.pointer_events(), 2);
    assert!(tree.worker_wakes() > 1);
    assert!(tree.admitted_frames() >= 9);
    assert_eq!(tree.persisted_bytes(), 5);
    assert_eq!(tree.released_owner_classes(), 9);
    let search = alpine_studio::native_validation::qualify_project_search_process()?;
    assert_eq!(search.keyboard_events(), 3);
    assert_eq!(search.ime_events(), 2);
    assert!(search.worker_wakes() >= 18);
    assert!(search.admitted_frames() >= 5);
    assert_eq!(search.matched_bytes(), 6);
    assert_eq!(search.released_owner_classes(), 9);
    Ok(())
}

#[cfg(all(alpine_native_validation, target_os = "macos", target_arch = "aarch64"))]
fn qualify_shipping_executable() -> Result<(), Box<dyn std::error::Error>> {
    use std::{
        ffi::OsStr,
        io::Read as _,
        process::{Command, Stdio},
        thread,
        time::{Duration, Instant, SystemTime, UNIX_EPOCH},
    };

    let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let root = std::env::temp_dir().join(format!(
        "alpine-studio-shipping-process-{}-{nonce}",
        std::process::id()
    ));
    let home = root.join("home");
    let path = root.join("document.rs");
    std::fs::create_dir_all(&home)?;
    std::fs::write(&path, "fn main() {}\n")?;
    let expected_evidence = match std::env::var_os("ALPINE_PRESENTATION_EVIDENCE_MODE") {
        None => "physical",
        Some(mode) if mode == OsStr::new("hosted-direct") => "hosted-direct",
        Some(mode) => {
            return Err(format!("unsupported presentation evidence mode: {mode:?}").into());
        }
    };

    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
        let mut child = Command::new(env!("CARGO_BIN_EXE_alpine-studio"))
            .arg(&path)
            .env(
                "ALPINE_STUDIO_NATIVE_PROCESS_SCENARIO",
                "production-single-window",
            )
            .env("HOME", &home)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let timeout = Duration::from_secs(8);
        let deadline = Instant::now() + timeout;
        let status = loop {
            if let Some(status) = child.try_wait()? {
                break status;
            }
            if Instant::now() >= deadline {
                child.kill()?;
                let status = child.wait()?;
                return Err(format!(
                    "shipping Alpine Studio exceeded {timeout:?} and was terminated with {status}"
                )
                .into());
            }
            thread::sleep(Duration::from_millis(10));
        };

        let mut stdout = String::new();
        let mut stderr = String::new();
        if let Some(mut pipe) = child.stdout.take() {
            pipe.read_to_string(&mut stdout)?;
        }
        if let Some(mut pipe) = child.stderr.take() {
            pipe.read_to_string(&mut stderr)?;
        }
        if !status.success() {
            return Err(format!(
                "shipping Alpine Studio failed with {status}; stdout={stdout:?}; stderr={stderr:?}"
            )
            .into());
        }
        let fields = stdout.split_whitespace().collect::<Vec<_>>();
        assert_eq!(fields.len(), 10);
        assert_eq!(fields[0], "alpine-native-journey");
        let submissions = fields[1]
            .strip_prefix("submissions=")
            .ok_or("missing submission evidence")?
            .parse::<u64>()?;
        let presented = fields[2]
            .strip_prefix("presented=")
            .ok_or("missing presentation evidence")?
            .parse::<u64>()?;
        let qualified = fields[3]
            .strip_prefix("qualified=")
            .ok_or("missing qualified-presentation evidence")?
            .parse::<u64>()?;
        let superseded = fields[4]
            .strip_prefix("superseded=")
            .ok_or("missing superseded-presentation evidence")?
            .parse::<u64>()?;
        let skipped = fields[5]
            .strip_prefix("skipped=")
            .ok_or("missing skipped-presentation evidence")?
            .parse::<u64>()?;
        let cancelled = fields[6]
            .strip_prefix("cancelled=")
            .ok_or("missing cancelled-presentation evidence")?
            .parse::<u64>()?;
        assert!(submissions >= 1);
        if expected_evidence == "physical" {
            assert!(submissions <= 4);
        }
        assert_eq!(presented + skipped + cancelled, submissions);
        assert_eq!(qualified + superseded, presented);
        assert!(qualified >= 1);
        assert_eq!(fields[7], "shutdown=true");
        assert_eq!(fields[8], "owners=9");
        assert_eq!(fields[9], format!("evidence={expected_evidence}"));
        assert!(stderr.lines().all(|line| {
            line.ends_with("Metal API Validation Enabled")
                || line.ends_with("Metal GPU Validation Enabled")
        }));
        qualify_recovery_launch_processes(&root, expected_evidence)?;
        Ok(())
    })();
    let cleanup = std::fs::remove_dir_all(root);
    match (result, cleanup) {
        (Err(error), _) => Err(error),
        (Ok(()), Err(error)) => Err(Box::new(error)),
        (Ok(()), Ok(())) => Ok(()),
    }
}

#[cfg(all(alpine_native_validation, target_os = "macos", target_arch = "aarch64"))]
fn qualify_recovery_launch_processes(
    root: &std::path::Path,
    expected_evidence: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let file_home = root.join("recovery-file-home");
    let recovered_file = root.join("recovered-file.rs");
    let requested_file = root.join("requested-file.rs");
    std::fs::create_dir_all(&file_home)?;
    std::fs::write(&recovered_file, "")?;
    std::fs::write(&requested_file, "requested\n")?;
    let file_journal = alpine_studio::native_validation::retain_pending_recovery_fixture(
        &file_home,
        &recovered_file,
        "unsaved recovery\n",
    )?;
    run_recovery_launch_process(
        &file_home,
        &requested_file,
        "production-recovery-file",
        expected_evidence,
    )?;
    alpine_studio::native_validation::qualify_retained_recovery_journal(
        &file_journal,
        &requested_file,
        false,
    )?;
    assert_eq!(
        std::fs::read_to_string(&recovered_file)?,
        "unsaved recovery\n"
    );

    let folder_home = root.join("recovery-folder-home");
    let recovered_folder_file = root.join("recovered-folder.rs");
    let requested_folder = root.join("requested-folder");
    std::fs::create_dir_all(&folder_home)?;
    std::fs::create_dir(&requested_folder)?;
    std::fs::write(&recovered_folder_file, "")?;
    std::fs::write(requested_folder.join("main.rs"), "fn main() {}\n")?;
    let folder_journal = alpine_studio::native_validation::retain_pending_recovery_fixture(
        &folder_home,
        &recovered_folder_file,
        "unsaved recovery\n",
    )?;
    run_recovery_launch_process(
        &folder_home,
        &requested_folder,
        "production-recovery-folder",
        expected_evidence,
    )?;
    alpine_studio::native_validation::qualify_retained_recovery_journal(
        &folder_journal,
        &requested_folder,
        true,
    )?;
    assert_eq!(
        std::fs::read_to_string(&recovered_folder_file)?,
        "unsaved recovery\n"
    );
    Ok(())
}

#[cfg(all(alpine_native_validation, target_os = "macos", target_arch = "aarch64"))]
fn run_recovery_launch_process(
    home: &std::path::Path,
    requested: &std::path::Path,
    scenario: &str,
    expected_evidence: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    use std::{
        io::Read as _,
        process::{Command, Stdio},
        thread,
        time::{Duration, Instant},
    };

    let mut child = Command::new(env!("CARGO_BIN_EXE_alpine-studio"))
        .arg(requested)
        .env("ALPINE_STUDIO_NATIVE_PROCESS_SCENARIO", scenario)
        .env("ALPINE_STUDIO_NATIVE_EXPECTED_PATH", requested)
        .env("HOME", home)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let timeout = Duration::from_secs(8);
    let deadline = Instant::now() + timeout;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            child.kill()?;
            let status = child.wait()?;
            return Err(format!(
                "recovery launch {scenario} exceeded {timeout:?} and was terminated with {status}"
            )
            .into());
        }
        thread::sleep(Duration::from_millis(10));
    };
    let mut stdout = String::new();
    let mut stderr = String::new();
    if let Some(mut pipe) = child.stdout.take() {
        pipe.read_to_string(&mut stdout)?;
    }
    if let Some(mut pipe) = child.stderr.take() {
        pipe.read_to_string(&mut stderr)?;
    }
    if !status.success() {
        return Err(format!(
            "recovery launch {scenario} failed with {status}; stdout={stdout:?}; stderr={stderr:?}"
        )
        .into());
    }
    assert!(stdout.contains("shutdown=true"));
    assert!(stdout.contains(&format!("evidence={expected_evidence}")));
    assert!(stderr.lines().all(|line| {
        line.ends_with("Metal API Validation Enabled")
            || line.ends_with("Metal GPU Validation Enabled")
    }));
    Ok(())
}

#[cfg(not(all(alpine_native_validation, target_os = "macos", target_arch = "aarch64")))]
fn main() {}
