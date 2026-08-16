//! Public contracts for three bounded asynchronous frame slots.

use alpine_platform::{
    FrameCompletionDisposition, FrameCompletionStatus, FrameOwnerGeneration, FrameSlotAdmission,
    FrameSlotRing, PresentationAction, PresentationEvent, PresentationState,
};

#[test]
fn public_three_slot_contract_bounds_and_reuses_completion_ownership()
-> Result<(), Box<dyn std::error::Error>> {
    let generation = FrameOwnerGeneration::new(1).ok_or("valid generation")?;
    let mut presentation = PresentationState::new();
    presentation.apply(PresentationAction::SetVisible(true))?;
    presentation.apply(PresentationAction::SetSized(true))?;
    let mut tokens = Vec::new();
    for _ in 0..4 {
        presentation.apply(PresentationAction::Invalidate)?;
        presentation.apply(PresentationAction::Resume)?;
        let prepared = presentation.apply(PresentationAction::Prepare)?;
        let PresentationEvent::Prepared(token) = prepared.event() else {
            return Err("missing frame token".into());
        };
        tokens.push(token);
        presentation.apply(PresentationAction::BeginUpdate(token))?;
        presentation.apply(PresentationAction::FailActive(token))?;
    }

    let mut slots = FrameSlotRing::new();
    let mut leases = Vec::new();
    for token in tokens.iter().copied().take(3) {
        let FrameSlotAdmission::Acquired(lease) = slots.acquire(token, generation)? else {
            return Err("early slot saturation".into());
        };
        slots.mark_submitted(lease)?;
        leases.push(lease);
    }
    assert_eq!(
        slots.acquire(tokens[3], generation)?,
        FrameSlotAdmission::Saturated
    );
    let released = slots.complete(
        leases[1],
        FrameCompletionStatus::Completed,
        generation,
        tokens[3].revision(),
        tokens[3].epoch(),
    )?;
    assert_eq!(
        released.disposition(),
        FrameCompletionDisposition::SupersededRevision
    );
    let FrameSlotAdmission::Acquired(reused) = slots.acquire(tokens[3], generation)? else {
        return Err("released slot was not reusable".into());
    };
    assert_eq!(reused.slot(), leases[1].slot());
    assert!(reused.sequence() > leases[1].sequence());
    assert_eq!(slots.snapshot().occupied_slots(), 3);
    assert_eq!(slots.snapshot().peak_occupied_slots(), 3);
    assert_eq!(slots.snapshot().saturation_count(), 1);
    assert!(slots.invariants_hold());
    Ok(())
}
