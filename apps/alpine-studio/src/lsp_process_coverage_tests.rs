use std::{env, thread, time::Instant};

use super::*;

const TIMEOUT: Duration = Duration::from_secs(3);

fn identity(generation: u64) -> ProcessIdentity {
    ProcessIdentity {
        workspace_revision: 1,
        generation,
    }
}

#[cfg(unix)]
fn shell(script: &str) -> Result<ProcessSpec, ConfigError> {
    ProcessSpec::new("/bin/sh", ["-c", script], None)
}

struct FaultSpawner(ProcessStage);

impl ThreadSpawner for FaultSpawner {
    fn spawn<F>(
        &self,
        stage: ProcessStage,
        name: &'static str,
        job: F,
    ) -> io::Result<JoinHandle<()>>
    where
        F: FnOnce() + Send + 'static,
    {
        if stage == self.0 {
            Err(io::Error::other("injected thread spawn failure"))
        } else {
            SystemThreadSpawner.spawn(stage, name, job)
        }
    }
}

#[derive(Default)]
struct TestWriter {
    write_error: Option<io::ErrorKind>,
    flush_error: Option<io::ErrorKind>,
}

impl Write for TestWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.write_error.map_or(Ok(bytes.len()), |kind| {
            Err(io::Error::new(kind, "injected write failure"))
        })
    }

    fn flush(&mut self) -> io::Result<()> {
        self.flush_error.map_or(Ok(()), |kind| {
            Err(io::Error::new(kind, "injected flush failure"))
        })
    }
}

struct FailingReader;

impl Read for FailingReader {
    fn read(&mut self, _bytes: &mut [u8]) -> io::Result<usize> {
        Err(io::Error::other("injected read failure"))
    }
}

fn started(identity: ProcessIdentity, epoch: ProcessEpoch) -> ProcessEvent {
    ProcessEvent::Started {
        identity,
        epoch,
        process_id: 1,
    }
}

fn detached_process(
    control: Option<SyncSender<Control>>,
    events: Receiver<ProcessEvent>,
) -> LanguageServerProcess {
    let (_completion_sender, supervisor_complete) = sync_channel(1);
    LanguageServerProcess {
        control,
        events,
        supervisor: None,
        supervisor_complete,
        shutdown: Arc::new(AtomicBool::new(false)),
        counters: Arc::new(Counters::default()),
        configuration_bytes: 7,
        identity: identity(1),
        epoch: ProcessEpoch(1),
        next_sequence: 0,
    }
}

fn wait_for(
    process: &mut LanguageServerProcess,
    predicate: impl Fn(&ProcessEvent) -> bool,
) -> Result<ProcessEvent, Box<dyn Error>> {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        if let Some(event) = process.try_event()?
            && predicate(&event)
        {
            return Ok(event);
        }
        if Instant::now() >= deadline {
            return Err("timed out waiting for language-server event".into());
        }
        thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn identities_and_configuration_fail_before_process_ownership() -> Result<(), Box<dyn Error>> {
    assert_eq!(ProcessIdentity::new(1, 1), Some(identity(1)));
    assert_eq!(ProcessIdentity::new(0, 1), None);
    assert_eq!(ProcessIdentity::new(1, 0), None);
    assert!(matches!(
        ProcessSpec::new("missing-alpine-language-server", ["x"], None),
        Err(ConfigError::MissingExecutable)
    ));
    let executable = env::current_exe()?;
    assert!(matches!(
        ProcessSpec::new(&executable, vec!["x"; MAX_ARGUMENTS + 1], None),
        Err(ConfigError::TooManyArguments)
    ));
    assert!(matches!(
        ProcessSpec::new(&executable, ["x".repeat(MAX_ARGUMENT_BYTES + 1)], None),
        Err(ConfigError::ArgumentTooLong)
    ));
    assert!(matches!(
        ProcessSpec::new(&executable, [OsString::from("a\0b")], None),
        Err(ConfigError::ContainsNul)
    ));
    assert!(matches!(
        ProcessSpec::new(&executable, ["x"], Some(&executable)),
        Err(ConfigError::WorkingDirectoryNotDirectory)
    ));
    Ok(())
}

#[test]
fn configuration_guards_and_diagnostics_are_exhaustive() -> Result<(), Box<dyn Error>> {
    assert!(matches!(
        ProcessSpec::new(env::temp_dir(), std::iter::empty::<&str>(), None),
        Err(ConfigError::ExecutableNotFile)
    ));
    assert_eq!(
        validate_path(Path::new(&"x".repeat(MAX_PATH_BYTES + 1))),
        Err(ConfigError::PathTooLong)
    );
    let executable = env::current_exe()?;
    let arguments = vec!["x".repeat(MAX_ARGUMENT_BYTES); 17];
    assert!(matches!(
        ProcessSpec::new(executable, arguments, None),
        Err(ConfigError::ConfigurationTooLarge)
    ));
    let diagnostics = [
        (
            ConfigError::MissingExecutable,
            "language-server executable is missing",
        ),
        (
            ConfigError::ExecutableNotFile,
            "language-server executable is not a regular file",
        ),
        (ConfigError::PathTooLong, "language-server path is too long"),
        (
            ConfigError::WorkingDirectoryNotDirectory,
            "language-server working directory is not a directory",
        ),
        (
            ConfigError::TooManyArguments,
            "language-server has too many arguments",
        ),
        (
            ConfigError::ArgumentTooLong,
            "language-server argument is too long",
        ),
        (
            ConfigError::ConfigurationTooLarge,
            "language-server configuration is too large",
        ),
        (
            ConfigError::ContainsNul,
            "language-server configuration contains NUL",
        ),
    ];
    for (error, message) in diagnostics {
        assert_eq!(error.to_string(), message);
        assert!(error.source().is_none());
    }
    let canonical_temp = fs::canonicalize(env::temp_dir())?;
    let spec = ProcessSpec::new(
        env::current_exe()?,
        std::iter::empty::<&str>(),
        Some(&canonical_temp),
    )?;
    assert_eq!(
        spec.working_directory.as_deref(),
        Some(canonical_temp.as_path())
    );
    let failure = ProcessFailure::retained(ProcessStage::Output);
    assert_eq!(
        failure.to_string(),
        "language-server process failed at Output: RetainedBudget"
    );
    assert!(failure.source().is_none());
    let counters = Arc::new(Counters::default());
    let payload = Payload::copy(b"abc", &counters, ProcessStage::Output)?;
    assert_eq!(format!("{payload:?}"), "Payload(3)");
    Ok(())
}

#[test]
fn output_and_write_forwarding_preserve_failures_and_overflow() -> Result<(), Box<dyn Error>> {
    let counters = Arc::new(Counters::default());
    let (output_sender, output_receiver) = sync_channel(1);
    output_sender.send(OutputPacket {
        stream: ProcessStream::Stdout,
        payload: Payload::copy(b"x", &counters, ProcessStage::Output)?,
    })?;
    let (full_events, _full_receiver) = sync_channel(0);
    assert_eq!(
        forward_outputs(
            &output_receiver,
            identity(1),
            ProcessEpoch(1),
            &full_events,
            &counters,
        ),
        Some(StopReason::EventOverflow)
    );

    let (write_sender, write_receiver) = sync_channel(2);
    let rejection = ProcessFailure::io(
        ProcessStage::Input,
        &io::Error::new(io::ErrorKind::BrokenPipe, "injected write failure"),
    );
    write_sender.send(WriteResult {
        sequence: InputSequence(7),
        bytes: 3,
        result: Err(rejection),
    })?;
    let (events, event_receiver) = sync_channel(2);
    assert_eq!(
        forward_writes(
            &write_receiver,
            identity(1),
            ProcessEpoch(1),
            &events,
            &counters,
        ),
        None
    );
    assert_eq!(
        event_receiver.recv_timeout(TIMEOUT)?.rejection(),
        Some((InputSequence(7), rejection))
    );
    write_sender.send(WriteResult {
        sequence: InputSequence(8),
        bytes: 1,
        result: Ok(()),
    })?;
    assert_eq!(
        forward_writes(
            &write_receiver,
            identity(1),
            ProcessEpoch(1),
            &full_events,
            &counters,
        ),
        Some(StopReason::EventOverflow)
    );
    assert_eq!(counters.written_inputs.load(Ordering::Relaxed), 1);
    Ok(())
}

#[test]
fn wait_forwarding_reports_failure_and_running_state() -> Result<(), Box<dyn Error>> {
    let counters = Counters::default();
    let (events, event_receiver) = sync_channel(2);
    let wait_error = io::Error::new(io::ErrorKind::PermissionDenied, "injected wait failure");
    assert_eq!(
        handle_wait(
            identity(1),
            ProcessEpoch(1),
            Err(wait_error),
            &events,
            &counters,
        ),
        WaitDecision::Terminate
    );
    assert!(matches!(
        event_receiver.recv_timeout(TIMEOUT)?.failure(),
        Some(ProcessFailure {
            stage: ProcessStage::Wait,
            ..
        })
    ));
    assert_eq!(
        handle_wait(identity(1), ProcessEpoch(1), Ok(None), &events, &counters,),
        WaitDecision::Running
    );
    Ok(())
}

#[test]
fn submission_failures_are_nonblocking_and_release_payloads() {
    let (_, disconnected_events) = sync_channel(1);
    let mut process = detached_process(None, disconnected_events);
    assert_eq!(
        process.send(&vec![0; MAX_MESSAGE_BYTES + 1]),
        Err(SubmitError::MessageTooLarge)
    );
    process.next_sequence = u64::MAX;
    assert_eq!(process.send(b"x"), Err(SubmitError::SequenceExhausted));
    process.next_sequence = 0;
    assert_eq!(process.send(b"x"), Err(SubmitError::Closed));
    assert_eq!(process.snapshot().retained_bytes, 0);

    let (closed_sender, closed_receiver) = sync_channel(1);
    drop(closed_receiver);
    process.control = Some(closed_sender);
    assert_eq!(process.send(b"closed"), Err(SubmitError::Closed));
    let (full_sender, _full_receiver) = sync_channel(0);
    process.control = Some(full_sender);
    assert_eq!(process.send(b"full"), Err(SubmitError::Saturated));
    assert_eq!(process.snapshot().input_saturations, 1);
    assert_eq!(process.snapshot().retained_bytes, 0);
}

#[test]
fn restart_and_event_channel_failures_are_exact() {
    let (_, disconnected_events) = sync_channel(1);
    let mut process = detached_process(None, disconnected_events);
    assert_eq!(process.restart(identity(1)), Err(SubmitError::StaleRestart));
    process.identity = identity(1);
    process.epoch = ProcessEpoch(u64::MAX);
    assert_eq!(
        process.restart(identity(2)),
        Err(SubmitError::EpochExhausted)
    );
    process.epoch = ProcessEpoch(1);
    assert_eq!(process.restart(identity(2)), Err(SubmitError::Closed));

    let (closed_sender, closed_receiver) = sync_channel(1);
    drop(closed_receiver);
    process.control = Some(closed_sender);
    assert_eq!(process.restart(identity(2)), Err(SubmitError::Closed));
    let (full_sender, _full_receiver) = sync_channel(0);
    process.control = Some(full_sender);
    assert_eq!(process.restart(identity(2)), Err(SubmitError::Saturated));

    let display = SubmitError::Saturated.to_string();
    assert_eq!(display, "language-server submission failed: Saturated");
    assert_eq!(
        SupervisorStopped.to_string(),
        "language-server supervisor stopped"
    );
}

#[test]
fn stale_events_are_discarded_before_disconnection_is_reported() -> Result<(), Box<dyn Error>> {
    let counters = Arc::new(Counters::default());
    let (sender, receiver) = sync_channel(EVENT_CAPACITY);
    assert!(emit(
        &sender,
        started(identity(1), ProcessEpoch(1)),
        &counters
    ));
    let failure = ProcessFailure::retained(ProcessStage::Output);
    assert!(emit(
        &sender,
        ProcessEvent::Failed {
            identity: identity(2),
            epoch: ProcessEpoch(2),
            failure,
        },
        &counters,
    ));
    drop(sender);
    let mut process = detached_process(None, receiver);
    process.counters = counters;
    process.identity = identity(2);
    process.epoch = ProcessEpoch(2);
    let event = process
        .try_event()?
        .ok_or("current process event was not delivered")?;
    assert_eq!(event.failure(), Some(failure));
    assert_eq!(process.snapshot().stale_events, 1);
    assert!(matches!(process.try_event(), Err(SupervisorStopped)));
    Ok(())
}

#[test]
fn pipe_wait_join_and_event_failures_are_structured() -> Result<(), Box<dyn Error>> {
    for stage in [
        ProcessStage::SpawnInput,
        ProcessStage::SpawnStdout,
        ProcessStage::SpawnStderr,
    ] {
        let failure = take_pipe::<u8>(None, stage)
            .err()
            .ok_or("missing pipe was accepted")?;
        assert_eq!(failure.stage, stage);
        assert_eq!(failure.kind, FailureKind::Io(io::ErrorKind::BrokenPipe));
    }
    let wait = classify_wait(
        identity(1),
        ProcessEpoch(1),
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "injected wait failure",
        )),
    );
    assert!(matches!(
        wait,
        Err(ProcessFailure {
            stage: ProcessStage::Wait,
            kind: FailureKind::Io(io::ErrorKind::PermissionDenied),
            ..
        })
    ));
    assert!(classify_wait(identity(1), ProcessEpoch(1), Ok(None))?.is_none());
    let mut helpers = vec![thread::spawn(|| {
        std::panic::resume_unwind(Box::new("injected helper panic"));
    })];
    assert!(join_helpers(&mut helpers));

    let counters = Counters::default();
    let (full_sender, _full_receiver) = sync_channel(0);
    assert!(!emit(
        &full_sender,
        started(identity(1), ProcessEpoch(1)),
        &counters
    ));
    let (closed_sender, closed_receiver) = sync_channel(1);
    drop(closed_receiver);
    assert!(!emit(
        &closed_sender,
        started(identity(1), ProcessEpoch(1)),
        &counters
    ));
    assert_eq!(counters.event_saturations.load(Ordering::Relaxed), 2);
    assert_eq!(counters.queued_events.load(Ordering::Relaxed), 0);
    Ok(())
}

#[test]
fn writer_reports_success_write_flush_and_result_queue_failures() -> Result<(), Box<dyn Error>> {
    for sink in [
        TestWriter::default(),
        TestWriter {
            write_error: Some(io::ErrorKind::BrokenPipe),
            flush_error: None,
        },
        TestWriter {
            write_error: None,
            flush_error: Some(io::ErrorKind::WriteZero),
        },
    ] {
        let counters = Arc::new(Counters::default());
        let (request_sender, request_receiver) = sync_channel(1);
        let (result_sender, result_receiver) = sync_channel(1);
        request_sender.send(WriteRequest {
            sequence: InputSequence(1),
            payload: Payload::copy(b"abc", &counters, ProcessStage::Input)?,
        })?;
        drop(request_sender);
        writer(sink, request_receiver, &result_sender);
        let result = result_receiver.recv_timeout(TIMEOUT)?;
        assert_eq!(result.sequence, InputSequence(1));
        assert_eq!(result.bytes, 3);
        assert_eq!(counters.retained_bytes.load(Ordering::Relaxed), 0);
    }

    let counters = Arc::new(Counters::default());
    let (request_sender, request_receiver) = sync_channel(1);
    let (result_sender, _result_receiver) = sync_channel(0);
    request_sender.send(WriteRequest {
        sequence: InputSequence(2),
        payload: Payload::copy(b"x", &counters, ProcessStage::Input)?,
    })?;
    drop(request_sender);
    writer(TestWriter::default(), request_receiver, &result_sender);
    assert_eq!(counters.retained_bytes.load(Ordering::Relaxed), 0);
    Ok(())
}

#[test]
fn reader_bounds_success_io_queue_and_retained_budget_paths() -> Result<(), Box<dyn Error>> {
    let counters = Arc::new(Counters::default());
    let overflowed = AtomicBool::new(false);
    let (sender, receiver) = sync_channel(1);
    reader(
        io::Cursor::new(b"abc"),
        ProcessStream::Stdout,
        &sender,
        &overflowed,
        &counters,
    );
    let packet = receiver.recv_timeout(TIMEOUT)?;
    assert_eq!(packet.stream, ProcessStream::Stdout);
    assert_eq!(&*packet.payload.bytes, b"abc");
    drop(packet);
    assert_eq!(counters.retained_bytes.load(Ordering::Relaxed), 0);

    reader(
        FailingReader,
        ProcessStream::Stderr,
        &sender,
        &overflowed,
        &counters,
    );
    assert!(!overflowed.load(Ordering::Relaxed));
    let (full_sender, _full_receiver) = sync_channel(0);
    reader(
        io::Cursor::new(b"x"),
        ProcessStream::Stdout,
        &full_sender,
        &overflowed,
        &counters,
    );
    assert!(overflowed.swap(false, Ordering::AcqRel));
    counters
        .retained_bytes
        .store(MAX_RETAINED_PAYLOAD_BYTES, Ordering::Release);
    reader(
        io::Cursor::new(b"x"),
        ProcessStream::Stdout,
        &sender,
        &overflowed,
        &counters,
    );
    assert!(overflowed.load(Ordering::Acquire));
    Ok(())
}

#[cfg(unix)]
#[test]
#[cfg_attr(miri, ignore = "Miri cannot emulate child-process creation")]
fn supervisor_and_helper_spawn_failures_are_injected_without_bypasses() -> Result<(), Box<dyn Error>>
{
    let failure = LanguageServerProcess::start_with_spawner(
        ProcessSpec::new("/bin/sleep", ["30"], None)?,
        identity(1),
        None,
        &FaultSpawner(ProcessStage::SpawnChild),
    )
    .err()
    .ok_or("supervisor spawn unexpectedly succeeded")?;
    assert_eq!(failure.stage, ProcessStage::SpawnChild);
    for stage in [
        ProcessStage::SpawnInput,
        ProcessStage::SpawnStdout,
        ProcessStage::SpawnStderr,
    ] {
        let counters = Arc::new(Counters::default());
        let result = spawn_process_with(
            &ProcessSpec::new("/bin/sleep", ["30"], None)?,
            identity(1),
            ProcessEpoch(1),
            &counters,
            &FaultSpawner(stage),
        );
        let failure = result.err().ok_or("helper spawn unexpectedly succeeded")?;
        assert_eq!(failure.stage, stage);
        assert_eq!(counters.retained_bytes.load(Ordering::Relaxed), 0);
    }
    let working_spec = ProcessSpec::new("/bin/sleep", ["30"], Some(&env::temp_dir()))?;
    let counters = Arc::new(Counters::default());
    let mut process = spawn_process(&working_spec, identity(1), ProcessEpoch(1), &counters)?;
    assert!(!stop_running(&mut process, true));
    Ok(())
}

#[test]
#[cfg_attr(miri, ignore = "Miri cannot emulate child-process creation")]
fn failed_child_start_rejects_queued_input_and_releases_payload() -> Result<(), Box<dyn Error>> {
    let mut spec = ProcessSpec::new(env::current_exe()?, std::iter::empty::<&str>(), None)?;
    spec.executable = PathBuf::from("/missing/alpine-language-server");
    let counters = Arc::new(Counters::default());
    let (control_sender, control_receiver) = sync_channel(1);
    let (event_sender, event_receiver) = sync_channel(EVENT_CAPACITY);
    control_sender.send(Control::Input {
        identity: identity(1),
        epoch: ProcessEpoch(1),
        sequence: InputSequence(1),
        payload: Payload::copy(b"x", &counters, ProcessStage::Input)?,
    })?;
    drop(control_sender);
    supervise(
        spec,
        identity(1),
        ProcessEpoch(1),
        control_receiver,
        event_sender,
        Arc::new(AtomicBool::new(false)),
        Arc::clone(&counters),
    );
    let events: Vec<_> = event_receiver.try_iter().collect();
    assert_eq!(events.len(), 2);
    assert!(matches!(
        events[0].failure(),
        Some(ProcessFailure {
            stage: ProcessStage::SpawnChild,
            ..
        })
    ));
    assert!(matches!(
        events[1].rejection(),
        Some((
            InputSequence(1),
            ProcessFailure {
                stage: ProcessStage::Input,
                ..
            }
        ))
    ));
    assert_eq!(counters.retained_bytes.load(Ordering::Relaxed), 0);
    Ok(())
}

#[cfg(unix)]
#[test]
#[cfg_attr(miri, ignore = "Miri cannot emulate child-process creation")]
fn input_admission_distinguishes_identity_full_disconnected_and_missing_sender()
-> Result<(), Box<dyn Error>> {
    let counters = Arc::new(Counters::default());
    let mut running = Some(spawn_process(
        &ProcessSpec::new("/bin/sleep", ["30"], None)?,
        identity(1),
        ProcessEpoch(1),
        &counters,
    )?);
    let make_request = |sequence| {
        Ok::<_, ProcessFailure>(WriteRequest {
            sequence: InputSequence(sequence),
            payload: Payload::copy(b"x", &counters, ProcessStage::Input)?,
        })
    };
    let stale = admit_input(&mut running, identity(2), ProcessEpoch(1), make_request(1)?)
        .err()
        .ok_or("stale input was admitted")?;
    assert!(!stale.2);
    running
        .as_mut()
        .ok_or("running process disappeared")?
        .input
        .take();
    let missing = admit_input(&mut running, identity(1), ProcessEpoch(1), make_request(2)?)
        .err()
        .ok_or("input without a sender was admitted")?;
    assert!(missing.2);
    let (full_sender, full_receiver) = sync_channel(0);
    running.as_mut().ok_or("running process disappeared")?.input = Some(full_sender);
    let full = admit_input(&mut running, identity(1), ProcessEpoch(1), make_request(3)?)
        .err()
        .ok_or("full input queue accepted a request")?;
    assert_eq!(full.1.kind, FailureKind::QueueSaturated);
    let (event_sender, event_receiver) = sync_channel(2);
    handle_input_control(
        &mut running,
        identity(1),
        ProcessEpoch(1),
        make_request(5)?,
        &event_sender,
        &counters,
    );
    assert!(matches!(
        event_receiver.recv_timeout(TIMEOUT)?.rejection(),
        Some((
            InputSequence(5),
            ProcessFailure {
                kind: FailureKind::QueueSaturated,
                ..
            }
        ))
    ));
    assert_eq!(counters.input_saturations.load(Ordering::Relaxed), 1);
    drop(full_receiver);
    let closed = admit_input(&mut running, identity(1), ProcessEpoch(1), make_request(4)?)
        .err()
        .ok_or("disconnected input queue accepted a request")?;
    assert_eq!(closed.1.kind, FailureKind::Io(io::ErrorKind::BrokenPipe));
    let mut process = running.take().ok_or("running process disappeared")?;
    assert!(!stop_running(&mut process, true));
    assert_eq!(counters.retained_bytes.load(Ordering::Relaxed), 0);
    Ok(())
}

#[cfg(unix)]
#[test]
#[cfg_attr(miri, ignore = "Miri cannot emulate child-process creation")]
fn restart_reports_panicked_helper_and_started_event_overflow() -> Result<(), Box<dyn Error>> {
    let counters = Arc::new(Counters::default());
    let mut process = spawn_process(
        &ProcessSpec::new("/bin/sleep", ["30"], None)?,
        identity(1),
        ProcessEpoch(1),
        &counters,
    )?;
    process.helpers.push(thread::spawn(|| {
        std::panic::resume_unwind(Box::new("injected restart helper panic"));
    }));
    let (event_sender, event_receiver) = sync_channel(4);
    stop_for_restart(process, &event_sender, &counters);
    let events: Vec<_> = event_receiver.try_iter().collect();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].stop_reason(), Some(StopReason::Restart));
    assert!(matches!(
        events[1].failure(),
        Some(ProcessFailure {
            kind: FailureKind::ThreadPanicked,
            ..
        })
    ));

    let (full_sender, _full_receiver) = sync_channel(0);
    assert!(
        start_running(
            &ProcessSpec::new("/bin/sleep", ["30"], None)?,
            identity(2),
            ProcessEpoch(2),
            &full_sender,
            &counters,
        )
        .is_none()
    );
    assert!(counters.event_saturations.load(Ordering::Relaxed) > 0);
    let mut missing = None;
    assert!(!stop_for_reason(
        &mut missing,
        StopReason::OutputOverflow,
        &event_sender,
        &counters,
    ));
    let mut running = Some(spawn_process(
        &ProcessSpec::new("/bin/sleep", ["30"], None)?,
        identity(3),
        ProcessEpoch(3),
        &counters,
    )?);
    assert!(stop_for_reason(
        &mut running,
        StopReason::EventOverflow,
        &event_sender,
        &counters,
    ));
    Ok(())
}

#[cfg(unix)]
#[test]
#[cfg_attr(miri, ignore = "Miri cannot emulate child-process creation")]
fn echo_restart_and_stale_events_are_bounded() -> Result<(), Box<dyn Error>> {
    let mut process = LanguageServerProcess::start(
        ProcessSpec::new("/bin/cat", std::iter::empty::<&str>(), None)?,
        identity(1),
    )?;
    let started = wait_for(&mut process, |event| {
        matches!(event, ProcessEvent::Started { .. })
    })?;
    assert!(matches!(
        started,
        ProcessEvent::Started {
            process_id: 1..,
            ..
        }
    ));
    let sequence = process.send(b"Content-Length: 2\r\n\r\n{}")?;
    assert_eq!(sequence.get(), 1);
    let mut written = false;
    let mut echoed = false;
    while !written || !echoed {
        let event = wait_for(&mut process, |_| true)?;
        match event {
            ProcessEvent::InputWritten {
                sequence: InputSequence(1),
                bytes: 23,
                ..
            } => written = true,
            output @ ProcessEvent::Output { .. } => {
                assert_eq!(
                    output.output(),
                    Some((ProcessStream::Stdout, &b"Content-Length: 2\r\n\r\n{}"[..]))
                );
                echoed = true;
            }
            unexpected => return Err(format!("unexpected echo event: {unexpected:?}").into()),
        }
    }
    assert_eq!(process.restart(identity(2))?.get(), 2);
    assert_eq!(process.restart(identity(2)), Err(SubmitError::StaleRestart));
    let _ = wait_for(
        &mut process,
        |event| matches!(event, ProcessEvent::Started { epoch, .. } if epoch.get() == 2),
    )?;
    assert!(process.snapshot().stale_events >= 1);
    let snapshot = process.shutdown();
    assert_eq!(snapshot.starts, 2);
    assert_eq!(snapshot.restarts, 1);
    assert_eq!(snapshot.written_inputs, 1);
    assert_eq!(snapshot.retained_bytes, 0);
    Ok(())
}

#[cfg(unix)]
#[test]
#[cfg_attr(miri, ignore = "Miri cannot emulate child-process creation")]
fn crash_is_structured_and_restart_recovers() -> Result<(), Box<dyn Error>> {
    let mut process = LanguageServerProcess::start(shell("exit 7")?, identity(1))?;
    let exited = wait_for(&mut process, |event| {
        matches!(event, ProcessEvent::Exited { .. })
    })?;
    assert!(matches!(
        exited,
        ProcessEvent::Exited {
            success: false,
            code: Some(7),
            ..
        }
    ));
    process.restart(identity(2))?;
    let restarted_exit = wait_for(
        &mut process,
        |event| matches!(event, ProcessEvent::Exited { epoch, .. } if epoch.get() == 2),
    )?;
    assert!(matches!(
        restarted_exit,
        ProcessEvent::Exited {
            success: false,
            code: Some(7),
            ..
        }
    ));
    let snapshot = process.shutdown();
    assert_eq!(snapshot.exits, 2);
    assert_eq!(snapshot.starts, 2);
    Ok(())
}

#[cfg(unix)]
#[test]
#[cfg_attr(miri, ignore = "Miri cannot emulate child-process creation")]
fn output_flood_terminates_without_unbounded_retention() -> Result<(), Box<dyn Error>> {
    let mut process =
        LanguageServerProcess::start(ProcessSpec::new("/usr/bin/yes", ["x"], None)?, identity(1))?;
    let deadline = Instant::now() + TIMEOUT;
    while Instant::now() < deadline {
        if process.try_event().is_err() {
            break;
        }
        if process.snapshot().output_saturations > 0 || process.snapshot().event_saturations > 0 {
            break;
        }
        thread::sleep(Duration::from_millis(2));
    }
    let snapshot = process.shutdown();
    assert!(snapshot.output_saturations > 0 || snapshot.event_saturations > 0);
    assert!(snapshot.peak_retained_bytes <= MAX_RETAINED_PAYLOAD_BYTES);
    assert!(snapshot.peak_queued_events <= EVENT_CAPACITY + 1);
    assert_eq!(snapshot.retained_bytes, 0);
    Ok(())
}

#[test]
fn event_observers_preserve_rejection_stop_and_failure_details() -> Result<(), Box<dyn Error>> {
    let identity = identity(1);
    let epoch = ProcessEpoch(1);
    let rejection = ProcessFailure::saturated(ProcessStage::Input);
    let rejected = ProcessEvent::InputRejected {
        identity,
        epoch,
        sequence: InputSequence(7),
        failure: rejection,
    };
    assert_eq!(rejected.rejection(), Some((InputSequence(7), rejection)));
    assert_eq!(rejected.identity(), identity);
    assert_eq!(rejected.epoch(), epoch);
    assert_eq!(rejected.output(), None);
    assert_eq!(rejected.stop_reason(), None);
    assert_eq!(rejected.failure(), None);
    let stopped = ProcessEvent::Stopped {
        identity,
        epoch,
        reason: StopReason::Restart,
    };
    assert_eq!(stopped.stop_reason(), Some(StopReason::Restart));
    assert_eq!(stopped.identity(), identity);
    assert_eq!(stopped.epoch(), epoch);
    assert_eq!(stopped.rejection(), None);
    let failure = ProcessFailure::panicked();
    let failed = ProcessEvent::Failed {
        identity,
        epoch,
        failure,
    };
    assert_eq!(failed.failure(), Some(failure));
    assert_eq!(failed.stop_reason(), None);

    let counters = Arc::new(Counters::default());
    let output = ProcessEvent::Output {
        identity,
        epoch,
        stream: ProcessStream::Stderr,
        payload: Payload::copy(b"stderr", &counters, ProcessStage::Output)?,
    };
    assert_eq!(output.identity(), identity);
    assert_eq!(output.epoch(), epoch);
    assert_eq!(
        output.output(),
        Some((ProcessStream::Stderr, &b"stderr"[..]))
    );
    let written = ProcessEvent::InputWritten {
        identity,
        epoch,
        sequence: InputSequence(9),
        bytes: 4,
    };
    assert_eq!(written.identity(), identity);
    assert_eq!(written.epoch(), epoch);
    let exited = ProcessEvent::Exited {
        identity,
        epoch,
        success: true,
        code: Some(0),
    };
    assert_eq!(exited.identity(), identity);
    assert_eq!(exited.epoch(), epoch);
    Ok(())
}

#[cfg(unix)]
#[test]
#[cfg_attr(miri, ignore = "Miri cannot emulate child-process creation")]
fn blocked_input_is_nonblocking_and_shutdown_releases_payloads() -> Result<(), Box<dyn Error>> {
    let mut process =
        LanguageServerProcess::start(ProcessSpec::new("/bin/sleep", ["30"], None)?, identity(1))?;
    let _ = wait_for(&mut process, |event| {
        matches!(event, ProcessEvent::Started { .. })
    })?;
    let payload = vec![b'x'; MAX_MESSAGE_BYTES / 2];
    let start = Instant::now();
    let mut admitted = 0;
    let mut rejected = 0;
    for _ in 0..16 {
        match process.send(&payload) {
            Ok(_) => admitted += 1,
            Err(SubmitError::RetainedBudget | SubmitError::Saturated) => rejected += 1,
            Err(error) => return Err(error.into()),
        }
    }
    assert!(start.elapsed() < Duration::from_secs(1));
    assert!(admitted > 0 && rejected > 0);
    let snapshot = process.shutdown();
    assert!(snapshot.peak_retained_bytes <= MAX_RETAINED_PAYLOAD_BYTES);
    assert_eq!(snapshot.retained_bytes, 0);
    Ok(())
}

#[test]
fn supervisor_decision_helpers_cover_every_transition() {
    assert!(matches!(
        merge_stop_reason(None, Some(StopReason::EventOverflow)),
        Some(StopReason::EventOverflow)
    ));
    assert!(matches!(
        merge_stop_reason(Some(StopReason::EventOverflow), None),
        Some(StopReason::EventOverflow)
    ));

    assert!(matches!(
        interpret_wait(WaitDecision::Running),
        (false, None)
    ));
    assert!(matches!(interpret_wait(WaitDecision::Exited), (true, None)));
    assert!(matches!(
        interpret_wait(WaitDecision::Terminate),
        (false, Some(StopReason::OutputOverflow))
    ));
}

#[test]
fn exact_limits_and_sequence_observers_are_independently_discriminating()
-> Result<(), Box<dyn Error>> {
    assert_eq!(InputSequence(7).get(), 7);
    assert!(validate_path(&PathBuf::from("x".repeat(MAX_PATH_BYTES))).is_ok());
    assert_eq!(
        validate_path(&PathBuf::from("x".repeat(MAX_PATH_BYTES + 1))),
        Err(ConfigError::PathTooLong)
    );

    let (control_sender, control_receiver) = sync_channel(1);
    let (_event_sender, event_receiver) = sync_channel(1);
    let mut process = detached_process(Some(control_sender), event_receiver);
    let payload = vec![b'x'; MAX_MESSAGE_BYTES];
    let sequence = process.send(&payload)?;
    assert_eq!(sequence.get(), 1);
    match control_receiver.recv_timeout(TIMEOUT)? {
        Control::Input { payload, .. } => drop(payload),
        Control::Restart { .. } => return Err("message boundary queued a restart".into()),
    }
    assert_eq!(process.snapshot().retained_bytes, 0);
    Ok(())
}

#[test]
fn stale_event_filter_checks_identity_and_epoch_independently() -> Result<(), Box<dyn Error>> {
    let counters = Arc::new(Counters::default());
    let (sender, receiver) = sync_channel(EVENT_CAPACITY);
    assert!(emit(
        &sender,
        started(identity(2), ProcessEpoch(1)),
        &counters
    ));
    assert!(emit(
        &sender,
        started(identity(1), ProcessEpoch(2)),
        &counters
    ));
    assert!(emit(
        &sender,
        started(identity(2), ProcessEpoch(2)),
        &counters
    ));
    let mut process = detached_process(None, receiver);
    process.counters = counters;
    process.identity = identity(2);
    process.epoch = ProcessEpoch(2);
    assert!(matches!(
        process.try_event()?,
        Some(ProcessEvent::Started {
            identity: event_identity,
            epoch,
            ..
        }) if event_identity == identity(2) && epoch == ProcessEpoch(2)
    ));
    assert_eq!(process.snapshot().stale_events, 2);
    assert_eq!(process.snapshot().peak_queued_events, 3);
    Ok(())
}

#[test]
fn drop_signals_and_joins_the_owned_supervisor() {
    let shutdown = Arc::new(AtomicBool::new(false));
    let observed = Arc::new(AtomicBool::new(false));
    let worker_shutdown = Arc::clone(&shutdown);
    let worker_observed = Arc::clone(&observed);
    let (completion_sender, supervisor_complete) = sync_channel(1);
    let supervisor = thread::spawn(move || {
        while !worker_shutdown.load(Ordering::Acquire) {
            thread::yield_now();
        }
        worker_observed.store(true, Ordering::Release);
        let _ = completion_sender.try_send(());
    });
    let (_event_sender, events) = sync_channel(1);
    let process = LanguageServerProcess {
        control: None,
        events,
        supervisor: Some(supervisor),
        supervisor_complete,
        shutdown,
        counters: Arc::new(Counters::default()),
        configuration_bytes: 0,
        identity: identity(1),
        epoch: ProcessEpoch(1),
        next_sequence: 0,
    };
    drop(process);
    assert!(observed.load(Ordering::Acquire));
}

#[test]
fn bounded_supervisor_join_reports_an_unfinished_worker() {
    let release = Arc::new(AtomicBool::new(false));
    let finished = Arc::new(AtomicBool::new(false));
    let worker_release = Arc::clone(&release);
    let worker_finished = Arc::clone(&finished);
    let supervisor = thread::spawn(move || {
        while !worker_release.load(Ordering::Acquire) {
            thread::yield_now();
        }
        worker_finished.store(true, Ordering::Release);
    });
    let (_completion_sender, completion) = sync_channel(1);
    let counters = Counters::default();
    join_supervisor(supervisor, &completion, Duration::ZERO, &counters);
    assert_eq!(counters.shutdown_timeouts.load(Ordering::Relaxed), 1);
    release.store(true, Ordering::Release);
    let deadline = Instant::now() + TIMEOUT;
    while !finished.load(Ordering::Acquire) && Instant::now() < deadline {
        thread::yield_now();
    }
    assert!(finished.load(Ordering::Acquire));
}

#[cfg(unix)]
#[test]
#[cfg_attr(miri, ignore = "Miri cannot emulate child-process creation")]
fn supervisor_exits_after_terminal_event_overflow() -> Result<(), Box<dyn Error>> {
    let spec = ProcessSpec::new("/usr/bin/yes", ["x"], None)?;
    let (control_sender, control_receiver) = sync_channel(1);
    let (event_sender, _event_receiver) = sync_channel(1);
    let shutdown = Arc::new(AtomicBool::new(false));
    let counters = Arc::new(Counters::default());
    let worker_shutdown = Arc::clone(&shutdown);
    let worker_counters = Arc::clone(&counters);
    let (done_sender, done_receiver) = sync_channel(1);
    let supervisor = thread::spawn(move || {
        supervise(
            spec,
            identity(1),
            ProcessEpoch(1),
            control_receiver,
            event_sender,
            worker_shutdown,
            worker_counters,
        );
        let _ = done_sender.try_send(());
    });
    let completed = done_receiver
        .recv_timeout(Duration::from_millis(250))
        .is_ok();
    shutdown.store(true, Ordering::Release);
    drop(control_sender);
    if !completed {
        let _ = done_receiver.recv_timeout(TIMEOUT);
    }
    let _ = supervisor.join();
    assert!(completed);
    assert!(counters.event_saturations.load(Ordering::Relaxed) > 0);
    assert_eq!(counters.retained_bytes.load(Ordering::Relaxed), 0);
    Ok(())
}

#[cfg(unix)]
#[test]
#[cfg_attr(miri, ignore = "Miri cannot emulate child-process creation")]
fn failed_spawn_cleanup_reaps_the_owned_child() -> Result<(), Box<dyn Error>> {
    let mut child = Command::new("sh").args(["-c", "sleep 30"]).spawn()?;
    let (input, input_receiver) = sync_channel::<WriteRequest>(1);
    let helper = thread::spawn(move || drop(input_receiver));
    cleanup_failed_spawn(&mut child, input, vec![helper]);
    let reaped = child.try_wait()?.is_some();
    if !reaped {
        let _ = child.kill();
        let _ = child.wait();
    }
    assert!(reaped);
    Ok(())
}

struct SharedWriteCounter(Arc<AtomicUsize>);

impl io::Write for SharedWriteCounter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.fetch_add(1, Ordering::Relaxed);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn writer_stops_after_losing_the_result_queue() -> Result<(), Box<dyn Error>> {
    let counters = Arc::new(Counters::default());
    let writes = Arc::new(AtomicUsize::new(0));
    let (request_sender, request_receiver) = sync_channel(2);
    for sequence in [1, 2] {
        request_sender.send(WriteRequest {
            sequence: InputSequence(sequence),
            payload: Payload::copy(b"x", &counters, ProcessStage::Input)?,
        })?;
    }
    drop(request_sender);
    let (result_sender, _result_receiver) = sync_channel(0);
    writer(
        SharedWriteCounter(Arc::clone(&writes)),
        request_receiver,
        &result_sender,
    );
    assert_eq!(writes.load(Ordering::Relaxed), 1);
    assert_eq!(counters.retained_bytes.load(Ordering::Relaxed), 0);
    Ok(())
}

#[test]
fn ordinary_events_reserve_exact_capacity_for_terminal_classification() {
    let (sender, receiver) = sync_channel(EVENT_CAPACITY);
    let counters = Counters::default();
    let started = |process_id| ProcessEvent::Started {
        identity: identity(1),
        epoch: ProcessEpoch(1),
        process_id,
    };

    for _ in 0..EVENT_CAPACITY - TERMINAL_EVENT_RESERVE {
        assert!(emit(&sender, started(1), &counters));
    }
    assert!(!emit(&sender, started(u32::MAX), &counters));
    assert_eq!(
        counters.queued_events.load(Ordering::Acquire),
        EVENT_CAPACITY - TERMINAL_EVENT_RESERVE
    );

    for reason in [StopReason::OutputOverflow, StopReason::EventOverflow] {
        assert!(emit_terminal(
            &sender,
            ProcessEvent::Stopped {
                identity: identity(1),
                epoch: ProcessEpoch(1),
                reason,
            },
            &counters,
        ));
    }
    assert!(!emit_terminal(
        &sender,
        ProcessEvent::Stopped {
            identity: identity(1),
            epoch: ProcessEpoch(1),
            reason: StopReason::Restart,
        },
        &counters,
    ));
    assert_eq!(
        counters.queued_events.load(Ordering::Acquire),
        EVENT_CAPACITY
    );
    assert_eq!(counters.event_saturations.load(Ordering::Relaxed), 2);

    let events = receiver.try_iter().collect::<Vec<_>>();
    assert_eq!(events.len(), EVENT_CAPACITY);
    assert_eq!(
        events[EVENT_CAPACITY - 2].stop_reason(),
        Some(StopReason::OutputOverflow)
    );
    assert_eq!(
        events[EVENT_CAPACITY - 1].stop_reason(),
        Some(StopReason::EventOverflow)
    );
}
