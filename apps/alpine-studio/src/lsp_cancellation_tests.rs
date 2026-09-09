use super::*;

fn prepared_client() -> Result<(LspClient, PreparedCancellation), Box<dyn Error>> {
    let identity = ProcessIdentity::new(1, 1).ok_or("identity")?;
    let mut client = LspClient::inert_for_test(identity);
    client.initialize_inert_for_test();
    assert!(client.take_input_for_test()?.is_some());
    assert!(client.take_input_for_test()?.is_none());
    let stamp = RequestStamp::new(1, 1, 1, 1, 1, 1).ok_or("stamp")?;
    let request = client.begin_request("textDocument/hover", None, stamp)?;
    assert!(client.take_input_for_test()?.is_some());
    let cancellation = client.prepare_cancel(request.request_id)?;
    assert_eq!(client.snapshot().peer.pending_requests(), 0);
    assert_eq!(client.snapshot().peer.cancelled_requests(), 1);
    assert!((1..128).contains(&cancellation.retained_bytes()));
    Ok((client, cancellation))
}

#[test]
fn prepared_cancellation_retains_original_bytes_through_both_pressure_bounds()
-> Result<(), Box<dyn Error>> {
    for retained_budget in [false, true] {
        let (mut client, mut cancellation) = prepared_client()?;
        let original = cancellation
            .outbound
            .as_ref()
            .ok_or("prepared bytes")?
            .bytes()
            .to_vec();
        let fill_count = if retained_budget {
            1
        } else {
            crate::lsp_process::INPUT_CAPACITY
        };
        for _ in 0..fill_count {
            if retained_budget {
                // Occupy the real transport byte budget, not a server transcript.
                // No successful writer event or child-process counter is forged.
                let _ = client.process.send(&vec![b'x'; 16_777_216])?;
            } else {
                let _ = client.notify("$/setTrace", None)?;
            }
        }
        let expected = if retained_budget {
            SubmitError::RetainedBudget
        } else {
            SubmitError::Saturated
        };
        for _ in 0..8 {
            assert_eq!(
                client.send_cancel(&mut cancellation),
                Err(LspClientError::Submit(expected))
            );
            assert_eq!(cancellation.retained_bytes(), original.len());
            assert_eq!(client.snapshot().peer.cancelled_requests(), 1);
        }
        for _ in 0..fill_count {
            assert!(client.take_input_for_test()?.is_some());
        }
        assert!(client.take_input_for_test()?.is_none());
        assert_eq!(client.snapshot().process.retained_bytes, 0);
        let _ = client.send_cancel(&mut cancellation)?;
        assert_eq!(
            client.take_input_for_test()?.ok_or("cancel delivery")?,
            original
        );
        assert_eq!(cancellation.retained_bytes(), 0);
        assert_eq!(
            client.send_cancel(&mut cancellation),
            Err(LspClientError::Protocol(ProtocolError::InvalidLifecycle))
        );
        assert!(client.take_input_for_test()?.is_none());
        assert_eq!(client.snapshot().peer.cancelled_requests(), 1);
        assert!(matches!(
            client.notify("$/cancelRequest", None),
            Err(LspClientError::Protocol(ProtocolError::InvalidLifecycle))
        ));
        let _ = client.shutdown();
    }
    Ok(())
}

#[test]
fn prepared_cancellation_rejects_old_process_before_started_check() -> Result<(), Box<dyn Error>> {
    let (mut client, mut cancellation) = prepared_client()?;
    let mut observer = client.take_input_observer_for_test()?;
    let _ = client.restart(ProcessIdentity::new(1, 2).ok_or("replacement")?)?;
    assert!(!client.snapshot().started);
    let before = client.snapshot().process;
    assert_eq!(
        client.send_cancel(&mut cancellation),
        Err(LspClientError::StaleCancellation)
    );
    assert_eq!(
        client.snapshot().process.submitted_inputs,
        before.submitted_inputs
    );
    assert_eq!(client.snapshot().process.restarts, before.restarts);
    assert!(observer.take_input()?.is_none());
    assert!(cancellation.retained_bytes() > 0);
    let _ = client.shutdown();
    Ok(())
}

#[test]
fn prepared_cancellation_cannot_bypass_shutdown_lifecycle() -> Result<(), Box<dyn Error>> {
    let (mut client, mut cancellation) = prepared_client()?;
    let _ = client.begin_shutdown()?;
    let submitted = client.snapshot().process.submitted_inputs;
    assert_eq!(
        client.send_cancel(&mut cancellation),
        Err(LspClientError::Protocol(ProtocolError::InvalidLifecycle))
    );
    assert_eq!(client.snapshot().process.submitted_inputs, submitted);
    let _ = client.shutdown();
    assert_eq!(
        client.send_cancel(&mut cancellation),
        Err(LspClientError::ProcessNotStarted)
    );
    Ok(())
}

#[test]
fn prepared_cancellation_rejects_another_client_with_the_same_logical_ids()
-> Result<(), Box<dyn Error>> {
    let (mut owner, mut cancellation) = prepared_client()?;
    let original = cancellation
        .outbound
        .as_ref()
        .ok_or("original cancellation")?
        .bytes()
        .to_vec();
    let body: serde_json::Value = serde_json::from_slice(
        cancellation
            .outbound
            .as_ref()
            .ok_or("cancellation body")?
            .body(),
    )?;
    let mut other = LspClient::inert_for_test(ProcessIdentity::new(1, 1).ok_or("identity")?);
    other.initialize_inert_for_test();
    assert!(other.take_input_for_test()?.is_some());
    let stamp = RequestStamp::new(1, 1, 1, 1, 1, 1).ok_or("stamp")?;
    let request = other.begin_request("textDocument/hover", None, stamp)?;
    assert_eq!(body["params"]["id"], request.request_id);
    assert!(other.take_input_for_test()?.is_some());
    assert!(other.take_input_for_test()?.is_none());
    let before = other.snapshot();
    assert_eq!(
        other.send_cancel(&mut cancellation),
        Err(LspClientError::StaleCancellation)
    );
    assert_eq!(
        other.snapshot().process.submitted_inputs,
        before.process.submitted_inputs
    );
    assert_eq!(other.snapshot().peer.pending_requests(), 1);
    assert_eq!(other.snapshot().peer.cancelled_requests(), 0);
    assert!(other.take_input_for_test()?.is_none());
    assert_eq!(cancellation.retained_bytes(), original.len());
    let _ = owner.send_cancel(&mut cancellation)?;
    assert_eq!(
        owner.take_input_for_test()?.ok_or("owner delivery")?,
        original
    );
    let _ = owner.shutdown();
    let _ = other.shutdown();
    Ok(())
}
