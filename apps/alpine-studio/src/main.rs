//! Alpine Studio process entry point.

fn main() -> Result<(), alpine_studio::StudioError> {
    let mut paths = std::env::args_os().skip(1);
    let path = paths.next();
    if paths.next().is_some() {
        return Err(alpine_studio::StudioError::Usage);
    }

    path.map_or_else(
        || alpine_studio::run().map_err(alpine_studio::StudioError::from),
        alpine_studio::run_path,
    )
}
