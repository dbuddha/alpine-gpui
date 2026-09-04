//! Native process composition for Studio clipboard and dirty-close behavior.

#[cfg(all(alpine_native_validation, target_os = "macos", target_arch = "aarch64"))]
#[path = "fixtures/lsp_mock_server.rs"]
mod lsp_mock_server;

#[cfg(all(alpine_native_validation, target_os = "macos", target_arch = "aarch64"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::var_os("ALPINE_STUDIO_NATIVE_LSP_SERVER").is_some() {
        lsp_mock_server::main();
        return Ok(());
    }
    if std::env::var_os("ALPINE_STUDIO_NATIVE_ACCESSIBILITY_CHILD").is_some() {
        let omitted = std::env::var("ALPINE_STUDIO_NATIVE_ACCESSIBILITY_OMIT").ok();
        let result = alpine_studio::native_validation::qualify_studio_accessibility_process();
        if let Some(omitted) = omitted {
            return match result {
                Err(error) => {
                    let error = error.to_string();
                    alpine_studio::native_validation::validate_native_accessibility_omission_failure(
                        &omitted,
                        &error,
                    )
                    .map_err(|validation| std::io::Error::other(validation.to_string()))?;
                    println!("alpine-native-accessibility-omission-rejected={omitted}");
                    println!("alpine-native-accessibility-omission-error={error}");
                    Ok(())
                }
                Ok(_) => Err(format!(
                    "native accessibility journey qualified with required step {omitted:?} omitted"
                )
                .into()),
            };
        }
        let evidence = result?;
        assert_eq!(evidence.tree_actions(), 3);
        assert_eq!(evidence.tab_actions(), 2);
        assert_eq!(evidence.command_actions(), 2);
        assert_eq!(evidence.diagnostic_actions(), 1);
        assert_eq!(evidence.query_frames(), 0);
        assert!(evidence.maximum_action_frames() <= 1);
        assert!(evidence.persisted_bytes() > 32);
        assert_eq!(evidence.released_owner_classes(), 9);
        assert_eq!(evidence.mismatch_control_marker(), 0xA11C_E551);
        assert_eq!(evidence.dispatch_failure_control_marker(), 0xD15F_A11E);
        println!("alpine-native-accessibility-qualified");
        return Ok(());
    }
    if std::env::var_os("ALPINE_STUDIO_NATIVE_PROCESS_SCOPE").as_deref()
        == Some(std::ffi::OsStr::new("accessibility"))
    {
        qualify_accessibility_child()?;
        return Ok(());
    }
    let initial = alpine_studio::initial_scene()?;
    assert_eq!(initial.revision().get(), 1);
    assert!(!initial.operations().is_empty());
    assert!(!initial.clips().is_empty());
    assert!(!initial.quads().is_empty());
    assert!(!initial.glyphs().is_empty());
    if std::env::var_os("ALPINE_STUDIO_NATIVE_PROCESS_SCOPE").as_deref()
        == Some(std::ffi::OsStr::new("shipping"))
    {
        qualify_shipping_executable()?;
        return Ok(());
    }
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
    qualify_accessibility_child()?;
    Ok(())
}

#[cfg(all(alpine_native_validation, target_os = "macos", target_arch = "aarch64"))]
fn qualify_accessibility_child() -> Result<(), Box<dyn std::error::Error>> {
    use std::{
        io::Read as _,
        os::unix::fs::PermissionsExt as _,
        process::{Command, Stdio},
        thread,
        time::{Duration, Instant, SystemTime, UNIX_EPOCH},
    };

    let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let root = std::env::temp_dir().join(format!(
        "alpine-studio-native-accessibility-child-{}-{nonce}",
        std::process::id()
    ));
    let server = root.join("rust-analyzer-fixture");
    std::fs::create_dir_all(&root)?;
    std::fs::write(
        &server,
        "#!/bin/sh\nprintf 'wrapper-invoked:%s\\n' \"$$\" >> \"$ALPINE_STUDIO_NATIVE_LSP_TRACE\"\nexport ALPINE_STUDIO_NATIVE_LSP_SERVER=1\nexec \"$ALPINE_STUDIO_NATIVE_PROCESS_EXE\"\n",
    )?;
    std::fs::set_permissions(&server, std::fs::Permissions::from_mode(0o700))?;
    let executable = std::env::current_exe()?;
    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
        let run_child = |omitted: Option<&str>, home: &std::path::Path| {
            std::fs::create_dir_all(home)?;
            let language_trace = home.join("language-phases.log");
            let mut command = Command::new(&executable);
            command
                .env("ALPINE_STUDIO_NATIVE_ACCESSIBILITY_CHILD", "1")
                .env("ALPINE_STUDIO_NATIVE_PROCESS_EXE", &executable)
                .env("ALPINE_RUST_ANALYZER", &server)
                .env("ALPINE_STUDIO_NATIVE_LSP_TRACE", &language_trace)
                .env("HOME", home)
                .env_remove("ALPINE_STUDIO_NATIVE_ACCESSIBILITY_OMIT")
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            if let Some(omitted) = omitted {
                command.env("ALPINE_STUDIO_NATIVE_ACCESSIBILITY_OMIT", omitted);
            }
            let mut child = command.spawn()?;
            let timeout = Duration::from_secs(15);
            let deadline = Instant::now() + timeout;
            let status = loop {
                if let Some(status) = child.try_wait()? {
                    break status;
                }
                if Instant::now() >= deadline {
                    child.kill()?;
                    let status = child.wait()?;
                    let trace = read_language_trace(&language_trace);
                    return Err(format!(
                        "native Studio accessibility child exceeded {timeout:?} and ended with {status}; language_trace={trace:?}"
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
            let trace = read_language_trace(&language_trace);
            Ok::<_, Box<dyn std::error::Error>>((status, stdout, stderr, trace))
        };

        let evidence_mode = std::env::var("ALPINE_PRESENTATION_EVIDENCE_MODE")
            .unwrap_or_else(|_| String::from("physical"));
        let (first_status, first_stdout, first_stderr, first_trace) =
            run_child(None, &root.join("home-normal"))?;
        let first_trace_complete =
            alpine_studio::native_validation::validate_native_language_startup_trace(&first_trace)
                .is_ok();
        let (status, stdout, stderr, trace) =
            if alpine_studio::native_validation::hosted_terminal_stall_retry_allowed(
                &evidence_mode,
                0,
                first_status.success(),
                &first_stdout,
                &first_stderr,
                first_trace_complete,
            ) {
                eprintln!(
                    "alpine-hosted-native-command-stall-retry attempt=1 status={first_status} stderr={first_stderr:?} language_trace={first_trace:?}"
                );
                let retry = run_child(None, &root.join("home-normal-retry"))?;
                if !retry.0.success() {
                    return Err(format!(
                        "native Studio accessibility hosted retry failed; first=(status={first_status} stdout={first_stdout:?} stderr={first_stderr:?} language_trace={first_trace:?}) retry=(status={} stdout={:?} stderr={:?} language_trace={:?})",
                        retry.0, retry.1, retry.2, retry.3
                    )
                    .into());
                }
                retry
            } else {
                (first_status, first_stdout, first_stderr, first_trace)
            };
        if !status.success() {
            return Err(format!(
                "native Studio accessibility child failed with {status}; stdout={stdout:?}; stderr={stderr:?}; language_trace={trace:?}"
            )
            .into());
        }
        require_language_trace(&trace, "normal")?;
        assert_eq!(stdout.trim(), "alpine-native-accessibility-qualified");
        assert!(stderr.lines().all(|line| {
            line.ends_with("Metal API Validation Enabled")
                || line.ends_with("Metal GPU Validation Enabled")
        }));
        for omitted in ["open", "edit", "action", "save", "close"] {
            let (status, stdout, stderr, trace) =
                run_child(Some(omitted), &root.join(format!("home-{omitted}")))?;
            if !status.success() {
                return Err(format!(
                    "native Studio accessibility omission control {omitted:?} failed with {status}; stdout={stdout:?}; stderr={stderr:?}; language_trace={trace:?}"
                )
                .into());
            }
            if omitted == "open" {
                require_language_trace_prefix(&trace, omitted)?;
            } else {
                require_language_trace(&trace, omitted)?;
            }
            require_omission_output(&stdout, omitted)?;
            assert!(stderr.lines().all(|line| {
                line.ends_with("Metal API Validation Enabled")
                    || line.ends_with("Metal GPU Validation Enabled")
            }));
        }
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
fn require_omission_output(stdout: &str, omitted: &str) -> Result<(), Box<dyn std::error::Error>> {
    let mut lines = stdout.lines();
    let marker = format!("alpine-native-accessibility-omission-rejected={omitted}");
    if lines.next() != Some(marker.as_str()) {
        return Err(
            format!("native accessibility omission {omitted:?} lost its exact marker").into(),
        );
    }
    let error = lines
        .next()
        .and_then(|line| line.strip_prefix("alpine-native-accessibility-omission-error="))
        .ok_or_else(|| format!("native accessibility omission {omitted:?} lost its root error"))?;
    if lines.next().is_some() {
        return Err(
            format!("native accessibility omission {omitted:?} published trailing output").into(),
        );
    }
    alpine_studio::native_validation::validate_native_accessibility_omission_failure(omitted, error)
        .map_err(|validation| std::io::Error::other(validation.to_string()).into())
}

#[cfg(all(alpine_native_validation, target_os = "macos", target_arch = "aarch64"))]
fn read_language_trace(path: &std::path::Path) -> String {
    std::fs::read_to_string(path)
        .unwrap_or_else(|error| format!("<language trace unavailable: {error}>"))
}

#[cfg(all(alpine_native_validation, target_os = "macos", target_arch = "aarch64"))]
fn require_language_trace(trace: &str, scenario: &str) -> Result<(), Box<dyn std::error::Error>> {
    require_language_startup_trace(trace).map_err(|error| {
        format!("language trace mismatch for scenario {scenario:?}: {error}").into()
    })
}

#[cfg(all(alpine_native_validation, target_os = "macos", target_arch = "aarch64"))]
fn require_language_trace_prefix(
    trace: &str,
    scenario: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    alpine_studio::native_validation::validate_native_language_startup_prefix(trace).map_err(
        |error| format!("language trace prefix mismatch for scenario {scenario:?}: {error}").into(),
    )
}

#[cfg(all(alpine_native_validation, target_os = "macos", target_arch = "aarch64"))]
fn require_language_startup_trace(trace: &str) -> Result<(), Box<dyn std::error::Error>> {
    alpine_studio::native_validation::validate_native_language_startup_trace(trace).map_err(
        |error| -> Box<dyn std::error::Error> {
            Box::new(std::io::Error::other(error.to_string()))
        },
    )
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
    let diagnostic = root.join("internal-diagnostic.json");
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
            .env("ALPINE_STUDIO_DOGFOOD_OUTPUT", &diagnostic)
            .env("ALPINE_STUDIO_DOGFOOD_WORKLOAD_ID", "hosted-close")
            .env("ALPINE_STUDIO_DOGFOOD_REVISION", "a".repeat(40))
            .env(
                "ALPINE_STUDIO_DOGFOOD_CAPTURED_AT_UTC",
                "2026-08-30T12:00:00Z",
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
        assert_eq!(fields.len(), 11);
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
        let failed = fields[7]
            .strip_prefix("failed=")
            .ok_or("missing failed-presentation evidence")?
            .parse::<u64>()?;
        assert!(submissions >= 1);
        if expected_evidence == "physical" {
            assert!(submissions <= 4);
        }
        assert_eq!(presented + cancelled + failed, submissions);
        assert!(skipped <= failed);
        assert_eq!(qualified + superseded, presented);
        if expected_evidence == "hosted-direct" && qualified == 0 {
            assert!(failed >= 1);
        } else {
            assert!(qualified >= 1);
        }
        assert_eq!(fields[8], "shutdown=true");
        assert_eq!(fields[9], "owners=9");
        assert_eq!(fields[10], format!("evidence={expected_evidence}"));
        let diagnostic_bytes = std::fs::read(&diagnostic)?;
        assert!(diagnostic_bytes.len() <= 262_144);
        let diagnostic: serde_json::Value = serde_json::from_slice(&diagnostic_bytes)?;
        assert_eq!(
            diagnostic.get("schema").and_then(serde_json::Value::as_str),
            Some("alpine-studio-internal-diagnostic/v1")
        );
        assert_eq!(
            diagnostic
                .pointer("/lifecycle/clean_shutdown")
                .and_then(serde_json::Value::as_bool),
            Some(true)
        );
        assert_eq!(
            diagnostic
                .pointer("/surface/current_retained_bytes")
                .and_then(serde_json::Value::as_u64),
            Some(0)
        );
        assert!(
            diagnostic
                .get("omissions")
                .and_then(serde_json::Value::as_array)
                .is_some_and(|items| items.iter().any(|item| item == "process-samples"))
        );
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
    let file_recovery = alpine_studio::native_validation::qualify_retained_recovery_journal(
        &file_journal,
        &requested_file,
        false,
    )?;
    assert_eq!(file_recovery.document_count, 0);
    assert!(!file_recovery.workspace_root_matches);
    assert!(file_recovery.tab_path_matches);
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
    let folder_recovery = alpine_studio::native_validation::qualify_retained_recovery_journal(
        &folder_journal,
        &requested_folder,
        true,
    )?;
    assert_eq!(folder_recovery.document_count, 0);
    assert!(folder_recovery.workspace_root_matches);
    assert!(!folder_recovery.tab_path_matches);
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
