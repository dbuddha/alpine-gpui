use crate::{
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
        let action = match choice % 14 {
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
            12 => PresentationAction::BeginShutdown,
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
    kani::cover!(state.outcome() == PresentationOutcome::Failed);
    kani::cover!(state.application() == crate::ApplicationState::Stopped);
}
