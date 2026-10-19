//! The per-user log directory and the file rules its crash and stall logs
//! share: private create-new files, a size cap, newest-10 retention.

use std::{
    ffi::OsString,
    fs::{self, File},
    io::{self, Read},
    path::{Path, PathBuf},
};

use crate::recovery;

pub(crate) const MAX_LOG_BYTES: usize = 65_536;
pub(crate) const KEPT_LOGS: usize = 10;
pub(crate) const TRUNCATED: &str = "\n[truncated]\n";
const MAX_IDENTITY_BYTES: u64 = 2_048;

/// Returns `$HOME/Library/Logs/Alpine Editor`, creating it when missing.
pub(crate) fn directory(home: Option<OsString>) -> io::Result<PathBuf> {
    let home = home
        .filter(|home| !home.is_empty())
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "HOME is not set"))?;
    let directory = PathBuf::from(home).join("Library/Logs/Alpine Editor");
    fs::create_dir_all(&directory)?;
    Ok(directory)
}

/// Creates `<kind>-<unix>-<pid>.log` with mode 0600, never replacing a file.
pub(crate) fn create(
    directory: &Path,
    kind: &str,
    unix: u64,
    pid: u32,
) -> io::Result<(PathBuf, File)> {
    let path = directory.join(format!("{kind}-{unix}-{pid}.log"));
    let file = recovery::create_new_private(&path)?;
    Ok((path, file))
}

/// The `(unix, pid)` stamp of a `<kind>-<unix>-<pid>.log` name.
pub(crate) fn stamp(name: &str, kind: &str) -> Option<(u64, u32)> {
    let (unix, pid) = name
        .strip_prefix(kind)?
        .strip_prefix('-')?
        .strip_suffix(".log")?
        .split_once('-')?;
    let digits = |text: &str| !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit());
    if !digits(unix) || !digits(pid) {
        return None;
    }
    Some((unix.parse().ok()?, pid.parse().ok()?))
}

/// Keeps `written` and the newest other logs of `kind`, `keep` in all, even
/// when a wrong clock stamps `written` oldest. Other files stay untouched.
pub(crate) fn prune(directory: &Path, kind: &str, keep: usize, written: &Path) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    let mut logs: Vec<_> = entries
        .flatten()
        .filter_map(|entry| {
            let stamp = stamp(entry.file_name().to_str()?, kind)?;
            let path = entry.path();
            (path != written).then_some((stamp, path))
        })
        .collect();
    logs.sort_unstable_by_key(|(stamp, _)| std::cmp::Reverse(*stamp));
    for (_, path) in logs.into_iter().skip(keep.saturating_sub(1)) {
        let _ = fs::remove_file(path);
    }
}

/// Cuts `text` to at most `limit` bytes, ending in the truncation marker.
pub(crate) fn cap(text: &mut String, limit: usize) {
    if text.len() > limit {
        text.truncate(char_floor(text, limit.saturating_sub(TRUNCATED.len())));
        text.push_str(TRUNCATED);
    }
}

pub(crate) fn truncated(text: &str, limit: usize) -> &str {
    &text[..char_floor(text, limit)]
}

fn char_floor(text: &str, limit: usize) -> usize {
    let mut end = limit.min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    end
}

pub(crate) fn bundle_identity_path() -> Option<PathBuf> {
    let executable = std::env::current_exe().ok()?;
    Some(
        executable
            .parent()?
            .parent()?
            .join("Resources/alpine-build-identity.toml"),
    )
}

pub(crate) fn read_identity(path: Option<&Path>) -> String {
    let mut bytes = Vec::new();
    let read = path.map(|path| {
        File::open(path).and_then(|file| file.take(MAX_IDENTITY_BYTES).read_to_end(&mut bytes))
    });
    match read {
        Some(Ok(length)) if length > 0 => String::from_utf8_lossy(&bytes).into_owned(),
        _ => String::from("unavailable\n"),
    }
}
