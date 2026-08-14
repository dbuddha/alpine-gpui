use crate::{FrameLifecycle, LifecycleAction, READBACK_ROW_ALIGNMENT, ReadbackLayout};

#[kani::proof]
fn readback_rows_stay_aligned_and_bounded() {
    let width: u16 = kani::any();
    kani::assume(width > 0);

    let layout = ReadbackLayout::new(u32::from(width), 1);
    let maximum = ReadbackLayout::new(u32::from(u16::MAX), 1);
    assert!(layout.is_ok());
    assert!(maximum.is_ok());
    if let (Ok(layout), Ok(maximum)) = (layout, maximum) {
        assert!(layout.aligned_bytes_per_row() >= layout.compact_bytes_per_row());
        assert_eq!(layout.aligned_bytes_per_row() % READBACK_ROW_ALIGNMENT, 0);
        assert!(layout.aligned_bytes_per_row() <= maximum.aligned_bytes_per_row());
        assert!(layout.compact_bytes_per_row() <= maximum.compact_bytes_per_row());
    }
    kani::cover!(width == 1);
    kani::cover!(width == u16::MAX);
}

#[kani::proof]
fn readback_capacity_stays_bounded_at_maximum_row() {
    let height: u16 = kani::any();
    kani::assume(height > 0);

    let layout = ReadbackLayout::new(u32::from(u16::MAX), u32::from(height));
    assert!(layout.is_ok());
    if let Ok(layout) = layout {
        assert!(layout.buffer_len() >= layout.compact_len());
        assert_eq!(
            layout.compact_len(),
            layout.compact_bytes_per_row() * usize::from(height)
        );
        assert_eq!(
            layout.buffer_len(),
            layout.aligned_bytes_per_row() * usize::from(height)
        );
    }
    kani::cover!(height == 1);
    kani::cover!(height == u16::MAX);
}

#[kani::proof]
fn lifecycle_actions_preserve_invariants() {
    let mut lifecycle = FrameLifecycle::new();
    for _ in 0..6 {
        let choice: u8 = kani::any();
        let action = match choice % 8 {
            0 => LifecycleAction::BeginFrame,
            1 => LifecycleAction::Encode,
            2 => LifecycleAction::Submit,
            3 => LifecycleAction::Complete,
            4 => LifecycleAction::Fail,
            5 => LifecycleAction::CancelBeforeSubmit,
            6 => LifecycleAction::BeginShutdown,
            _ => LifecycleAction::StopAfterDrain,
        };
        let _ = lifecycle.apply(action);
        assert!(lifecycle.invariants_hold());
    }
    kani::cover!(lifecycle.frame() == crate::FrameState::Completed);
    kani::cover!(lifecycle.frame() == crate::FrameState::Failed);
    kani::cover!(lifecycle.frame() == crate::FrameState::Cancelled);
    kani::cover!(lifecycle.renderer() == crate::RendererState::Stopped);
}
