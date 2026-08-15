//! Process-boundary validation for unsupported hosted targets.

#[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
#[test]
fn executable_propagates_the_structured_platform_failure() -> Result<(), Box<dyn std::error::Error>>
{
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_alpine-studio")).output()?;
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr)?;
    assert!(stderr.contains("UnsupportedPlatform"));
    Ok(())
}
