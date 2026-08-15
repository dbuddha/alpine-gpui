//! Alpine Studio process entry point.

fn main() -> Result<(), alpine_platform_macos::SurfaceError> {
    alpine_studio::run()
}
