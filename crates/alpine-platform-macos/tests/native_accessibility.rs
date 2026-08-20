//! Production `AppKit` accessibility selector qualification.

#[cfg(all(alpine_native_validation, target_os = "macos", target_arch = "aarch64"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::sync::{Arc, Mutex};

    use alpine_platform_macos::{
        AccessibilityAction, AccessibilityActionResult, AccessibilityError, AccessibilityNode,
        AccessibilityNodeId, AccessibilityPayload, AccessibilityRequest, AccessibilityResponse,
        AccessibilityRevision, AccessibilityRole, AccessibilitySelection, AccessibilitySnapshot,
        AccessibilityText, AccessibilityTextRange, ClipboardOperation, ClipboardText,
        ClipboardWrite, CloseDisposition, SurfaceDescriptor, SurfaceEvent, SurfaceResponse,
        native_validation,
    };

    #[derive(Clone)]
    struct State {
        revision: AccessibilityRevision,
        selection: AccessibilitySelection,
        text: String,
        snapshot_requests: usize,
    }

    fn snapshot(state: &State) -> Result<AccessibilitySnapshot, AccessibilityError> {
        AccessibilitySnapshot::new(
            state.revision,
            AccessibilityNodeId::new(1),
            vec![
                AccessibilityNode::new(
                    AccessibilityNodeId::new(1),
                    None,
                    AccessibilityRole::Window,
                    "Alpine Studio".into(),
                    false,
                    false,
                    false,
                )?,
                AccessibilityNode::new(
                    AccessibilityNodeId::new(2),
                    Some(AccessibilityNodeId::new(1)),
                    AccessibilityRole::TabList,
                    "Open editors".into(),
                    false,
                    false,
                    false,
                )?,
                AccessibilityNode::new(
                    AccessibilityNodeId::new(3),
                    Some(AccessibilityNodeId::new(2)),
                    AccessibilityRole::Tab,
                    "main.rs".into(),
                    false,
                    true,
                    false,
                )?,
                AccessibilityNode::new(
                    AccessibilityNodeId::new(4),
                    Some(AccessibilityNodeId::new(1)),
                    AccessibilityRole::CodeEditor,
                    "main.rs editor".into(),
                    true,
                    false,
                    false,
                )?,
                AccessibilityNode::new(
                    AccessibilityNodeId::new(5),
                    Some(AccessibilityNodeId::new(1)),
                    AccessibilityRole::Status,
                    "Ready".into(),
                    false,
                    false,
                    true,
                )?,
            ],
            state.selection,
            state.text.encode_utf16().count(),
            2,
            false,
        )
    }

    fn utf16_slice(text: &str, range: AccessibilityTextRange) -> Option<String> {
        let units = text.encode_utf16().collect::<Vec<_>>();
        let end = range.end_utf16().ok()?;
        String::from_utf16(units.get(range.start_utf16()..end)?).ok()
    }

    fn respond(
        state: &mut State,
        request: &AccessibilityRequest,
    ) -> Result<AccessibilityResponse, AccessibilityError> {
        let observed = state.revision;
        let result = match request.operation() {
            alpine_platform_macos::AccessibilityOperation::Snapshot => {
                state.snapshot_requests = state.snapshot_requests.saturating_add(1);
                snapshot(state).map(AccessibilityPayload::Snapshot)
            }
            alpine_platform_macos::AccessibilityOperation::Text { revision, range }
                if *revision == observed =>
            {
                utf16_slice(&state.text, *range)
                    .ok_or(AccessibilityError::TextMappingFailed)
                    .and_then(AccessibilityText::new)
                    .map(AccessibilityPayload::Text)
            }
            alpine_platform_macos::AccessibilityOperation::Selection { revision }
                if *revision == observed =>
            {
                Ok(AccessibilityPayload::Selection(state.selection))
            }
            alpine_platform_macos::AccessibilityOperation::LineForIndex {
                revision,
                index_utf16,
            } if *revision == observed && *index_utf16 <= state.text.encode_utf16().count() => {
                Ok(AccessibilityPayload::Line(usize::from(*index_utf16 > 4)))
            }
            alpine_platform_macos::AccessibilityOperation::RangeForLine { revision, line }
                if *revision == observed && *line < 2 =>
            {
                Ok(AccessibilityPayload::Range(if *line == 0 {
                    AccessibilityTextRange::new(0, 4)
                } else {
                    AccessibilityTextRange::new(5, 7)
                }))
            }
            alpine_platform_macos::AccessibilityOperation::RangeForIndex {
                revision,
                index_utf16,
            } if *revision == observed && *index_utf16 <= state.text.encode_utf16().count() => {
                Ok(AccessibilityPayload::Range(AccessibilityTextRange::new(
                    *index_utf16,
                    usize::from(*index_utf16 < state.text.encode_utf16().count()),
                )))
            }
            alpine_platform_macos::AccessibilityOperation::Action(
                AccessibilityAction::SetSelection {
                    revision,
                    selection,
                },
            ) if *revision == observed
                && selection.anchor_utf16() <= state.text.encode_utf16().count()
                && selection.head_utf16() <= state.text.encode_utf16().count() =>
            {
                state.selection = *selection;
                state.revision = AccessibilityRevision::new(
                    observed.document(),
                    observed.buffer().saturating_add(1),
                );
                Ok(AccessibilityPayload::Action(
                    AccessibilityActionResult::Applied,
                ))
            }
            _ => Err(AccessibilityError::StaleRevision {
                expected: request.revision().unwrap_or(observed),
                actual: observed,
            }),
        };
        match result {
            Ok(payload) => AccessibilityResponse::success(request, observed, payload),
            Err(error) => Ok(AccessibilityResponse::failure(request, observed, error)),
        }
    }

    fn surface_response(
        state: &mut State,
        request: &AccessibilityRequest,
        clipboard_write: Option<ClipboardWrite>,
    ) -> SurfaceResponse {
        respond(state, request).map_or_else(
            |_| SurfaceResponse::default(),
            |response| {
                SurfaceResponse::from_channels(
                    None,
                    clipboard_write,
                    CloseDisposition::NotRequested,
                    Some(response),
                )
            },
        )
    }

    let descriptor = SurfaceDescriptor::new("Alpine native accessibility", 96.0, 64.0, 1.0)?;
    let surface = native_validation::new_surface(&descriptor)?;
    surface.show()?;
    let state = Arc::new(Mutex::new(State {
        revision: AccessibilityRevision::new(7, 11),
        selection: AccessibilitySelection::new(0, 4),
        text: "zero\none two".into(),
        snapshot_requests: 0,
    }));
    let callback_state = Arc::clone(&state);
    let evidence = native_validation::replay_native_accessibility_path(&surface, move |event| {
        let SurfaceEvent::Accessibility { request, .. } = event else {
            return SurfaceResponse::default();
        };
        let mut state = match callback_state.lock() {
            Ok(state) => state,
            Err(_) => return SurfaceResponse::default(),
        };
        surface_response(&mut state, &request, None)
    })?;

    assert_eq!(evidence.root_children(), 1);
    assert!(evidence.stable_root_identity());
    assert_eq!(evidence.role(), "AXTextArea");
    assert_eq!(evidence.label(), "main.rs editor");
    assert_eq!(evidence.text_length_utf16(), 12);
    assert_eq!(evidence.selected_text(), "zero");
    assert_eq!(evidence.bounded_text(), "one");
    assert_eq!(evidence.selected_range(), AccessibilityTextRange::new(0, 4));
    assert_eq!(evidence.line_for_index(), 1);
    assert_eq!(evidence.range_for_line(), AccessibilityTextRange::new(5, 7));
    assert_eq!(
        evidence.range_for_index(),
        AccessibilityTextRange::new(3, 1)
    );
    assert!(evidence.bounded_text_selector_allowed());
    assert!(!evidence.geometry_selector_allowed());
    assert!(evidence.semantic_tree_valid());
    assert!(evidence.text_selector_scope_valid());
    assert_eq!(
        evidence.accepted_selection(),
        AccessibilityTextRange::new(2, 2)
    );
    assert!(evidence.stale_action_rejected());
    assert_eq!(evidence.peak_elements(), 5);
    assert_eq!(evidence.created_elements(), 5);
    assert_eq!(evidence.released_elements(), 5);
    assert_eq!(evidence.current_elements_after_revoke(), 0);
    assert_eq!(
        evidence.retained_slot_bytes_before_revoke(),
        evidence.peak_elements() * core::mem::size_of::<usize>()
    );
    assert_eq!(evidence.retained_slot_bytes_after_revoke(), 0);
    assert!(evidence.late_selector_rejected());
    assert_eq!(evidence.notification_counts(), [0, 0, 1, 1, 0]);
    assert_eq!(
        state
            .lock()
            .map_err(|_| "accessibility state lock poisoned")?
            .snapshot_requests,
        5
    );

    let rejected_descriptor =
        SurfaceDescriptor::new("Alpine rejected accessibility", 96.0, 64.0, 1.0)?;
    let rejected_surface = native_validation::new_surface(&rejected_descriptor)?;
    rejected_surface.show()?;
    let rejected_state = Arc::new(Mutex::new(State {
        revision: AccessibilityRevision::new(7, 11),
        selection: AccessibilitySelection::new(0, 4),
        text: "zero\none two".into(),
        snapshot_requests: 0,
    }));
    let callback_rejected_state = Arc::clone(&rejected_state);
    let forbidden_write = ClipboardWrite::new(
        ClipboardOperation::Copy,
        ClipboardText::new("must-not-write-during-accessibility")?,
    )?;
    let rejected =
        native_validation::replay_native_accessibility_path(&rejected_surface, move |event| {
            let SurfaceEvent::Accessibility { request, .. } = event else {
                return SurfaceResponse::default();
            };
            let mut state = match callback_rejected_state.lock() {
                Ok(state) => state,
                Err(_) => return SurfaceResponse::default(),
            };
            surface_response(&mut state, &request, Some(forbidden_write.clone()))
        });
    assert!(matches!(
        rejected,
        Err(alpine_platform_macos::SurfaceError::DriverUnavailable)
    ));
    let rejected_owner_evidence = native_validation::close_with_owner_evidence(rejected_surface)?;
    assert_eq!(rejected_owner_evidence.active(), [0; 10]);
    assert_eq!(rejected_owner_evidence.release_order_violations(), 0);

    let owner_evidence = native_validation::close_with_owner_evidence(surface)?;
    assert_eq!(owner_evidence.active(), [0; 10]);
    assert_eq!(owner_evidence.release_order_violations(), 0);
    Ok(())
}

#[cfg(not(all(alpine_native_validation, target_os = "macos", target_arch = "aarch64")))]
fn main() {}
