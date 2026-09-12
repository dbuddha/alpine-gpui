//! Native `AppKit` responder and text-input qualification.

#[cfg(all(alpine_native_validation, target_os = "macos", target_arch = "aarch64"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::{
        sync::{Arc, Mutex},
        time::Duration,
    };

    use alpine_platform_macos::{
        ImeEvent, InputEpoch, KeyState, Modifiers, PointerAction, PointerButton, ScrollPhase,
        SurfaceDescriptor, SurfaceEvent, SurfaceResponse, native_validation,
    };

    fn validate_startup_focus_publication(
        descriptor: &SurfaceDescriptor,
        input_epoch: InputEpoch,
        focused: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let surface = native_validation::new_surface(descriptor)?;
        surface.show()?;
        native_validation::set_input_focus_state(&surface, input_epoch, focused);
        let received = Arc::new(Mutex::new(Vec::new()));
        let callback_received = Arc::clone(&received);
        native_validation::arm_window_close(&surface, Duration::from_millis(500));
        let timeout = native_validation::arm_run_timeout(&surface, Duration::from_secs(2));
        let run_result = surface.run_with_event_handler(move |event| {
            if let Ok(mut received) = callback_received.lock() {
                received.push(event);
            }
            SurfaceResponse::default()
        });
        timeout.cancel();
        if timeout.expired() {
            return Err(
                format!("startup focus run timed out for {input_epoch:?}/{focused}").into(),
            );
        }
        run_result.map_err(|error| {
            format!("startup focus run failed for {input_epoch:?}/{focused}: {error}")
        })?;

        let received = received
            .lock()
            .map_err(|_| "startup focus receiver poisoned")?;
        assert!(matches!(
            received.first(),
            Some(SurfaceEvent::Focus {
                input_epoch: actual_epoch,
                focused: actual_focus,
                ..
            }) if *actual_epoch == input_epoch && *actual_focus == focused
        ));
        assert!(matches!(received.get(1), Some(SurfaceEvent::Wake { .. })));
        Ok(())
    }

    let descriptor = SurfaceDescriptor::new("Alpine native input", 96.0, 64.0, 1.0)?;
    let surface = native_validation::new_surface(&descriptor)?;
    surface.show()?;

    let received = Arc::new(Mutex::new(Vec::new()));
    let callback_received = Arc::clone(&received);
    // AppKit now reads a real text owner before interpreting a key. Model the
    // empty editor for this transport fixture; Studio tests own document edits.
    use alpine_platform_macos::{
        AccessibilityBounds, AccessibilityNode, AccessibilityNodeId, AccessibilityOperation,
        AccessibilityPayload, AccessibilityResponse, AccessibilityRevision, AccessibilityRole,
        AccessibilitySelection, AccessibilitySnapshot, AccessibilityText,
    };
    let root = AccessibilityNodeId::new(1);
    let snapshot = AccessibilitySnapshot::new(
        AccessibilityRevision::new(1, 1),
        root,
        vec![
            AccessibilityNode::new(
                root,
                None,
                AccessibilityRole::Window,
                "Input fixture".into(),
                false,
                false,
                false,
            )?,
            AccessibilityNode::new(
                AccessibilityNodeId::new(2),
                Some(root),
                AccessibilityRole::CodeEditor,
                "Editor".into(),
                true,
                false,
                false,
            )?,
        ],
        AccessibilitySelection::new(0, 0),
        0,
        1,
        false,
    )?;
    let mut composing = false;
    native_validation::replay_native_input_path(&surface, move |event| {
        let response = match &event {
            SurfaceEvent::Accessibility { request, .. } => {
                let payload = match request.operation() {
                    AccessibilityOperation::Snapshot => Some(AccessibilityPayload::Snapshot(
                        snapshot.clone().with_editor_composition(composing),
                    )),
                    AccessibilityOperation::Text { range, .. }
                        if range.start_utf16() == 0 && range.length_utf16() == 0 =>
                    {
                        Some(AccessibilityPayload::Text(
                            AccessibilityText::new("").expect("empty text"),
                        ))
                    }
                    AccessibilityOperation::FirstRectForRange { range, .. }
                        if range.start_utf16() == 0 && range.length_utf16() == 0 =>
                    {
                        Some(AccessibilityPayload::TextGeometry {
                            range: *range,
                            bounds: AccessibilityBounds::new(0.0, 0.0, 0.0, 16.0).expect("caret"),
                        })
                    }
                    _ => None,
                };
                payload
                    .and_then(|payload| {
                        AccessibilityResponse::success(request, snapshot.revision(), payload).ok()
                    })
                    .map_or_else(SurfaceResponse::default, |response| {
                        SurfaceResponse::from_channels(
                            None,
                            None,
                            alpine_platform_macos::CloseDisposition::NotRequested,
                            Some(response),
                        )
                    })
            }
            SurfaceEvent::Ime { event, .. } => {
                composing = matches!(event, ImeEvent::Started | ImeEvent::Updated { .. });
                SurfaceResponse::default()
            }
            _ => SurfaceResponse::default(),
        };
        if let Ok(mut received) = callback_received.lock() {
            received.push(event);
        }
        response
    })
    .map_err(|error| format!("native input replay failed: {error}"))?;

    let received = received
        .lock()
        .map_err(|_| "native input receiver poisoned")?;
    assert!(
        received
            .iter()
            .any(|event| matches!(event, SurfaceEvent::Accessibility { .. }))
    );
    // Metadata requests have their own AX contract; retain the exact input journey.
    let received: Vec<_> = received
        .iter()
        .filter(|event| !matches!(event, SurfaceEvent::Accessibility { .. }))
        .collect();
    assert_eq!(received.len(), 12, "native input events: {received:?}");
    assert!(matches!(
        &received[0],
        SurfaceEvent::Keyboard {
            state: KeyState::Down,
            physical_key: 0,
            logical_key,
            modifiers,
            repeat: false,
            ..
        } if logical_key.as_ref() == "a" && modifiers.bits() == Modifiers::SHIFT
    ));
    assert!(matches!(
        &received[1],
        SurfaceEvent::Ime {
            input_epoch: InputEpoch::INITIAL,
            event: ImeEvent::Committed(text),
            ..
        } if text.as_ref() == "A"
    ));
    assert!(matches!(
        &received[2],
        SurfaceEvent::Ime {
            input_epoch: InputEpoch::INITIAL,
            event: ImeEvent::Started,
            ..
        }
    ));
    assert!(matches!(
        &received[3],
        SurfaceEvent::Ime {
            input_epoch: InputEpoch::INITIAL,
            event: ImeEvent::Updated {
                text,
                selected_start_utf16: 1,
                selected_length_utf16: 1,
            },
            ..
        } if text.as_ref() == "漢字"
    ));
    assert!(matches!(
        &received[4],
        SurfaceEvent::Ime {
            input_epoch: InputEpoch::INITIAL,
            event: ImeEvent::Committed(text),
            ..
        } if text.as_ref() == "漢字"
    ));
    assert!(matches!(
        &received[5],
        SurfaceEvent::Ime {
            input_epoch: InputEpoch::INITIAL,
            event: ImeEvent::Started,
            ..
        }
    ));
    assert!(matches!(
        &received[6],
        SurfaceEvent::Ime {
            input_epoch: InputEpoch::INITIAL,
            event: ImeEvent::Updated { text, .. },
            ..
        } if text.as_ref() == "かな"
    ));
    assert!(matches!(
        &received[7],
        SurfaceEvent::Ime {
            input_epoch: InputEpoch::INITIAL,
            event: ImeEvent::Cancelled,
            ..
        }
    ));
    let next_epoch = InputEpoch::INITIAL
        .checked_next()
        .ok_or("next input epoch")?;
    assert!(matches!(
        &received[8],
        SurfaceEvent::Focus {
            input_epoch,
            focused: false,
            ..
        } if *input_epoch == next_epoch
    ));
    assert!(matches!(
        &received[9],
        SurfaceEvent::Focus {
            input_epoch,
            focused: true,
            ..
        } if *input_epoch == next_epoch
    ));
    assert!(matches!(
        &received[10],
        SurfaceEvent::Pointer {
            action: PointerAction::Down,
            position,
            button: PointerButton::Primary,
            modifiers,
            ..
        } if position.x() == 12.0
            && position.y() == 46.0
            && modifiers.bits() == Modifiers::COMMAND
    ));
    assert!(matches!(
        &received[11],
        SurfaceEvent::Scroll {
            delta_x: 4.0,
            delta_y: -3.0,
            phase: ScrollPhase::None,
            precise: false,
            modifiers,
            ..
        } if modifiers.bits() == 0
    ));

    let timestamps = received
        .iter()
        .map(|event| event.timestamp().get())
        .collect::<Vec<_>>();
    assert!(
        timestamps.windows(2).all(|pair| pair[0] < pair[1]),
        "native input timestamps must increase: {timestamps:?}"
    );
    drop(received);

    let evidence = native_validation::close_with_owner_evidence(surface)?;
    assert_eq!(evidence.active(), [0; 10]);
    assert_eq!(evidence.pasteboard_releases(), 0);
    assert_eq!(evidence.release_order_violations(), 0);

    validate_startup_focus_publication(&descriptor, next_epoch, true)?;
    validate_startup_focus_publication(&descriptor, InputEpoch::INITIAL, false)?;
    Ok(())
}

#[cfg(not(all(alpine_native_validation, target_os = "macos", target_arch = "aarch64")))]
fn main() {}
