//! Native handle-free event transport qualification.

#[cfg(all(alpine_native_validation, target_os = "macos", target_arch = "aarch64"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::sync::{Arc, Mutex};

    use alpine_core::Point;
    use alpine_platform_macos::{
        ClipboardError, ClipboardEvent, ClipboardOperation, ClipboardText, ClipboardWrite,
        CloseDisposition, EventTimestamp, ImeEvent, KeyState, Modifiers, PointerAction,
        PointerButton, ScrollPhase, SurfaceDescriptor, SurfaceEvent, SurfaceExtent,
        SurfaceLifecycle, SurfaceResponse, native_validation,
    };
    use native_validation::CloseReplayScenario;

    let descriptor = SurfaceDescriptor::new("Alpine runtime events", 96.0, 64.0, 1.0)?;
    let surface = native_validation::new_surface(&descriptor)?;
    let point = Point::new(12.0, 18.0).ok_or("valid pointer position")?;
    let events = vec![
        SurfaceEvent::Keyboard {
            timestamp: EventTimestamp::new(1),
            state: KeyState::Down,
            physical_key: 12,
            logical_key: "q".into(),
            modifiers: Modifiers::from_bits(Modifiers::COMMAND | Modifiers::SHIFT),
            repeat: true,
        },
        SurfaceEvent::Pointer {
            timestamp: EventTimestamp::new(2),
            action: PointerAction::Down,
            position: point,
            button: PointerButton::Primary,
            modifiers: Modifiers::default(),
        },
        SurfaceEvent::Scroll {
            timestamp: EventTimestamp::new(3),
            delta_x: 1.25,
            delta_y: -3.5,
            phase: ScrollPhase::Changed,
            precise: true,
            modifiers: Modifiers::from_bits(Modifiers::OPTION),
        },
        SurfaceEvent::Focus {
            timestamp: EventTimestamp::new(4),
            focused: true,
        },
        SurfaceEvent::Resize {
            timestamp: EventTimestamp::new(5),
            extent: SurfaceExtent::new(96.0, 64.0, 2.0)?,
        },
        SurfaceEvent::Clipboard {
            timestamp: EventTimestamp::new(6),
            event: ClipboardEvent::PasteCompleted(Err(ClipboardError::Unavailable)),
        },
        SurfaceEvent::Ime {
            timestamp: EventTimestamp::new(7),
            event: ImeEvent::Updated {
                text: "kana".into(),
                selected_start_utf16: 1,
                selected_length_utf16: 2,
            },
        },
        SurfaceEvent::Wake {
            timestamp: EventTimestamp::new(8),
        },
        SurfaceEvent::CloseRequested {
            timestamp: EventTimestamp::new(9),
        },
    ];
    let received = Arc::new(Mutex::new(Vec::new()));
    let callback_received = Arc::clone(&received);
    native_validation::replay_surface_events(&surface, &events, move |event| {
        if let Ok(mut received) = callback_received.lock() {
            received.push(event);
        }
        SurfaceResponse::default()
    })?;
    assert_eq!(
        *received.lock().map_err(|_| "event receiver poisoned")?,
        events
    );

    let replayed = Arc::new(Mutex::new(Vec::new()));
    let callback_replayed = Arc::clone(&replayed);
    native_validation::replay_surface_events(&surface, &events, move |event| {
        if let Ok(mut replayed) = callback_replayed.lock() {
            replayed.push(event);
        }
        SurfaceResponse::default()
    })?;
    assert_eq!(
        *replayed
            .lock()
            .map_err(|_| "event replay receiver poisoned")?,
        events
    );

    let callback_received = Arc::new(Mutex::new(Vec::new()));
    let appkit_callback_received = Arc::clone(&callback_received);
    native_validation::replay_callback_surface_events(&surface, &events, move |event| {
        if let Ok(mut received) = appkit_callback_received.lock() {
            received.push(event);
        }
        SurfaceResponse::default()
    })?;
    assert_eq!(
        *callback_received
            .lock()
            .map_err(|_| "AppKit callback receiver poisoned")?,
        events
    );

    let copy_events = Arc::new(Mutex::new(Vec::new()));
    let copy_received = Arc::clone(&copy_events);
    native_validation::replay_native_clipboard_operation(
        &surface,
        ClipboardOperation::Copy,
        move |event| {
            let response = if matches!(
                &event,
                SurfaceEvent::Keyboard {
                    state: KeyState::Down,
                    logical_key,
                    modifiers,
                    repeat: false,
                    ..
                } if logical_key.as_ref() == "c"
                    && modifiers.bits() == Modifiers::COMMAND
            ) {
                let text = ClipboardText::new("native-copy")
                    .and_then(|text| ClipboardWrite::new(ClipboardOperation::Copy, text));
                text.map_or_else(
                    |_| SurfaceResponse::default(),
                    |write| SurfaceResponse::new(None, Some(write), CloseDisposition::NotRequested),
                )
            } else {
                SurfaceResponse::default()
            };
            if let SurfaceEvent::Clipboard { event, .. } = event
                && let Ok(mut received) = copy_received.lock()
            {
                received.push(event);
            }
            response
        },
    )?;
    assert_eq!(
        *copy_events.lock().map_err(|_| "copy receiver poisoned")?,
        vec![ClipboardEvent::CopyCompleted(Ok(()))]
    );

    let paste_events = Arc::new(Mutex::new(Vec::new()));
    let paste_received = Arc::clone(&paste_events);
    native_validation::replay_native_clipboard_operation(
        &surface,
        ClipboardOperation::Paste,
        move |event| {
            if let SurfaceEvent::Clipboard { event, .. } = event
                && let Ok(mut received) = paste_received.lock()
            {
                received.push(event);
            }
            SurfaceResponse::default()
        },
    )?;
    assert!(matches!(
        paste_events
            .lock()
            .map_err(|_| "paste receiver poisoned")?
            .as_slice(),
        [ClipboardEvent::PasteCompleted(Ok(text))] if text.as_str() == "native-copy"
    ));

    native_validation::inject_clipboard_error(&surface, ClipboardError::WriteRejected);
    let failure_events = Arc::new(Mutex::new(Vec::new()));
    let failure_received = Arc::clone(&failure_events);
    native_validation::replay_native_clipboard_operation(
        &surface,
        ClipboardOperation::Cut,
        move |event| {
            let response = if matches!(&event, SurfaceEvent::Keyboard { .. }) {
                ClipboardText::new("preserved-cut")
                    .and_then(|text| ClipboardWrite::new(ClipboardOperation::Cut, text))
                    .map_or_else(
                        |_| SurfaceResponse::default(),
                        |write| {
                            SurfaceResponse::new(None, Some(write), CloseDisposition::NotRequested)
                        },
                    )
            } else {
                SurfaceResponse::default()
            };
            if let SurfaceEvent::Clipboard { event, .. } = event
                && let Ok(mut received) = failure_received.lock()
            {
                received.push(event);
            }
            response
        },
    )?;
    assert_eq!(
        *failure_events
            .lock()
            .map_err(|_| "clipboard failure receiver poisoned")?,
        vec![ClipboardEvent::CutCompleted(Err(
            ClipboardError::WriteRejected
        ))]
    );

    let observer = surface.observer();
    assert!(!native_validation::replay_close(
        &surface,
        CloseReplayScenario::MissingHandler
    )?);
    assert_eq!(observer.lifecycle(), SurfaceLifecycle::Live);
    assert!(!native_validation::replay_close(
        &surface,
        CloseReplayScenario::ReentrantHandler
    )?);
    assert_eq!(observer.lifecycle(), SurfaceLifecycle::Live);
    assert!(!native_validation::replay_close(
        &surface,
        CloseReplayScenario::Cancel
    )?);
    assert_eq!(observer.lifecycle(), SurfaceLifecycle::Live);
    assert!(native_validation::replay_close(
        &surface,
        CloseReplayScenario::Allow
    )?);
    assert_eq!(observer.lifecycle(), SurfaceLifecycle::Closing);

    let evidence = native_validation::close_with_owner_evidence(surface)?;
    assert_eq!(evidence.active(), [0; 9]);
    assert_eq!(evidence.release_order_violations(), 0);
    Ok(())
}

#[cfg(not(all(alpine_native_validation, target_os = "macos", target_arch = "aarch64")))]
fn main() {}
