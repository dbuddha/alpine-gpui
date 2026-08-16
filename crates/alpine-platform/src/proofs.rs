use crate::{
    FrameCompletionStatus, FrameOwnerGeneration, FrameSlotAdmission, FrameSlotLease, FrameSlotRing,
    FrameToken, PresentationAction, PresentationOutcome, PresentationRevision, PresentationState,
    SurfaceEpoch,
};

#[kani::proof]
fn presentation_actions_preserve_invariants_and_atomicity() {
    let mut state = PresentationState::new();
    for _ in 0..9 {
        let choice: u8 = kani::any();
        let token = state.active_token().unwrap_or(FrameToken {
            attempt: 1,
            revision: PresentationRevision::INITIAL,
            epoch: SurfaceEpoch::INITIAL,
        });
        let action = match choice % 15 {
            0 => PresentationAction::Invalidate,
            1 => PresentationAction::AdvanceSurfaceEpoch,
            2 => PresentationAction::SetVisible(kani::any()),
            3 => PresentationAction::SetSized(kani::any()),
            4 => PresentationAction::Resume,
            5 => PresentationAction::Prepare,
            6 => PresentationAction::BeginUpdate(token),
            7 => PresentationAction::DiscardStale(token),
            8 => PresentationAction::Submit(token),
            9 => PresentationAction::CallPresent(token),
            10 => PresentationAction::CompletePresentation(token),
            11 => PresentationAction::FailActive(token),
            12 => PresentationAction::CancelActive(token),
            13 => PresentationAction::BeginShutdown,
            _ => PresentationAction::StopAfterDrain,
        };
        let before = state;
        let result = state.apply(action);
        assert!(state.invariants_hold());
        if result.is_err() {
            assert_eq!(state, before);
        }
    }
    kani::cover!(state.outcome() == PresentationOutcome::Presented);
    kani::cover!(state.outcome() == PresentationOutcome::Superseded);
    kani::cover!(state.outcome() == PresentationOutcome::Cancelled);
    kani::cover!(state.outcome() == PresentationOutcome::Failed);
    kani::cover!(state.application() == crate::ApplicationState::Stopped);
}

#[kani::proof]
fn frame_slot_actions_preserve_bounded_unique_ownership() {
    let Some(generation) = FrameOwnerGeneration::new(1) else {
        return;
    };
    let mut ring = FrameSlotRing::new();
    let mut leases: [Option<FrameSlotLease>; 3] = [None; 3];
    for step in 0_u64..7 {
        let choice: u8 = kani::any();
        let slot = usize::from(choice % 3);
        match choice % 4 {
            0 => {
                let token = FrameToken {
                    attempt: step + 1,
                    revision: PresentationRevision(step),
                    epoch: SurfaceEpoch(step % 2),
                };
                if let Ok(FrameSlotAdmission::Acquired(lease)) = ring.acquire(token, generation) {
                    leases[usize::from(lease.slot().get())] = Some(lease);
                }
            }
            1 => {
                if let Some(lease) = leases[slot] {
                    let _ = ring.mark_submitted(lease);
                }
            }
            2 => {
                if let Some(lease) = leases[slot]
                    && ring
                        .complete(
                            lease,
                            FrameCompletionStatus::Completed,
                            generation,
                            PresentationRevision(step),
                            SurfaceEpoch(step % 2),
                        )
                        .is_ok()
                {
                    leases[slot] = None;
                }
            }
            _ => {
                if let Some(lease) = leases[slot]
                    && ring.cancel_encoding(lease).is_ok()
                {
                    leases[slot] = None;
                }
            }
        }
        assert!(ring.invariants_hold());
        assert!(ring.snapshot().occupied_slots() <= 3);
    }
    kani::cover!(ring.snapshot().peak_occupied_slots() == 3);
    kani::cover!(ring.snapshot().saturation_count() > 0);
    kani::cover!(ring.snapshot().completion_count() > 0);
}
