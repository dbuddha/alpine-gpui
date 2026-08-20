//! Native `AppKit` responder and text-input qualification.

#[cfg(all(alpine_native_validation, target_os = "macos", target_arch = "aarch64"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::sync::{Arc, Mutex};

    use alpine_platform_macos::{
        ImeEvent, InputEpoch, KeyState, Modifiers, PointerAction, PointerButton, ScrollPhase,
        SurfaceDescriptor, SurfaceEvent, SurfaceResponse, native_validation,
    };

    let descriptor = SurfaceDescriptor::new("Alpine native input", 96.0, 64.0, 1.0)?;
    let surface = native_validation::new_surface(&descriptor)?;
    surface.show()?;

    let received = Arc::new(Mutex::new(Vec::new()));
    let callback_received = Arc::clone(&received);
    native_validation::replay_native_input_path(&surface, move |event| {
        if let Ok(mut received) = callback_received.lock() {
            received.push(event);
        }
        SurfaceResponse::default()
    })?;

    let received = received
        .lock()
        .map_err(|_| "native input receiver poisoned")?;
    assert_eq!(received.len(), 12);
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
    Ok(())
}

#[cfg(not(all(alpine_native_validation, target_os = "macos", target_arch = "aarch64")))]
fn main() {}
