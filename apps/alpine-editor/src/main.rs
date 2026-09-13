//! Alpine Editor process entry point.

fn main() -> Result<(), alpine_editor::EditorError> {
    let mut paths = std::env::args_os().skip(1);
    let path = paths.next();
    if paths.next().is_some() {
        return Err(alpine_editor::EditorError::Usage);
    }

    // Runs before settings or session are resolved so a renamed installation
    // keeps the local state written under the previous application name.
    alpine_editor::migrate_legacy_data();

    path.map_or_else(
        || alpine_editor::run().map_err(alpine_editor::EditorError::from),
        alpine_editor::run_path,
    )
}
