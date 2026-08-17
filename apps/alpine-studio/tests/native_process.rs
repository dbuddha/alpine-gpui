//! Native process composition for Studio clipboard and dirty-close behavior.

#[cfg(all(alpine_native_validation, target_os = "macos", target_arch = "aarch64"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let evidence = alpine_studio::native_validation::qualify_clipboard_and_close_process()?;
    assert_eq!(evidence.input_events(), 7);
    assert_eq!(evidence.input_frames(), 5);
    assert!(evidence.persisted_bytes() > 1_000);
    assert_eq!(evidence.released_owner_classes(), 9);
    let tree = alpine_studio::native_validation::qualify_file_tree_process()?;
    assert_eq!(tree.keyboard_events(), 6);
    assert_eq!(tree.pointer_events(), 2);
    assert!(tree.worker_wakes() > 1);
    assert!(tree.admitted_frames() >= 9);
    assert_eq!(tree.persisted_bytes(), 5);
    assert_eq!(tree.released_owner_classes(), 9);
    let search = alpine_studio::native_validation::qualify_project_search_process()?;
    assert_eq!(search.keyboard_events(), 3);
    assert_eq!(search.ime_events(), 2);
    assert!(search.worker_wakes() >= 18);
    assert!(search.admitted_frames() >= 5);
    assert_eq!(search.matched_bytes(), 6);
    assert_eq!(search.released_owner_classes(), 9);
    Ok(())
}

#[cfg(not(all(alpine_native_validation, target_os = "macos", target_arch = "aarch64")))]
fn main() {}
