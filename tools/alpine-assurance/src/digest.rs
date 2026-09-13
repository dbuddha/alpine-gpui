//! SHA-256 digests for evidence files.
//!
//! Shells out rather than vendoring a hash implementation, so the digest comes
//! from the same tool an operator would run by hand when checking an artifact.

use std::{path::Path, process::Command};

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn sha256_from_output(output: &[u8]) -> Option<String> {
    String::from_utf8_lossy(output)
        .split_ascii_whitespace()
        .map(str::to_ascii_lowercase)
        .find(|candidate| valid_sha256(candidate))
}

/// Returns the lowercase SHA-256 digest of `path`.
///
/// # Errors
///
/// Returns a message when no supported digest tool produced a valid hash.
pub(crate) fn calculate_sha256(path: &Path) -> Result<String, String> {
    for (program, arguments) in [("sha256sum", &[][..]), ("shasum", &["-a", "256"][..])] {
        let output = Command::new(program).args(arguments).arg(path).output();
        let Ok(output) = output else {
            continue;
        };
        if !output.status.success() {
            continue;
        }
        if let Some(digest) = sha256_from_output(&output.stdout) {
            return Ok(digest);
        }
    }

    #[cfg(windows)]
    {
        let output = Command::new("certutil")
            .arg("-hashfile")
            .arg(path)
            .arg("SHA256")
            .output();
        if let Ok(output) = output
            && output.status.success()
            && let Some(digest) = sha256_from_output(&output.stdout)
        {
            return Ok(digest);
        }
    }

    Err(format!(
        "cannot calculate SHA-256 for {}; sha256sum, shasum, or certutil is required",
        path.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::{calculate_sha256, sha256_from_output, valid_sha256};
    use std::fs;

    const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

    #[test]
    fn only_lowercase_hexadecimal_of_the_exact_length_is_accepted() {
        assert!(valid_sha256(EMPTY_SHA256));
        assert!(!valid_sha256(&EMPTY_SHA256[..63]));
        assert!(!valid_sha256(&EMPTY_SHA256.to_ascii_uppercase()));
        assert!(!valid_sha256(&format!("{}g", &EMPTY_SHA256[..63])));
    }

    #[test]
    fn the_digest_is_selected_from_surrounding_tool_output() {
        let line = format!("{EMPTY_SHA256}  /tmp/example\n");
        assert_eq!(
            sha256_from_output(line.as_bytes()).as_deref(),
            Some(EMPTY_SHA256)
        );
        assert_eq!(sha256_from_output(b"no digest here\n"), None);
    }

    #[test]
    fn an_empty_file_hashes_to_the_known_empty_digest() -> Result<(), Box<dyn std::error::Error>> {
        let path = std::env::temp_dir().join(format!("alpine-digest-{}", std::process::id()));
        fs::write(&path, b"")?;
        let digest = calculate_sha256(&path);
        let _ = fs::remove_file(&path);
        assert_eq!(digest.as_deref(), Ok(EMPTY_SHA256));
        Ok(())
    }
}
