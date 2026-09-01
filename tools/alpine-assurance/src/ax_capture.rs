//! Captures bounded native AX client evidence for the physical orchestrator.

use alpine_ax_client::{
    AxAction, AxClient, AxClientFactory, AxEventBatch, AxGeneration, AxLimits, AxNode,
    AxNotificationKind, NativeAxClientFactory,
};
use serde::Serialize;
use std::{
    fs::{self, File},
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const MAX_PHASE_MS: u64 = 120_000;
const DRAIN_SLICE: Duration = Duration::from_millis(250);

macro_rules! capture_try {
    ($result:expr, $message:literal $(, $argument:expr)*) => {
        match $result {
            Ok(value) => value,
            Err(error) => return Err(format!($message $(, $argument)*, error = error)),
        }
    };
}

#[derive(Serialize)]
struct TreeRow<'a> {
    sequence: u64,
    depth: u16,
    identifier: &'a str,
    parent_identifier: Option<&'a str>,
    role: &'a str,
    label: &'a str,
    focused: bool,
}

#[derive(Serialize)]
struct EventRow<'a> {
    sequence: u64,
    monotonic_ns: u64,
    source: &'a str,
    kind: &'a str,
    identifier: &'a str,
    detail: &'a str,
    ax_error: i32,
}

#[derive(Serialize)]
struct LatencyRow<'a> {
    sequence: u64,
    operation: &'a str,
    identifier: &'a str,
    start_ns: u64,
    end_ns: u64,
    ax_error: i32,
}

struct CaptureRows {
    tree: Vec<String>,
    events: Vec<String>,
    latency: Vec<String>,
}

pub(crate) fn run_native(
    pid: i32,
    generation: u64,
    pre_action_ms: u64,
    post_action_ms: u64,
    output: &Path,
) -> Result<String, Vec<String>> {
    #[cfg(target_os = "macos")]
    let factory = NativeAxClientFactory::default();
    #[cfg(not(target_os = "macos"))]
    let factory = NativeAxClientFactory;
    match run_with_factory(
        &factory,
        pid,
        generation,
        pre_action_ms,
        post_action_ms,
        output,
    ) {
        Ok(message) => Ok(message),
        Err(error) => Err(vec![error]),
    }
}

fn run_with_factory<F: AxClientFactory>(
    factory: &F,
    pid: i32,
    generation: u64,
    pre_action_ms: u64,
    post_action_ms: u64,
    output: &Path,
) -> Result<String, String> {
    if pid <= 0 {
        return Err("capture PID must be positive".to_owned());
    }
    validate_phase_durations(pre_action_ms, post_action_ms)?;
    let trusted = capture_try!(factory.is_trusted(), "cannot query AX trust: {error}");
    if !trusted {
        return Err(
            "AX client is not trusted; no prompt or privacy mutation was attempted".to_owned(),
        );
    }
    let generation = capture_try!(AxGeneration::new(generation), "{error}");
    let limits = capture_try!(
        AxLimits::new(271, 65_536, 65_536, 128, 1_048_576, Duration::from_secs(5)),
        "{error}"
    );
    let mut client = capture_try!(
        factory.attach(pid, generation, limits),
        "cannot attach AX client to PID {pid}: {error}"
    );
    let started = Instant::now();
    let query_start = elapsed_ns(started);
    let nodes = capture_try!(client.snapshot_tree(), "cannot snapshot AX tree: {error}");
    let query_end = later_than(query_start, elapsed_ns(started));
    let action = select_action(&nodes)?;
    capture_try!(
        client.retain_for_stale_query(generation, &action.0.identifier),
        "cannot retain stale AX control: {error}"
    );

    let pre_events = drain_for(
        &mut client,
        generation,
        Duration::from_millis(pre_action_ms),
    )?;
    let action_start = elapsed_ns(started);
    let action_error = capture_try!(
        client.perform_action(generation, &action.0.identifier, action.1),
        "cannot perform AX action: {error}"
    );
    let action_end = later_than(action_start, elapsed_ns(started));
    let notification_start = elapsed_ns(started);
    let post_events = drain_for(
        &mut client,
        generation,
        Duration::from_millis(post_action_ms),
    )?;
    let notification_end = later_than(notification_start, elapsed_ns(started));
    let stale_start = elapsed_ns(started);
    let stale = capture_try!(
        client.query_retained_stale(generation),
        "cannot query retained stale AX control: {error}"
    );
    let stale_end = later_than(stale_start, elapsed_ns(started));
    let close_start = elapsed_ns(started);
    capture_try!(client.close(generation), "cannot close AX client: {error}");
    let close_end = later_than(close_start, elapsed_ns(started));

    let rows = render_rows(
        &nodes,
        (&action.0.identifier, action.1, action_error),
        &pre_events,
        &post_events,
        stale.ax_error,
        [
            (query_start, query_end),
            (action_start, action_end),
            (notification_start, notification_end),
            (stale_start, stale_end),
            (close_start, close_end),
        ],
    )?;
    publish(output, &rows)?;
    Ok(format!(
        "captured bounded AX client evidence for PID {pid}, generation {}, {} nodes, and {} observer events at {}",
        generation.get(),
        nodes.len(),
        pre_events
            .events
            .len()
            .saturating_add(post_events.events.len()),
        output.display()
    ))
}

fn validate_phase_durations(pre_action_ms: u64, post_action_ms: u64) -> Result<(), String> {
    if pre_action_ms == 0
        || post_action_ms == 0
        || pre_action_ms > MAX_PHASE_MS
        || post_action_ms > MAX_PHASE_MS
    {
        return Err(format!(
            "capture phase durations must be between 1 and {MAX_PHASE_MS} milliseconds"
        ));
    }
    Ok(())
}

fn select_action(nodes: &[AxNode]) -> Result<(&AxNode, AxAction), String> {
    for action in [AxAction::Confirm, AxAction::ShowMenu, AxAction::Press] {
        if let Some(node) = nodes.iter().find(|node| {
            !node.identifier.to_ascii_lowercase().contains("close")
                && node
                    .enabled_actions
                    .iter()
                    .any(|candidate| candidate == action.native_name())
        }) {
            return Ok((node, action));
        }
    }
    Err("AX snapshot has no allowlisted non-close action".to_owned())
}

fn drain_for<C: AxClient>(
    client: &mut C,
    generation: AxGeneration,
    duration: Duration,
) -> Result<AxEventBatch, String> {
    let started = Instant::now();
    let mut batch = AxEventBatch {
        events: Vec::new(),
        omitted_events: 0,
        stale_events: 0,
    };
    while let Some(remaining) = duration.checked_sub(started.elapsed()) {
        if remaining.is_zero() {
            break;
        }
        let slice = remaining.min(DRAIN_SLICE);
        let next = capture_try!(
            client.drain_events(generation, slice),
            "cannot drain AX observer: {error}"
        );
        batch.events.extend(next.events);
        batch.omitted_events = batch.omitted_events.saturating_add(next.omitted_events);
        batch.stale_events = batch.stale_events.saturating_add(next.stale_events);
    }
    if batch.omitted_events != 0 || batch.stale_events != 0 {
        return Err(format!(
            "AX observer omitted {} events and rejected {} stale events",
            batch.omitted_events, batch.stale_events
        ));
    }
    Ok(batch)
}

fn render_rows(
    nodes: &[AxNode],
    action: (&str, AxAction, i32),
    pre_events: &AxEventBatch,
    post_events: &AxEventBatch,
    stale_error: i32,
    latency: [(u64, u64); 5],
) -> Result<CaptureRows, String> {
    let mut tree = Vec::with_capacity(nodes.len());
    for (index, node) in nodes.iter().enumerate() {
        tree.push(json(&TreeRow {
            sequence: sequence(index)?,
            depth: node.depth,
            identifier: &node.identifier,
            parent_identifier: node.parent_identifier.as_deref(),
            role: &node.role,
            label: &node.label,
            focused: node.focused,
        })?);
    }

    let mut events = Vec::new();
    let mut timestamp = 0_u64;
    let action_timestamp = timestamp.saturating_add(1);
    push_event(
        &mut events,
        &mut timestamp,
        "process",
        "launch",
        "application",
        "process-start",
        0,
        1,
    )?;
    for observed in &pre_events.events {
        push_observed(&mut events, &mut timestamp, observed)?;
    }
    push_event(
        &mut events,
        &mut timestamp,
        "ax-action",
        "action",
        action.0,
        action.1.native_name(),
        action.2,
        action_timestamp,
    )?;
    for observed in &post_events.events {
        push_observed(&mut events, &mut timestamp, observed)?;
    }
    let stale_timestamp = timestamp.saturating_add(1);
    push_event(
        &mut events,
        &mut timestamp,
        "ax-query",
        "stale-control",
        action.0,
        if stale_error == -25_211 {
            "kAXErrorInvalidUIElement"
        } else {
            "unexpected-stale-result"
        },
        stale_error,
        stale_timestamp,
    )?;
    let operations = ["query", "action", "notification", "stale-query", "close"];
    let identifiers = ["application", action.0, action.0, action.0, "application"];
    let errors = [0, action.2, 0, stale_error, 0];
    let mut latency_rows = Vec::with_capacity(operations.len());
    for index in 0..operations.len() {
        latency_rows.push(json(&LatencyRow {
            sequence: sequence(index)?,
            operation: operations[index],
            identifier: identifiers[index],
            start_ns: latency[index].0,
            end_ns: latency[index].1,
            ax_error: errors[index],
        })?);
    }
    Ok(CaptureRows {
        tree,
        events,
        latency: latency_rows,
    })
}

fn push_observed(
    rows: &mut Vec<String>,
    timestamp: &mut u64,
    observed: &alpine_ax_client::AxObservedEvent,
) -> Result<(), String> {
    let kind = match observed.kind {
        AxNotificationKind::Focus => "focus",
        AxNotificationKind::Value => "value",
        AxNotificationKind::Selection => "selection",
        AxNotificationKind::Layout => "layout",
        AxNotificationKind::Announcement => "announcement",
        AxNotificationKind::Minimized => "minimized",
        AxNotificationKind::Restored => "restored",
        AxNotificationKind::Destroyed => "destroyed",
    };
    push_event(
        rows,
        timestamp,
        "ax-observer",
        kind,
        &observed.identifier,
        observed.kind.native_name(),
        0,
        observed.monotonic_ns,
    )
}

#[allow(clippy::too_many_arguments, reason = "one exact evidence record")]
fn push_event(
    rows: &mut Vec<String>,
    timestamp: &mut u64,
    source: &str,
    kind: &str,
    identifier: &str,
    detail: &str,
    ax_error: i32,
    candidate_ns: u64,
) -> Result<(), String> {
    *timestamp = later_than(*timestamp, candidate_ns);
    rows.push(json(&EventRow {
        sequence: match u64::try_from(rows.len()) {
            Ok(value) => value.saturating_add(1),
            Err(_) => return Err("AX event sequence overflow".to_owned()),
        },
        monotonic_ns: *timestamp,
        source,
        kind,
        identifier,
        detail,
        ax_error,
    })?);
    Ok(())
}

fn publish(output: &Path, rows: &CaptureRows) -> Result<(), String> {
    if output.exists() {
        return Err(format!(
            "AX capture output already exists: {}",
            output.display()
        ));
    }
    let parent = output.parent().unwrap_or_else(|| Path::new("."));
    capture_try!(
        fs::create_dir_all(parent),
        "cannot create AX capture parent: {error}"
    );
    let staging = staging_path(output);
    if staging.exists() {
        return Err(format!(
            "AX capture staging path already exists: {}",
            staging.display()
        ));
    }
    capture_try!(
        fs::create_dir(&staging),
        "cannot create AX capture staging directory: {error}"
    );
    let result = (|| {
        write_rows(&staging.join("tree.jsonl"), &rows.tree)?;
        write_rows(&staging.join("events.jsonl"), &rows.events)?;
        write_rows(&staging.join("latency.jsonl"), &rows.latency)?;
        match fs::rename(&staging, output) {
            Ok(()) => Ok(()),
            Err(error) => Err(format!("cannot publish AX capture atomically: {error}")),
        }
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    result
}

fn write_rows(path: &Path, rows: &[String]) -> Result<(), String> {
    let file = capture_try!(
        File::create(path),
        "cannot create {}: {error}",
        path.display()
    );
    let mut writer = BufWriter::new(file);
    for row in rows {
        capture_try!(
            writeln!(writer, "{row}"),
            "cannot write {}: {error}",
            path.display()
        );
    }
    match writer.flush() {
        Ok(()) => Ok(()),
        Err(error) => Err(format!("cannot flush {}: {error}", path.display())),
    }
}

fn staging_path(output: &Path) -> PathBuf {
    let name = output
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("ax-capture");
    output.with_file_name(format!(".{name}.staging-{}", std::process::id()))
}

fn json<T: Serialize>(value: &T) -> Result<String, String> {
    match serde_json::to_string(value) {
        Ok(encoded) => Ok(encoded),
        Err(error) => Err(format!("cannot serialize AX evidence: {error}")),
    }
}

fn sequence(index: usize) -> Result<u64, String> {
    match u64::try_from(index) {
        Ok(value) => Ok(value.saturating_add(1)),
        Err(_) => Err("AX record sequence overflow".to_owned()),
    }
}

fn elapsed_ns(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX)
}

const fn later_than(previous: u64, candidate: u64) -> u64 {
    if candidate > previous {
        candidate
    } else {
        previous.saturating_add(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alpine_ax_client::{
        AxClientError, AxEventBatch, AxObservedEvent, AxQueryResult, AxRect, AxTextRange,
    };

    struct FakeFactory {
        trusted: bool,
    }

    impl AxClientFactory for FakeFactory {
        type Client = FakeClient;

        fn is_trusted(&self) -> Result<bool, AxClientError> {
            Ok(self.trusted)
        }

        fn attach(
            &self,
            _pid: i32,
            generation: AxGeneration,
            _limits: AxLimits,
        ) -> Result<Self::Client, AxClientError> {
            Ok(FakeClient {
                generation,
                drains: 0,
                closed: false,
                omitted_events: 0,
                stale_events: 0,
            })
        }
    }

    struct FakeClient {
        generation: AxGeneration,
        drains: usize,
        closed: bool,
        omitted_events: usize,
        stale_events: usize,
    }

    impl AxClient for FakeClient {
        fn generation(&self) -> AxGeneration {
            self.generation
        }

        fn snapshot_tree(&mut self) -> Result<Vec<AxNode>, AxClientError> {
            Ok(vec![
                node("application", None, 0, "AXApplication", false, &[]),
                node("window", Some("application"), 1, "AXWindow", false, &[]),
                node(
                    "editor",
                    Some("window"),
                    2,
                    "AXTextArea",
                    true,
                    &["AXConfirm"],
                ),
            ])
        }

        fn drain_events(
            &mut self,
            generation: AxGeneration,
            _timeout: Duration,
        ) -> Result<AxEventBatch, AxClientError> {
            assert_eq!(generation, self.generation);
            self.drains = self.drains.saturating_add(1);
            let events = match self.drains {
                1 => vec![observed(self.generation, AxNotificationKind::Focus, 2)],
                2 => vec![observed(self.generation, AxNotificationKind::Value, 3)],
                _ => Vec::new(),
            };
            Ok(AxEventBatch {
                events,
                omitted_events: self.omitted_events,
                stale_events: self.stale_events,
            })
        }

        fn perform_action(
            &mut self,
            generation: AxGeneration,
            identifier: &str,
            action: AxAction,
        ) -> Result<i32, AxClientError> {
            assert_eq!(generation, self.generation);
            assert_eq!(identifier, "editor");
            assert_eq!(action, AxAction::Confirm);
            Ok(0)
        }

        fn retain_for_stale_query(
            &mut self,
            generation: AxGeneration,
            identifier: &str,
        ) -> Result<(), AxClientError> {
            assert_eq!(generation, self.generation);
            assert_eq!(identifier, "editor");
            Ok(())
        }

        fn query_retained_stale(
            &mut self,
            generation: AxGeneration,
        ) -> Result<AxQueryResult, AxClientError> {
            assert_eq!(generation, self.generation);
            Ok(AxQueryResult { ax_error: -25_211 })
        }

        fn close(&mut self, generation: AxGeneration) -> Result<(), AxClientError> {
            assert_eq!(generation, self.generation);
            assert!(!self.closed);
            self.closed = true;
            Ok(())
        }
    }

    fn node(
        identifier: &str,
        parent_identifier: Option<&str>,
        depth: u16,
        role: &str,
        focused: bool,
        actions: &[&str],
    ) -> AxNode {
        AxNode {
            identifier: identifier.to_owned(),
            parent_identifier: parent_identifier.map(str::to_owned),
            depth,
            role: role.to_owned(),
            label: identifier.to_owned(),
            value: None,
            focused,
            selected_text: None,
            selected_range: Some(AxTextRange {
                location: 0,
                length: 0,
            }),
            frame: Some(AxRect {
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
            }),
            enabled_actions: actions.iter().map(|value| (*value).to_owned()).collect(),
        }
    }

    fn observed(
        generation: AxGeneration,
        kind: AxNotificationKind,
        monotonic_ns: u64,
    ) -> AxObservedEvent {
        AxObservedEvent {
            generation,
            kind,
            identifier: "editor".to_owned(),
            monotonic_ns,
        }
    }

    fn output(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
            "../../target/ax-capture-{name}-{}",
            std::process::id()
        ))
    }

    #[test]
    fn capture_is_trusted_bounded_and_published_without_replacement() -> Result<(), String> {
        let output = output("valid");
        let _ = fs::remove_dir_all(&output);
        let message = run_with_factory(&FakeFactory { trusted: true }, 42, 1, 1, 1, &output)?;
        assert!(message.contains("3 nodes"));
        let tree = fs::read_to_string(output.join("tree.jsonl")).map_err(|e| e.to_string())?;
        let events = fs::read_to_string(output.join("events.jsonl")).map_err(|e| e.to_string())?;
        let latency =
            fs::read_to_string(output.join("latency.jsonl")).map_err(|e| e.to_string())?;
        assert_eq!(tree.lines().count(), 3);
        assert!(tree.contains("\"sequence\":1"));
        assert!(tree.contains("\"sequence\":2"));
        assert!(tree.contains("\"sequence\":3"));
        assert!(events.contains("\"kind\":\"focus\""));
        assert!(events.contains("\"detail\":\"AXConfirm\""));
        assert!(events.contains("\"detail\":\"kAXErrorInvalidUIElement\""));
        assert!(events.contains("\"ax_error\":-25211"));
        assert_eq!(latency.lines().count(), 5);
        assert!(run_with_factory(&FakeFactory { trusted: true }, 42, 1, 1, 1, &output).is_err());
        fs::remove_dir_all(output).map_err(|error| error.to_string())
    }

    #[test]
    fn untrusted_invalid_and_unactionable_captures_fail_closed() {
        let output = output("invalid");
        assert!(validate_phase_durations(1, 1).is_ok());
        assert!(validate_phase_durations(MAX_PHASE_MS, MAX_PHASE_MS).is_ok());
        assert!(validate_phase_durations(0, 1).is_err());
        assert!(validate_phase_durations(1, 0).is_err());
        assert!(validate_phase_durations(MAX_PHASE_MS + 1, 1).is_err());
        assert!(validate_phase_durations(1, MAX_PHASE_MS + 1).is_err());
        assert!(run_with_factory(&FakeFactory { trusted: false }, 42, 1, 1, 1, &output).is_err());
        assert!(run_with_factory(&FakeFactory { trusted: true }, 0, 1, 1, 1, &output).is_err());
        assert!(run_with_factory(&FakeFactory { trusted: true }, 42, 0, 1, 1, &output).is_err());
        assert!(run_with_factory(&FakeFactory { trusted: true }, 42, 1, 0, 1, &output).is_err());
        assert!(
            run_with_factory(
                &FakeFactory { trusted: true },
                42,
                1,
                MAX_PHASE_MS + 1,
                1,
                &output,
            )
            .is_err()
        );
    }

    #[test]
    fn observer_loss_and_staleness_fail_closed_independently() -> Result<(), String> {
        let generation = AxGeneration::new(1).map_err(|error| error.to_string())?;
        for (omitted_events, stale_events) in [(1, 0), (0, 1)] {
            let mut client = FakeClient {
                generation,
                drains: 0,
                closed: false,
                omitted_events,
                stale_events,
            };
            assert!(drain_for(&mut client, generation, Duration::from_millis(1)).is_err());
        }
        Ok(())
    }

    #[test]
    fn record_order_and_time_helpers_preserve_strict_boundaries() {
        assert_eq!(sequence(0), Ok(1));
        assert_eq!(sequence(1), Ok(2));
        assert_eq!(later_than(5, 10), 10);
        assert_eq!(later_than(5, 5), 6);
        assert_eq!(later_than(5, 4), 6);
        assert_eq!(later_than(0, 0), 1);
        let started = Instant::now();
        std::thread::sleep(Duration::from_millis(1));
        assert!(elapsed_ns(started) > 1);
    }
}
