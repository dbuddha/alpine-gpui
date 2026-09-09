//! Non-GUI controls for the exact stdio collector used by native Studio tests.
#![cfg(unix)]

#[path = "support/bounded_capture.rs"]
mod bounded_capture;

use std::{
    error::Error,
    fs, io,
    path::Path,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

fn shell(script: &str) -> Command {
    let mut command = Command::new("/bin/sh");
    command.args(["-c", script, "alpine-capture-fixture"]);
    command
}

#[test]
fn capture_preserves_exact_streams_and_nonzero_exit() -> Result<(), Box<dyn Error>> {
    for code in [0, 7] {
        let mut command = shell("printf 'alpha\\n'; printf 'beta\\n' >&2; exit \"$1\"");
        command.arg(code.to_string());
        let output = bounded_capture::run(&mut command, Duration::from_secs(5))?;
        assert_eq!(output.status.code(), Some(code));
        assert_eq!(output.stdout, b"alpha\n");
        assert_eq!(output.stderr, b"beta\n");
    }
    Ok(())
}

#[test]
fn capture_closes_owned_piped_stdin() -> Result<(), Box<dyn Error>> {
    let mut command = shell("read value || printf 'eof'");
    command.stdin(Stdio::piped());
    let output = bounded_capture::run(&mut command, Duration::from_secs(5))?;
    assert!(output.status.success());
    assert_eq!(output.stdout, b"eof");
    Ok(())
}

#[test]
fn capture_drains_both_large_streams_without_waiting_for_exit() -> Result<(), Box<dyn Error>> {
    let mut command = shell(
        "i=0; while [ \"$i\" -lt 8192 ]; do printf '0123456789abcdef\\n'; printf 'fedcba9876543210\\n' >&2; i=$((i+1)); done",
    );
    let output = bounded_capture::run(&mut command, Duration::from_secs(5))?;
    assert!(output.status.success());
    assert_eq!(output.stdout, b"0123456789abcdef\n".repeat(8192));
    assert_eq!(output.stderr, b"fedcba9876543210\n".repeat(8192));
    Ok(())
}

#[test]
fn capture_accepts_exact_limit_then_observes_eof() -> Result<(), Box<dyn Error>> {
    let mut command =
        shell("i=0; while [ \"$i\" -lt 256 ]; do printf '%01024d' 0; i=$((i+1)); done");
    let output = bounded_capture::run(&mut command, Duration::from_secs(5))?;
    assert!(output.status.success());
    assert_eq!(output.stdout.len(), bounded_capture::OUTPUT_LIMIT);
    assert!(output.stdout.iter().all(|byte| *byte == b'0'));
    assert!(output.stderr.is_empty());
    Ok(())
}

#[test]
fn capture_rejects_excess_bytes_on_either_stream() {
    for script in [
        "while :; do printf '%01024d' 0; done",
        "while :; do printf '%01024d' 0 >&2; done",
    ] {
        let result = bounded_capture::run(&mut shell(script), Duration::from_secs(5));
        assert!(matches!(result, Err(error) if error.kind() == io::ErrorKind::InvalidData));
    }
}

#[test]
fn capture_preserves_invalid_utf8_for_strict_caller_rejection() -> Result<(), Box<dyn Error>> {
    let output = bounded_capture::run(&mut shell("printf '\\377'"), Duration::from_secs(5))?;
    assert!(output.status.success());
    assert_eq!(output.stdout, [255]);
    assert!(String::from_utf8(output.stdout).is_err());
    Ok(())
}

fn with_root(run: impl FnOnce(&Path) -> Result<(), Box<dyn Error>>) -> Result<(), Box<dyn Error>> {
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let root = std::env::temp_dir().join(format!(
        "alpine-native-capture-{}-{nonce}",
        std::process::id(),
    ));
    fs::create_dir(&root)?;
    let result = run(&root);
    if result.is_ok() {
        fs::remove_dir_all(&root)?;
    } else {
        eprintln!("retained native capture fixture: {}", root.display());
    }
    result
}

fn fixture_pid(path: &Path) -> Result<u32, Box<dyn Error>> {
    let pid = fs::read_to_string(path)?.trim().parse::<u32>()?;
    if pid <= 1 || pid == std::process::id() {
        return Err("invalid capture-owned fixture PID".into());
    }
    Ok(pid)
}

fn fixture_pid_exists(pid: u32) -> io::Result<bool> {
    // Signal zero only observes the PID recorded by this disposable fixture.
    Command::new("/bin/kill")
        .args(["-0", &pid.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
}

fn await_fixture_exit(pid: u32) -> Result<(), Box<dyn Error>> {
    let deadline = Instant::now() + Duration::from_secs(4);
    while fixture_pid_exists(pid)? {
        if Instant::now() >= deadline {
            return Err(format!("capture fixture {pid} remained observable").into());
        }
        thread::sleep(Duration::from_millis(10));
    }
    Ok(())
}

#[test]
fn capture_timeout_reaps_its_owned_direct_child() -> Result<(), Box<dyn Error>> {
    with_root(|root| {
        let pid_path = root.join("child.pid");
        let mut command = shell("printf '%s\\n' \"$$\" > \"$1\"; exec sleep 2");
        command.arg(&pid_path);
        let result = bounded_capture::run(&mut command, Duration::from_secs(1));
        let pid = fixture_pid(&pid_path)?;
        let pid_observable_after_return = fixture_pid_exists(pid)?;
        // A broken cleanup control still has an independent finite backstop.
        let fixture_exit = await_fixture_exit(pid);
        // Assert the immediate reap observation before propagating an eventual
        // exit error: a dropped Child can remain an unreaped PID without still running.
        assert!(
            !pid_observable_after_return,
            "capture returned before owned-child reap",
        );
        fixture_exit?;
        assert!(matches!(result, Err(error) if error.kind() == io::ErrorKind::TimedOut));
        Ok(())
    })
}

fn descendant_case(stdout_held: bool) -> Result<(), Box<dyn Error>> {
    with_root(|root| {
        let pid_path = root.join("descendant.pid");
        let script = if stdout_held {
            "exec 2>/dev/null; ( sleep 2 ) & printf '%s\\n' \"$!\" > \"$1\"; exit 0"
        } else {
            "exec 1>/dev/null; ( sleep 2 ) & printf '%s\\n' \"$!\" > \"$1\"; exit 0"
        };
        let mut command = shell(script);
        command.arg(&pid_path);
        let result = bounded_capture::run(&mut command, Duration::from_secs(1));
        await_fixture_exit(fixture_pid(&pid_path)?)?;
        let error = result.err().ok_or("descendant-held output was accepted")?;
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        let failure = error
            .get_ref()
            .and_then(|source| source.downcast_ref::<bounded_capture::CaptureFailure>())
            .ok_or("missing structured capture failure")?;
        assert!(failure.observed_exit.is_some_and(|status| status.success()));
        assert_eq!(failure.stdout_eof, !stdout_held);
        assert_eq!(failure.stderr_eof, stdout_held);
        assert!(failure.cleanup_error.is_none());
        Ok(())
    })
}

#[test]
fn capture_deadline_includes_descendant_held_stdout() -> Result<(), Box<dyn Error>> {
    descendant_case(true)
}

#[test]
fn capture_deadline_includes_descendant_held_stderr() -> Result<(), Box<dyn Error>> {
    descendant_case(false)
}

#[test]
fn capture_rejects_zero_deadline_and_reports_spawn_failure() {
    let zero = bounded_capture::run(&mut shell("exit 0"), Duration::ZERO);
    assert!(matches!(zero, Err(error) if error.kind() == io::ErrorKind::InvalidInput));
    let mut missing = Command::new("/dev/null/alpine-missing-capture-program");
    assert!(bounded_capture::run(&mut missing, Duration::from_secs(1)).is_err());
}
