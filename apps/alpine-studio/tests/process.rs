//! Process-boundary validation for unsupported hosted targets.

#[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
#[test]
fn executable_propagates_the_structured_platform_failure() -> Result<(), Box<dyn std::error::Error>>
{
    let executable = env!("CARGO_BIN_EXE_alpine-studio");
    for arguments in [Vec::new(), vec!["document.txt"], vec!["."]] {
        let output = std::process::Command::new(executable)
            .args(arguments)
            .output()?;
        assert!(!output.status.success());
        let stderr = String::from_utf8(output.stderr)?;
        assert!(stderr.contains("UnsupportedPlatform"));
    }

    let usage = std::process::Command::new(executable)
        .args(["first.txt", "second.txt"])
        .output()?;
    assert!(!usage.status.success());
    assert!(String::from_utf8(usage.stderr)?.contains("Usage"));
    Ok(())
}
