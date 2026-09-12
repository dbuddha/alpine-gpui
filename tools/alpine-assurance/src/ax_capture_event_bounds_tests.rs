//! Regression controls for the production AX capture accumulator.

use super::append_drain;
use alpine_ax_client::{AxEventBatch, AxGeneration, AxNotificationKind, AxObservedEvent};

fn batch(count: usize) -> AxEventBatch {
    let generation = match AxGeneration::new(1) {
        Ok(generation) => generation,
        Err(error) => unreachable!("valid generation: {error}"),
    };
    AxEventBatch {
        events: (0..count)
            .map(|_| AxObservedEvent {
                generation,
                kind: AxNotificationKind::Value,
                identifier: "editor".to_owned(),
                monotonic_ns: 1,
            })
            .collect(),
        omitted_events: 0,
        stale_events: 0,
    }
}

#[test]
fn successive_valid_drains_share_one_limit() {
    let mut accumulated = batch(0);
    assert_eq!(append_drain(&mut accumulated, batch(2), 3), Ok(()));
    assert_eq!(append_drain(&mut accumulated, batch(1), 3), Ok(()));
    let accepted = accumulated.clone();
    assert!(append_drain(&mut accumulated, batch(1), 3).is_err());
    assert_eq!(accumulated, accepted);
}

#[test]
fn oversized_drain_is_rejected_before_accumulator_mutation() {
    let mut accumulated = batch(1);
    let accepted = accumulated.clone();
    assert!(append_drain(&mut accumulated, batch(3), 3).is_err());
    assert_eq!(accumulated, accepted);
}

#[test]
fn second_phase_uses_only_the_unspent_capture_budget() {
    let mut before_action = batch(0);
    assert_eq!(append_drain(&mut before_action, batch(2), 3), Ok(()));
    let remaining = 3 - before_action.events.len();
    let mut after_action = batch(0);
    assert_eq!(append_drain(&mut after_action, batch(1), remaining), Ok(()));
    assert!(append_drain(&mut after_action, batch(1), remaining).is_err());
    assert_eq!(before_action.events.len() + after_action.events.len(), 3);
}

#[test]
fn exhausted_budget_accepts_no_new_events() {
    let mut accumulated = batch(0);
    assert_eq!(append_drain(&mut accumulated, batch(0), 0), Ok(()));
    assert!(append_drain(&mut accumulated, batch(1), 0).is_err());
    assert!(accumulated.events.is_empty());
}

#[test]
fn omissions_and_stale_events_fail_before_retaining_the_drain() {
    for (omitted_events, stale_events) in [(1, 0), (0, 1), (usize::MAX, usize::MAX)] {
        let mut accumulated = batch(1);
        let accepted = accumulated.clone();
        let mut next = batch(1);
        next.omitted_events = omitted_events;
        next.stale_events = stale_events;
        assert!(append_drain(&mut accumulated, next, 3).is_err());
        assert_eq!(accumulated, accepted);
    }
}
