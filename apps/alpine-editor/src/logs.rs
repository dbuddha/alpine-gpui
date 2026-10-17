//! The per-user log directory shared by performance and crash logs.

use std::{ffi::OsString, fs, io, path::PathBuf};

/// Returns `$HOME/Library/Logs/Alpine Editor`, creating it when missing.
pub(crate) fn directory(home: Option<OsString>) -> io::Result<PathBuf> {
    let home = home
        .filter(|home| !home.is_empty())
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "HOME is not set"))?;
    let directory = PathBuf::from(home).join("Library/Logs/Alpine Editor");
    fs::create_dir_all(&directory)?;
    Ok(directory)
}
