//! One-time import of the pre-rename `Alpine Studio` support directory.
//!
//! The application was renamed from Alpine Studio to Alpine Editor. Settings,
//! sessions and recovery journals live under `Application Support`, so the
//! rename would otherwise orphan existing local state.
//!
//! Data already present in the new location always wins. The legacy directory
//! is never modified or removed, so a failed import leaves the original intact
//! and can be retried or inspected by hand.

use std::{
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
};

/// Directory name used before the rename.
pub(crate) const LEGACY_DIRECTORY: &str = "Alpine Studio";
/// Directory name used by this application.
pub(crate) const CURRENT_DIRECTORY: &str = "Alpine Editor";

/// Result of one migration attempt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum MigrationOutcome {
    /// No home directory was available, so no location could be resolved.
    MissingHome,
    /// Current data already exists and legacy data was left untouched.
    CurrentDataPresent,
    /// No legacy directory exists, so there was nothing to import.
    NoLegacyData,
    /// Legacy files were copied into the current location.
    Imported { files: usize },
    /// The import failed. The legacy directory is unchanged.
    Failed {
        operation: &'static str,
        kind: io::ErrorKind,
    },
}

fn support_root(home: &Path) -> PathBuf {
    home.join("Library").join("Application Support")
}

/// Imports the legacy support directory when the current one is absent.
pub(crate) fn migrate(home: Option<OsString>) -> MigrationOutcome {
    let Some(home) = home.filter(|value| !value.is_empty()).map(PathBuf::from) else {
        return MigrationOutcome::MissingHome;
    };
    let root = support_root(&home);
    let current = root.join(CURRENT_DIRECTORY);
    let legacy = root.join(LEGACY_DIRECTORY);

    if current.exists() {
        return MigrationOutcome::CurrentDataPresent;
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

    // Stage into a temporary sibling so a partial copy never becomes the
    // current directory. Only a complete import is published.
    let staging = root.join(".alpine-editor-import");
    let _ = fs::remove_dir_all(&staging);
    if let Err(error) = fs::create_dir_all(&staging) {
        return MigrationOutcome::Failed {
            operation: "create-staging",
            kind: error.kind(),
        };
    }

    let mut files = 0_usize;
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                let _ = fs::remove_dir_all(&staging);
                return MigrationOutcome::Failed {
                    operation: "read-entry",
                    kind: error.kind(),
                };
            }
        };
        let source = entry.path();
        if !source.is_file() {
            continue;
        }
        let Some(name) = source.file_name() else {
            continue;
        };
        if let Err(error) = fs::copy(&source, staging.join(name)) {
            let _ = fs::remove_dir_all(&staging);
            return MigrationOutcome::Failed {
                operation: "copy",
                kind: error.kind(),
            };
        }
        files = files.saturating_add(1);
    }

    if let Err(error) = fs::rename(&staging, &current) {
        let _ = fs::remove_dir_all(&staging);
        return MigrationOutcome::Failed {
            operation: "publish",
            kind: error.kind(),
        };
    }
    MigrationOutcome::Imported { files }
}

#[cfg(test)]
mod tests {
    use super::{CURRENT_DIRECTORY, LEGACY_DIRECTORY, MigrationOutcome, migrate, support_root};
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
        let legacy = support_root(&home.0).join(LEGACY_DIRECTORY);
        fs::create_dir_all(&legacy)?;
        fs::write(legacy.join("settings.json"), b"{\"font_size\":15}")?;
        fs::write(legacy.join("session-v1.bin"), b"ALPNSESS")?;

        assert_eq!(
            migrate(Some(home.os_string())),
            MigrationOutcome::Imported { files: 2 }
        );

        let current = support_root(&home.0).join(CURRENT_DIRECTORY);
        assert_eq!(
            fs::read(current.join("settings.json"))?,
            b"{\"font_size\":15}"
        );
        assert_eq!(fs::read(current.join("session-v1.bin"))?, b"ALPNSESS");
        // The legacy directory is evidence and must survive the import.
        assert!(legacy.join("settings.json").is_file());
        Ok(())
    }

    #[test]
    fn existing_current_data_always_wins() -> Result<(), Box<dyn std::error::Error>> {
        let home = DisposableHome::new("conflict")?;
        let root = support_root(&home.0);
        let legacy = root.join(LEGACY_DIRECTORY);
        let current = root.join(CURRENT_DIRECTORY);
        fs::create_dir_all(&legacy)?;
        fs::create_dir_all(&current)?;
        fs::write(legacy.join("settings.json"), b"legacy")?;
        fs::write(current.join("settings.json"), b"current")?;

        assert_eq!(
            migrate(Some(home.os_string())),
            MigrationOutcome::CurrentDataPresent
        );
        assert_eq!(fs::read(current.join("settings.json"))?, b"current");
        Ok(())
    }

    #[test]
    fn a_second_run_is_a_no_op() -> Result<(), Box<dyn std::error::Error>> {
        let home = DisposableHome::new("repeat")?;
        let legacy = support_root(&home.0).join(LEGACY_DIRECTORY);
        fs::create_dir_all(&legacy)?;
        fs::write(legacy.join("settings.json"), b"once")?;
        let first = migrate(Some(home.os_string()));
        let second = migrate(Some(home.os_string()));
        assert_eq!(first, MigrationOutcome::Imported { files: 1 });
        assert_eq!(second, MigrationOutcome::CurrentDataPresent);
        Ok(())
    }

    #[test]
    fn directories_inside_the_legacy_location_are_skipped() -> Result<(), Box<dyn std::error::Error>>
    {
        let home = DisposableHome::new("nested")?;
        let legacy = support_root(&home.0).join(LEGACY_DIRECTORY);
        fs::create_dir_all(legacy.join("nested"))?;
        fs::write(legacy.join("settings.json"), b"flat")?;
        assert_eq!(
            migrate(Some(home.os_string())),
            MigrationOutcome::Imported { files: 1 }
        );
        let current = support_root(&home.0).join(CURRENT_DIRECTORY);
        assert!(!current.join("nested").exists());
        Ok(())
    }
}
