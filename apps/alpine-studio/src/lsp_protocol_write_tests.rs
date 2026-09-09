use super::*;

fn frame(value: &serde_json::Value) -> Vec<u8> {
    let body = value.to_string();
    format!("Content-Length: {}\r\n\r\n{body}", body.len()).into_bytes()
}

fn decode(bytes: &[u8]) -> Result<serde_json::Value, Box<dyn Error>> {
    let mut framer = LspFramer::new(LspFrameLimits::default());
    let batch = framer.ingest(bytes)?;
    assert_eq!(batch.consumed(), bytes.len());
    assert_eq!(batch.frames().len(), 1);
    let value = serde_json::from_slice(batch.frames()[0].body())?;
    framer.finish()?;
    Ok(value)
}

fn ready() -> Result<LspClient, Box<dyn Error>> {
    let identity = ProcessIdentity::new(1, 1).ok_or("identity")?;
    let mut client = LspClient::inert_for_test(identity);
    let request = client.begin_initialize()?;
    let _ = client.take_input_for_test()?.ok_or("initialize")?;
    client.inject_stdout_for_test(&frame(&serde_json::json!({
        "jsonrpc":"2.0", "id":request.request_id, "result":{"capabilities":{}}
    })))?;
    let mut initialized = 0;
    let _ = client.poll(None, |event| {
        initialized += usize::from(matches!(event, PeerEvent::Initialized(_)));
    })?;
    assert_eq!(initialized, 1);
    let bytes = client.take_input_for_test()?.ok_or("initialized")?;
    assert_eq!(decode(&bytes)?["method"], "initialized");
    assert!(client.take_input_for_test()?.is_none());
    Ok(client)
}

// Occupy the real bounded transport controls, not a server transcript.
fn saturate(client: &mut LspClient) -> Result<(), Box<dyn Error>> {
    for _ in 0..8 {
        let _ = client.process.send(b"occupied")?;
    }
    assert_eq!(
        client.process.send(b"overflow"),
        Err(SubmitError::Saturated)
    );
    Ok(())
}

fn drain_pressure(client: &mut LspClient) -> Result<(), Box<dyn Error>> {
    for _ in 0..8 {
        assert_eq!(
            client.take_input_for_test()?.as_deref(),
            Some(b"occupied".as_slice())
        );
    }
    assert!(client.take_input_for_test()?.is_none());
    Ok(())
}

fn refresh(id: u32) -> serde_json::Value {
    serde_json::json!({"jsonrpc":"2.0","id":id,"method":"workspace/diagnostic/refresh"})
}

fn poll_without_publication(client: &mut LspClient) -> Result<LspClientPoll, LspClientError> {
    let mut published = 0;
    let result = client.poll(None, |_| published += 1);
    assert_eq!(published, 0, "unexpected peer publication");
    result
}

#[test]
fn protocol_writes_graceful_shutdown_drains_pressure_with_one_deadline()
-> Result<(), Box<dyn Error>> {
    let mut client = ready()?;
    saturate(&mut client)?;
    client.inject_stdout_for_test(&frame(&refresh(91)))?;
    let _ = client.poll(None, |_| {})?;
    assert_eq!(client.snapshot().protocol_writes.queued, 1);
    let mut observer = client.take_input_observer_for_test()?;
    let deadline = Instant::now() + Duration::from_secs(1);
    let (report, snapshot) = thread::scope(|scope| -> Result<_, Box<dyn Error>> {
        let worker = scope.spawn(move || {
            let report = client.shutdown_gracefully_until(deadline);
            (report, client.snapshot())
        });
        let mut messages = Vec::new();
        while messages.len() < 10 {
            assert!(
                Instant::now() < deadline,
                "shutdown did not drain its original pressure"
            );
            if let Some(bytes) = observer.take_input()? {
                messages.push(bytes);
            } else {
                thread::sleep(Duration::from_millis(1));
            }
        }
        for bytes in &messages[..8] {
            assert_eq!(bytes.as_slice(), b"occupied");
        }
        assert_eq!(decode(&messages[8])?["id"], 91);
        assert_eq!(decode(&messages[9])?["method"], "shutdown");
        worker.join().map_err(|_| "shutdown worker panicked".into())
    })?;
    // The controlled transport supplies no shutdown reply or process exit.
    // Draining pressure must not turn missing terminal evidence into success.
    assert_eq!(report.protocol, LspShutdownProtocol::Deadline);
    assert_eq!(snapshot.peer.lifecycle(), PeerLifecycle::ShuttingDown);
    assert_eq!(snapshot.protocol_writes.retained_bytes, 0);
    assert!(snapshot.protocol_writes.peak_retained_bytes > 0);
    assert_eq!(snapshot.process.retained_bytes, 0);
    assert!(!snapshot.started);
    Ok(())
}

#[test]
fn protocol_writes_preserve_multiple_batches_partial_tail_and_fifo() -> Result<(), Box<dyn Error>> {
    let mut client = ready()?;
    saturate(&mut client)?;
    let mut bytes = Vec::new();
    for id in 1..=33 {
        bytes.extend(frame(&refresh(id)));
    }
    let tail = frame(&refresh(34));
    let split = tail.len() / 2;
    bytes.extend_from_slice(&tail[..split]);
    client.inject_stdout_for_test(&bytes)?;
    let mut refreshed = Vec::new();
    let _ = client.poll(None, |event| {
        if let PeerEvent::InboundRequest { id, .. } = event {
            refreshed.push(id);
        }
    })?;
    assert_eq!(refreshed, (1..=33).collect::<Vec<_>>());
    let snapshot = client.snapshot().protocol_writes;
    assert_eq!(snapshot.queued, 33);
    assert!(snapshot.retained_bytes > snapshot.payload_bytes);
    assert_eq!(
        snapshot.retained_bytes,
        snapshot.payload_bytes + snapshot.capacity_bytes
    );
    assert!(!snapshot.failed);
    assert_eq!(
        client.notify("textDocument/didSave", None),
        Err(SubmitError::Saturated.into())
    );
    drain_pressure(&mut client)?;
    let mut delivered = Vec::new();
    for _ in 0..5 {
        let _ = poll_without_publication(&mut client)?;
        while let Some(bytes) = client.take_input_for_test()? {
            let response = decode(&bytes)?;
            assert!(response["result"].is_null());
            delivered.push(response["id"].as_u64().ok_or("response id")?);
        }
    }
    assert_eq!(delivered, (1..=33).collect::<Vec<_>>());
    assert_eq!(client.snapshot().protocol_writes.retained_bytes, 0);
    assert!(client.snapshot().protocol_writes.peak_retained_bytes > 0);
    client.inject_stdout_for_test(&tail[split..])?;
    let _ = client.poll(None, |event| {
        if let PeerEvent::InboundRequest { id, .. } = event {
            refreshed.push(id);
        }
    })?;
    assert_eq!(refreshed, (1..=34).collect::<Vec<_>>());
    assert_eq!(
        decode(&client.take_input_for_test()?.ok_or("tail response")?)?["id"],
        34
    );
    assert_eq!(poll_without_publication(&mut client)?, LspClientPoll::Idle);
    assert_eq!(client.shutdown().protocol_writes.retained_bytes, 0);
    Ok(())
}

#[test]
fn protocol_writes_retry_after_local_output_budget_release() -> Result<(), Box<dyn Error>> {
    let mut client = ready()?;
    client.inject_stdout_for_test(&frame(&refresh(91)))?;
    let used = client.snapshot().process.retained_bytes;
    let filler = vec![0_u8; 16_777_216 - used];
    let _ = client.process.send(&filler)?;
    drop(filler);
    assert_eq!(client.snapshot().process.retained_bytes, 16_777_216);
    let mut refreshed = 0;
    let _ = client.poll(None, |event| {
        refreshed += usize::from(matches!(event, PeerEvent::InboundRequest { .. }));
    })?;
    assert_eq!(refreshed, 1);
    // No new external event was supplied. Dropping the consumed output
    // payload admitted the retained response in this very same poll.
    assert_eq!(client.snapshot().protocol_writes.queued, 0);
    assert!(client.snapshot().protocol_writes.peak_retained_bytes > 0);
    let _ = client.take_input_for_test()?.ok_or("budget filler")?;
    assert_eq!(
        decode(&client.take_input_for_test()?.ok_or("response")?)?["id"],
        91
    );
    assert!(client.take_input_for_test()?.is_none());
    let _ = client.shutdown();
    Ok(())
}

#[test]
fn protocol_writes_defer_initialized_and_exit_until_enqueue() -> Result<(), Box<dyn Error>> {
    let mut client = LspClient::inert_for_test(ProcessIdentity::new(1, 1).ok_or("identity")?);
    let request = client.begin_initialize()?;
    let _ = client.take_input_for_test()?.ok_or("initialize")?;
    saturate(&mut client)?;
    client.inject_stdout_for_test(&frame(&serde_json::json!({
        "jsonrpc":"2.0","id":request.request_id,"result":{"capabilities":{}}
    })))?;
    let _ = poll_without_publication(&mut client)?;
    assert_eq!(client.snapshot().protocol_writes.queued, 1);
    assert_eq!(
        client.notify("textDocument/didOpen", None),
        Err(SubmitError::Saturated.into())
    );
    drain_pressure(&mut client)?;
    let mut initialized = 0;
    let _ = client.poll(None, |event| {
        initialized += usize::from(matches!(event, PeerEvent::Initialized(_)));
    })?;
    assert_eq!(initialized, 1);
    assert_eq!(
        decode(&client.take_input_for_test()?.ok_or("initialized")?)?["method"],
        "initialized"
    );
    let shutdown = client.begin_shutdown()?;
    let _ = client.take_input_for_test()?.ok_or("shutdown")?;
    saturate(&mut client)?;
    let mut bytes = frame(&refresh(91));
    bytes.extend(frame(
        &serde_json::json!({"jsonrpc":"2.0","id":shutdown.request_id,"result":null}),
    ));
    client.inject_stdout_for_test(&bytes)?;
    let _ = poll_without_publication(&mut client)?;
    assert_eq!(client.snapshot().protocol_writes.queued, 2);
    drain_pressure(&mut client)?;
    let mut exited = 0;
    let _ = client.poll(None, |event| {
        exited += usize::from(matches!(event, PeerEvent::ShutdownAcknowledged));
    })?;
    assert_eq!(exited, 1);
    let response = decode(&client.take_input_for_test()?.ok_or("shutdown response")?)?;
    assert_eq!(response["id"], 91);
    assert_eq!(response["error"]["code"], -32800);
    assert_eq!(
        decode(&client.take_input_for_test()?.ok_or("exit")?)?["method"],
        "exit"
    );
    assert!(client.take_input_for_test()?.is_none());
    assert_eq!(client.snapshot().protocol_writes.retained_bytes, 0);
    let _ = client.shutdown();
    Ok(())
}

#[test]
fn protocol_writes_limits_fail_closed_and_retire_owned_bytes() -> Result<(), Box<dyn Error>> {
    for byte_limit in [false, true] {
        let mut client = ready()?;
        saturate(&mut client)?;
        let mut bytes = Vec::new();
        for id in 1..=257 {
            bytes.extend(frame(&if byte_limit {
                serde_json::json!({"jsonrpc":"2.0","id":id,"method":"unsupported"})
            } else {
                refresh(id)
            }));
        }
        client.inject_stdout_for_test(&bytes)?;
        assert_eq!(
            client.poll(None, |_| {}),
            Err(LspClientError::ProtocolWriteBudget)
        );
        let snapshot = client.snapshot().protocol_writes;
        assert!(snapshot.failed);
        assert!(snapshot.payload_bytes <= MAX_PROTOCOL_PAYLOAD_BYTES);
        if byte_limit {
            assert!(snapshot.queued < MAX_PROTOCOL_WRITES);
        } else {
            assert_eq!(snapshot.queued, MAX_PROTOCOL_WRITES);
        }
        assert_eq!(
            client.notify("textDocument/didSave", None),
            Err(LspClientError::ProtocolWriteBudget)
        );
        client.inject_stdout_for_test(&frame(&refresh(999)))?;
        assert_eq!(
            poll_without_publication(&mut client),
            Err(LspClientError::ProtocolWriteBudget)
        );
        assert_eq!(client.snapshot().process.queued_events, 0);
        assert_eq!(client.snapshot().protocol_writes, snapshot);
        drain_pressure(&mut client)?;
        assert_eq!(
            poll_without_publication(&mut client),
            Err(LspClientError::ProtocolWriteBudget)
        );
        assert!(client.take_input_for_test()?.is_none());
        let result = client.shutdown();
        assert_eq!(result.protocol_writes.retained_bytes, 0);
        assert_eq!(result.process.retained_bytes, 0);
    }
    Ok(())
}

#[test]
fn protocol_writes_restart_is_transactional_and_cannot_replay_old_replies()
-> Result<(), Box<dyn Error>> {
    let mut client = ready()?;
    saturate(&mut client)?;
    client.inject_stdout_for_test(&frame(&refresh(91)))?;
    let _ = client.poll(None, |_| {})?;
    let snapshot = client.snapshot().protocol_writes;
    let identity = ProcessIdentity::new(1, 2).ok_or("replacement identity")?;
    assert_eq!(client.restart(identity), Err(SubmitError::Saturated.into()));
    assert_eq!(client.snapshot().protocol_writes, snapshot);
    drain_pressure(&mut client)?;
    let _ = client.restart(identity)?;
    assert_eq!(client.snapshot().protocol_writes.retained_bytes, 0);
    assert!(!client.snapshot().started);
    let _ = client.shutdown();
    Ok(())
}

#[test]
fn protocol_writes_fatal_latch_cannot_publish_delayed_initialization() -> Result<(), Box<dyn Error>>
{
    let mut client = LspClient::inert_for_test(ProcessIdentity::new(1, 1).ok_or("identity")?);
    let request = client.begin_initialize()?;
    let _ = client.take_input_for_test()?.ok_or("initialize")?;
    saturate(&mut client)?;
    let mut bytes = frame(&serde_json::json!({
        "jsonrpc":"2.0","id":request.request_id,"result":{"capabilities":{}}
    }));
    for id in 1..=257 {
        bytes.extend(frame(&refresh(id)));
    }
    client.inject_stdout_for_test(&bytes)?;
    let mut initialized = 0;
    assert_eq!(
        client.poll(None, |event| {
            initialized += usize::from(matches!(event, PeerEvent::Initialized(_)));
        }),
        Err(LspClientError::ProtocolWriteBudget)
    );
    assert_eq!(initialized, 0);
    drain_pressure(&mut client)?;
    assert_eq!(
        poll_without_publication(&mut client),
        Err(LspClientError::ProtocolWriteBudget)
    );
    assert!(client.take_input_for_test()?.is_none());
    let report = client.shutdown_gracefully_until(Instant::now() + Duration::from_millis(30));
    assert_eq!(
        report.protocol,
        LspShutdownProtocol::Failed(LspClientError::ProtocolWriteBudget)
    );
    assert_eq!(client.snapshot().protocol_writes.retained_bytes, 0);
    Ok(())
}
