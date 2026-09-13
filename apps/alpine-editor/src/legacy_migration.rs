//! One-time import of the pre-rename `Alpine Studio` support directory.
//!
//! The application was renamed from Alpine Studio to Alpine Editor. Settings,
//! sessions and recovery journals live under `Application Support`, so the
//! rename would otherwise orphan existing local state.
//!
//! Completion is recorded by a marker file rather than inferred from the
//! current directory existing. An ordinary launch creates that directory as
//! soon as it writes a session, so directory existence cannot distinguish "this
//! was imported" from "an import failed and the app then saved". Without the
//! marker the import is retried, and it copies only files the current location
//! does not already have. The legacy directory is never modified, so a failure
//! leaves the original intact and inspectable.

use std::{
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
};

/// Directory name used before the rename.
pub(crate) const LEGACY_DIRECTORY: &str = "Alpine Studio";
/// Directory name used by this application.
pub(crate) const CURRENT_DIRECTORY: &str = "Alpine Editor";
/// Written into the current directory once an import completes.
pub(crate) const MARKER: &str = ".imported-from-alpine-studio";

/// Result of one migration attempt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum MigrationOutcome {
    /// No home directory was available, so no location could be resolved.
    MissingHome,
    /// An import already completed, recorded by the marker file.
    AlreadyImported,
    /// No legacy directory exists, so there is nothing to import.
    NoLegacyData,
    /// Import finished. `copied` excludes files the current location already had.
    Imported { copied: usize, skipped: usize },
    /// The import failed. The legacy directory is unchanged and no marker was
    /// written, so the next launch retries.
    Failed {
        operation: &'static str,
        kind: io::ErrorKind,
    },
}

fn support_root(home: &Path) -> PathBuf {
    home.join("Library").join("Application Support")
}

/// Imports the legacy support directory unless an import already completed.
pub(crate) fn migrate(home: Option<OsString>) -> MigrationOutcome {
    let Some(home) = home.filter(|value| !value.is_empty()).map(PathBuf::from) else {
        return MigrationOutcome::MissingHome;
    };
    let root = support_root(&home);
    let current = root.join(CURRENT_DIRECTORY);
    let legacy = root.join(LEGACY_DIRECTORY);

    if current.join(MARKER).exists() {
        return MigrationOutcome::AlreadyImported;
    }
    if !legacy.is_dir() {
        return MigrationOutcome::NoLegacyData;
    }

    let entries = match fs::read_dir(&legacy) {
        Ok(entries) => entries,
        Err(error) => {
            return MigrationOutcome::Failed {
                operation: "read-legacy",
                kind: error.kind(),
            };
        }
    };
    if let Err(error) = fs::create_dir_all(&current) {
        return MigrationOutcome::Failed {
            operation: "create-current",
            kind: error.kind(),
        };
    }

    let mut copied = 0_usize;
    let mut skipped = 0_usize;
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                return MigrationOutcome::Failed {
                    operation: "read-entry",
                    kind: error.kind(),
                };
            }
        };
        let source = entry.path();
        // Only regular files migrate. A symlink is not followed, so a hostile
        // or stale link in the legacy directory cannot write outside it.
        if !entry.file_type().is_ok_and(|kind| kind.is_file()) {
            continue;
        }
        let Some(name) = source.file_name() else {
            continue;
        };
        let destination = current.join(name);
        // Data already in the current location always wins.
        if destination.exists() {
            skipped = skipped.saturating_add(1);
            continue;
        }
        // Copy to a temporary name first so an interrupted copy never presents
        // itself as a complete file at the destination.
        let staged = current.join(format!(".{}.import", name.to_string_lossy()));
        if let Err(error) = fs::copy(&source, &staged) {
            let _ = fs::remove_file(&staged);
            return MigrationOutcome::Failed {
                operation: "copy",
                kind: error.kind(),
            };
        }
        if let Err(error) = fs::rename(&staged, &destination) {
            let _ = fs::remove_file(&staged);
            return MigrationOutcome::Failed {
                operation: "publish",
                kind: error.kind(),
            };
        }
        copied = copied.saturating_add(1);
    }

    if let Err(error) = fs::write(current.join(MARKER), b"alpine-studio\n") {
        return MigrationOutcome::Failed {
            operation: "mark",
            kind: error.kind(),
        };
    }
    MigrationOutcome::Imported { copied, skipped }
}

#[cfg(test)]
mod tests {
    use super::{
        CURRENT_DIRECTORY, LEGACY_DIRECTORY, MARKER, MigrationOutcome, migrate, support_root,
    };
    use std::{ffi::OsString, fs, path::PathBuf};

    /// Owns one disposable home so a failed assertion still removes the tree.
    struct DisposableHome(PathBuf);

    impl DisposableHome {
        fn new(label: &str) -> Result<Self, std::io::Error> {
            let path = std::env::temp_dir().join(format!(
                "alpine-legacy-migration-{label}-{}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path)?;
            Ok(Self(path))
        }

        fn os_string(&self) -> OsString {
            self.0.as_os_str().to_owned()
        }

        fn legacy(&self) -> PathBuf {
            support_root(&self.0).join(LEGACY_DIRECTORY)
        }

        fn current(&self) -> PathBuf {
            support_root(&self.0).join(CURRENT_DIRECTORY)
        }
    }

    impl Drop for DisposableHome {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn missing_home_is_reported_rather_than_guessed() {
        assert_eq!(migrate(None), MigrationOutcome::MissingHome);
        assert_eq!(
            migrate(Some(OsString::new())),
            MigrationOutcome::MissingHome
        );
    }

    #[test]
    fn absent_legacy_data_is_not_an_error() -> Result<(), Box<dyn std::error::Error>> {
        let home = DisposableHome::new("absent")?;
        assert_eq!(
            migrate(Some(home.os_string())),
            MigrationOutcome::NoLegacyData
        );
        Ok(())
    }

    #[test]
    fn legacy_files_are_imported_and_the_original_is_preserved()
    -> Result<(), Box<dyn std::error::Error>> {
        let home = DisposableHome::new("import")?;
        fs::create_dir_all(home.legacy())?;
        fs::write(home.legacy().join("settings.json"), b"{\"font_size\":15}")?;
        fs::write(home.legacy().join("session-v1.bin"), b"ALPNSESS")?;

        assert_eq!(
            migrate(Some(home.os_string())),
            MigrationOutcome::Imported {
                copied: 2,
                skipped: 0
            }
        );

        assert_eq!(
            fs::read(home.current().join("settings.json"))?,
            b"{\"font_size\":15}"
        );
        assert_eq!(
            fs::read(home.current().join("session-v1.bin"))?,
            b"ALPNSESS"
        );
        // The legacy directory is evidence and must survive the import.
        assert!(home.legacy().join("settings.json").is_file());
        assert!(home.current().join(MARKER).is_file());
        Ok(())
    }

    #[test]
    fn current_files_win_while_absent_ones_are_still_imported()
    -> Result<(), Box<dyn std::error::Error>> {
        let home = DisposableHome::new("conflict")?;
        fs::create_dir_all(home.legacy())?;
        fs::create_dir_all(home.current())?;
        fs::write(home.legacy().join("settings.json"), b"legacy")?;
        fs::write(home.legacy().join("session-v1.bin"), b"legacy-session")?;
        fs::write(home.current().join("settings.json"), b"current")?;

        assert_eq!(
            migrate(Some(home.os_string())),
            MigrationOutcome::Imported {
                copied: 1,
                skipped: 1
            }
        );
        assert_eq!(fs::read(home.current().join("settings.json"))?, b"current");
        assert_eq!(
            fs::read(home.current().join("session-v1.bin"))?,
            b"legacy-session"
        );
        Ok(())
    }

    /// The defect this guards: an ordinary launch creates the current directory
    /// as soon as it saves, so directory existence must not end the import.
    #[test]
    fn a_session_written_before_import_does_not_strand_legacy_data()
    -> Result<(), Box<dyn std::error::Error>> {
        let home = DisposableHome::new("stranded")?;
        fs::create_dir_all(home.legacy())?;
        fs::write(home.legacy().join("settings.json"), b"legacy")?;
        // The application saved a session without any import having happened.
        fs::create_dir_all(home.current())?;
        fs::write(home.current().join("session-v1.bin"), b"fresh")?;

        assert_eq!(
            migrate(Some(home.os_string())),
            MigrationOutcome::Imported {
                copied: 1,
                skipped: 0
            }
        );
        assert_eq!(fs::read(home.current().join("settings.json"))?, b"legacy");
        assert_eq!(fs::read(home.current().join("session-v1.bin"))?, b"fresh");
        Ok(())
    }

    #[test]
    fn a_second_run_is_a_no_op() -> Result<(), Box<dyn std::error::Error>> {
        let home = DisposableHome::new("repeat")?;
        fs::create_dir_all(home.legacy())?;
        fs::write(home.legacy().join("settings.json"), b"once")?;
        let first = migrate(Some(home.os_string()));
        let second = migrate(Some(home.os_string()));
        assert_eq!(
            first,
            MigrationOutcome::Imported {
                copied: 1,
                skipped: 0
            }
        );
        assert_eq!(second, MigrationOutcome::AlreadyImported);
        Ok(())
    }

    #[test]
    fn directories_and_symlinks_in_the_legacy_location_are_skipped()
    -> Result<(), Box<dyn std::error::Error>> {
        let home = DisposableHome::new("nested")?;
        fs::create_dir_all(home.legacy().join("nested"))?;
        fs::write(home.legacy().join("settings.json"), b"flat")?;
        let outside = home.0.join("outside.txt");
        fs::write(&outside, b"must not be copied")?;
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, home.legacy().join("link.txt"))?;

        assert_eq!(
            migrate(Some(home.os_string())),
            MigrationOutcome::Imported {
                copied: 1,
                skipped: 0
            }
        );
        assert!(!home.current().join("nested").exists());
        assert!(!home.current().join("link.txt").exists());
        Ok(())
    }

    #[test]
    fn no_staging_files_survive_a_successful_import() -> Result<(), Box<dyn std::error::Error>> {
        let home = DisposableHome::new("staging")?;
        fs::create_dir_all(home.legacy())?;
        fs::write(home.legacy().join("settings.json"), b"value")?;
        assert!(matches!(
            migrate(Some(home.os_string())),
            MigrationOutcome::Imported { .. }
        ));
        let leftovers: Vec<_> = fs::read_dir(home.current())?
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".import"))
            .collect();
        assert!(leftovers.is_empty(), "staging files left: {leftovers:?}");
        Ok(())
    }
}
