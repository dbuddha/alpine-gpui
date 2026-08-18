//! Native cross-thread, coalesced, zero-frame wake qualification.

#[cfg(all(alpine_native_validation, target_os = "macos", target_arch = "aarch64"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::{
        sync::{Arc, Mutex, mpsc::sync_channel},
        thread,
        time::Duration,
    };

    use alpine_platform_macos::{
        SurfaceDescriptor, SurfaceEvent, SurfaceLifecycle, SurfaceResponse, SurfaceWakeAdmission,
        native_validation,
    };

    let descriptor = SurfaceDescriptor::new("Alpine worker wake", 96.0, 64.0, 1.0)?;
    let surface = native_validation::new_surface(&descriptor)?;
    surface.show()?;
    let observer = surface.observer();
    let waker = surface.waker();
    let worker_waker = waker.clone();
    let (ready_sender, ready_receiver) = sync_channel(1);
    let (admission_sender, admission_receiver) = sync_channel(1);
    let worker = thread::spawn(move || {
        ready_receiver
            .recv_timeout(Duration::from_secs(1))
            .map_err(|_| "initial wake was not admitted")?;
        let first = worker_waker.wake();
        let second = worker_waker.wake();
        admission_sender
            .send((first, second))
            .map_err(|_| "wake admission receiver disconnected")
    });

    let received = Arc::new(Mutex::new(Vec::new()));
    let callback_received = Arc::clone(&received);
    let admissions = Arc::new(Mutex::new(None));
    let callback_admissions = Arc::clone(&admissions);
    let mut ready_sender = Some(ready_sender);
    native_validation::arm_window_close(&surface, Duration::from_millis(50));
    surface.run_with_event_handler(move |event| {
        if let SurfaceEvent::Wake { timestamp } = event
            && let Ok(mut received) = callback_received.lock()
        {
            received.push(timestamp.get());
            if let Some(sender) = ready_sender.take() {
                let _ = sender.send(());
                if let Ok(admitted) = admission_receiver.recv_timeout(Duration::from_secs(1))
                    && let Ok(mut admissions) = callback_admissions.lock()
                {
                    *admissions = Some(admitted);
                }
            }
        }
        SurfaceResponse::default()
    })?;
    worker.join().map_err(|_| "worker wake thread panicked")??;

    let received = received.lock().map_err(|_| "wake receiver poisoned")?;
    assert_eq!(received.len(), 2);
    assert!(received[1] > received[0]);
    drop(received);
    assert_eq!(observer.lifecycle(), SurfaceLifecycle::Closing);
    assert_eq!(surface.snapshot().submission_count(), 0);
    assert_eq!(
        *admissions.lock().map_err(|_| "admissions poisoned")?,
        Some((
            SurfaceWakeAdmission::Scheduled,
            SurfaceWakeAdmission::Coalesced
        ))
    );

    let evidence = waker.snapshot();
    assert_eq!(evidence.requests(), 2);
    assert_eq!(evidence.scheduled(), 1);
    assert_eq!(evidence.coalesced(), 1);
    assert_eq!(evidence.dispatched(), 1);
    assert_eq!(evidence.rejected(), 0);
    assert_eq!(waker.wake(), SurfaceWakeAdmission::Closed);
    assert_eq!(waker.snapshot().rejected(), 1);

    let owners = native_validation::close_with_owner_evidence(surface)?;
    assert_eq!(owners.active(), [0; 9]);
    assert_eq!(owners.release_order_violations(), 0);
    Ok(())
}

#[cfg(not(all(alpine_native_validation, target_os = "macos", target_arch = "aarch64")))]
fn main() {}
