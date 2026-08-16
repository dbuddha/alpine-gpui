//! Native handle-free event transport qualification.

#[cfg(all(alpine_native_validation, target_os = "macos", target_arch = "aarch64"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::sync::{Arc, Mutex};

    use alpine_core::Point;
    use alpine_platform_macos::{
        ClipboardOperation, EventTimestamp, ImeEvent, KeyState, Modifiers, PointerAction,
        PointerButton, ScrollPhase, SurfaceDescriptor, SurfaceEvent, SurfaceExtent,
        native_validation,
    };

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
            operation: ClipboardOperation::Paste,
            succeeded: false,
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
        None
    })?;
    assert_eq!(
        *received.lock().map_err(|_| "event receiver poisoned")?,
        events
    );

    let evidence = native_validation::close_with_owner_evidence(surface)?;
    assert_eq!(evidence.active(), [0; 9]);
    assert_eq!(evidence.release_order_violations(), 0);
    Ok(())
}

#[cfg(not(all(alpine_native_validation, target_os = "macos", target_arch = "aarch64")))]
fn main() {}
