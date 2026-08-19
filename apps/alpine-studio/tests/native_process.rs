//! Native process composition for Studio clipboard and dirty-close behavior.

#[cfg(all(alpine_native_validation, target_os = "macos", target_arch = "aarch64"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    qualify_shipping_executable()?;
    let evidence = alpine_studio::native_validation::qualify_clipboard_and_close_process()?;
    assert_eq!(evidence.input_events(), 7);
    assert_eq!(evidence.input_frames(), 5);
    assert!(evidence.persisted_bytes() > 1_000);
    assert_eq!(evidence.released_owner_classes(), 9);
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
        assert!((1..=4).contains(&submissions));
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
        Ok(())
    })();
    let cleanup = std::fs::remove_dir_all(root);
    match (result, cleanup) {
        (Err(error), _) => Err(error),
        (Ok(()), Err(error)) => Err(Box::new(error)),
        (Ok(()), Ok(())) => Ok(()),
    }
}

#[cfg(not(all(alpine_native_validation, target_os = "macos", target_arch = "aarch64")))]
fn main() {}
