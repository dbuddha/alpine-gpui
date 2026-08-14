//! Public integration contracts for the portable presentation state machine.

use alpine_platform::{
    DisplayLinkDirective, PresentationAction, PresentationEvent, PresentationOutcome,
    PresentationPhase, PresentationResource, PresentationState,
};

#[test]
fn public_contract_coalesces_and_presents_one_current_frame()
-> Result<(), Box<dyn std::error::Error>> {
    let mut state = PresentationState::new();
    state.apply(PresentationAction::SetVisible(true))?;
    state.apply(PresentationAction::SetSized(true))?;
    assert_eq!(
        state.apply(PresentationAction::Invalidate)?.display_link(),
        DisplayLinkDirective::Resume
    );
    state.apply(PresentationAction::Invalidate)?;
    state.apply(PresentationAction::Resume)?;
    let prepared = state.apply(PresentationAction::Prepare)?;
    let PresentationEvent::Prepared(token) = prepared.event() else {
        return Err("preparation did not return a frame token".into());
    };
    state.apply(PresentationAction::BeginUpdate(token))?;
    state.apply(PresentationAction::Submit(token))?;
    state.apply(PresentationAction::CallPresent(token))?;
    let completed = state.apply(PresentationAction::CompletePresentation(token))?;
    let PresentationEvent::Terminal(evidence) = completed.event() else {
        return Err("completion did not return terminal evidence".into());
    };

    assert_eq!(evidence.outcome(), PresentationOutcome::Presented);
    assert_eq!(evidence.frame_revision(), state.requested_revision());
    assert_eq!(evidence.requested_revision(), state.requested_revision());
    assert_eq!(evidence.frame_epoch(), state.surface_epoch());
    assert_eq!(state.phase(), PresentationPhase::Idle);
    assert_eq!(state.resource(), PresentationResource::Free);
    assert!(state.invariants_hold());
    Ok(())
}

#[test]
fn public_contract_drains_committed_shutdown() -> Result<(), Box<dyn std::error::Error>> {
    let mut state = PresentationState::new();
    state.apply(PresentationAction::SetVisible(true))?;
    state.apply(PresentationAction::SetSized(true))?;
    state.apply(PresentationAction::Invalidate)?;
    state.apply(PresentationAction::Resume)?;
    let prepared = state.apply(PresentationAction::Prepare)?;
    let PresentationEvent::Prepared(token) = prepared.event() else {
        return Err("preparation did not return a frame token".into());
    };
    state.apply(PresentationAction::BeginUpdate(token))?;
    state.apply(PresentationAction::Submit(token))?;
    assert_eq!(
        state.apply(PresentationAction::BeginShutdown)?.event(),
        PresentationEvent::ShutdownDraining
    );
    state.apply(PresentationAction::CallPresent(token))?;
    state.apply(PresentationAction::CompletePresentation(token))?;
    state.apply(PresentationAction::StopAfterDrain)?;

    assert!(state.invariants_hold());
    assert_eq!(state.resource(), PresentationResource::Free);
    Ok(())
}
