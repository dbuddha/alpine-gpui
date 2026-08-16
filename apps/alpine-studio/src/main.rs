//! Alpine Studio process entry point.

fn main() -> Result<(), alpine_runtime::RuntimeError> {
    alpine_studio::run()
}
