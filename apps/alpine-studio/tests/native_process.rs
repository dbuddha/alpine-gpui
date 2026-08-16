//! Native process composition for Studio clipboard and dirty-close behavior.

#[cfg(all(alpine_native_validation, target_os = "macos", target_arch = "aarch64"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let evidence = alpine_studio::native_validation::qualify_clipboard_and_close_process()?;
    assert_eq!(evidence.input_events(), 7);
    assert_eq!(evidence.input_frames(), 6);
    assert!(evidence.persisted_bytes() > 1_000);
    assert_eq!(evidence.released_owner_classes(), 9);
    assert_eq!(evidence.release_order_violations(), 0);
    Ok(())
}

#[cfg(not(all(alpine_native_validation, target_os = "macos", target_arch = "aarch64")))]
fn main() {}
