use std::{
    collections::BTreeMap,
    env,
    fs::{File, OpenOptions},
    io::{self, Read, Write},
    path::PathBuf,
    process, thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const TRACE_ENVIRONMENT: &str = "ALPINE_STUDIO_NATIVE_LSP_TRACE";

struct PhaseTrace {
    file: Option<File>,
    startup_timing: Option<File>,
}

impl PhaseTrace {
    fn from_environment() -> io::Result<Self> {
        let file = env::var_os(TRACE_ENVIRONMENT)
            .map(PathBuf::from)
            .map(|path| OpenOptions::new().create(true).append(true).open(path))
            .transpose()?;
        let startup_timing = env::var_os("ALPINE_STUDIO_LSP_STARTUP_TIMING")
            .map(PathBuf::from)
            .map(|path| OpenOptions::new().append(true).open(path))
            .transpose()?;
        Ok(Self {
            file,
            startup_timing,
        })
    }

    fn record(&mut self, phase: &str) -> io::Result<()> {
        if let Some(file) = self.file.as_mut() {
            let record = format!("{phase}\n");
            file.write_all(record.as_bytes())?;
        }
        if let Some(file) = self.startup_timing.as_mut() {
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(io::Error::other)?
                .as_nanos();
            let record = format!("{timestamp}\tchild\t{}\t{phase}\n", process::id());
            file.write_all(record.as_bytes())?;
        }
        Ok(())
    }
}

pub(crate) fn main() {
    if run().is_err() {
        process::exit(2);
    }
}

fn run() -> io::Result<()> {
    let mut trace = PhaseTrace::from_environment()?;
    trace.record(&format!("process-spawned:{}", process::id()))?;
    let mut input = io::stdin().lock();
    let mut output = io::stdout().lock();
    let mut buffered = Vec::new();
    let mut chunk = [0_u8; 4_096];
    let mut initialized = false;
    let mut open_documents: BTreeMap<Box<str>, u64> = BTreeMap::new();
    let mut clean_documents: BTreeMap<Box<str>, bool> = BTreeMap::new();
    let mut startup_document_observed = false;
    let mut acknowledge_shutdown = true;
    let mut successful_exit = true;

    loop {
        let read = input.read(&mut chunk)?;
        if read == 0 {
            return Ok(());
        }
        buffered.extend_from_slice(&chunk[..read]);
        while let Some(body) = take_frame(&mut buffered)? {
            let message = std::str::from_utf8(&body)
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "non-UTF-8 JSON"))?;
            let method = json_string(message, "method");
            match method {
                Some("initialize") => {
                    trace.record("initialize-received")?;
                    let id = json_id(message)?;
                    write_frame(
                        &mut output,
                        &format!(
                            r#"{{"jsonrpc":"2.0","id":{id},"result":{{"capabilities":{{"textDocumentSync":{{"openClose":true,"change":2,"save":{{"includeText":false}}}},"diagnosticProvider":{{"interFileDependencies":true,"workspaceDiagnostics":false}}}}}}}}"#
                        ),
                    )?;
                    trace.record("initialize-responded")?;
                }
                Some("initialized") => {
                    trace.record("initialized-received")?;
                    initialized = true;
                }
                Some("textDocument/didOpen") if initialized => {
                    let uri = json_string(message, "uri").ok_or_else(|| {
                        io::Error::new(io::ErrorKind::InvalidData, "missing document URI")
                    })?;
                    if open_documents.contains_key(uri) || open_documents.len() == 32 {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "duplicate document open or workspace overlay limit",
                        ));
                    }
                    open_documents.insert(uri.into(), json_number(message, "version")?);
                    clean_documents.insert(uri.into(), false);
                    if !startup_document_observed {
                        trace.record("did-open-received")?;
                    }
                    write_diagnostics(&mut output, message, false)?;
                    if !startup_document_observed {
                        trace.record("diagnostics-written")?;
                        startup_document_observed = true;
                    }
                    write_frame(
                        &mut output,
                        r#"{"jsonrpc":"2.0","method":"test/unrelated-notification"}"#,
                    )?;
                }
                Some("textDocument/didChange") if initialized => {
                    let uri = json_string(message, "uri").ok_or_else(|| {
                        io::Error::new(io::ErrorKind::InvalidData, "missing document URI")
                    })?;
                    let version = json_number(message, "version")?;
                    let previous = open_documents.get_mut(uri).ok_or_else(|| {
                        io::Error::new(io::ErrorKind::InvalidData, "change for unopened document")
                    })?;
                    if version <= *previous {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "document change version did not advance",
                        ));
                    }
                    *previous = version;
                    clean_documents.insert(uri.into(), message.contains("let ok"));
                    if message.contains("ALPINE_CRASH") {
                        process::exit(7);
                    }
                    if message.contains("ALPINE_PROTOCOL_ERROR") {
                        write_frame(
                            &mut output,
                            r#"{"jsonrpc":"2.0","id":4294967294,"result":null}"#,
                        )?;
                        continue;
                    }
                    write_diagnostics(&mut output, message, message.contains("let ok"))?;
                }
                Some("textDocument/didSave") if initialized => {
                    let uri = json_string(message, "uri").ok_or_else(|| {
                        io::Error::new(io::ErrorKind::InvalidData, "missing saved document URI")
                    })?;
                    if !open_documents.contains_key(uri) {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "save for unopened document",
                        ));
                    }
                    // A URI-only disk save does not replace the current overlay
                    // or reset its version, even if newer unsaved edits exist.
                }
                Some("textDocument/didClose") if initialized => {
                    let uri = json_string(message, "uri").ok_or_else(|| {
                        io::Error::new(io::ErrorKind::InvalidData, "missing document URI")
                    })?;
                    if open_documents.remove(uri).is_none() {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "document close did not belong to an open document",
                        ));
                    }
                    clean_documents.remove(uri);
                }
                Some("textDocument/diagnostic") if initialized => {
                    let uri = json_string(message, "uri").ok_or_else(|| {
                        io::Error::new(io::ErrorKind::InvalidData, "missing diagnostic URI")
                    })?;
                    let clean = clean_documents.get(uri).ok_or_else(|| {
                        io::Error::new(
                            io::ErrorKind::InvalidData,
                            "diagnostic request for unopened document",
                        )
                    })?;
                    let id = json_id(message)?;
                    let items = if *clean {
                        "[]"
                    } else {
                        r#"[{"range":{"start":{"line":0,"character":0},"end":{"line":1,"character":0}},"severity":1,"message":"mock broken"}]"#
                    };
                    write_frame(
                        &mut output,
                        &format!(
                            r#"{{"jsonrpc":"2.0","id":{id},"result":{{"kind":"full","resultId":"constant-not-a-revision","items":{items}}}}}"#
                        ),
                    )?;
                }
                Some("textDocument/completion") if initialized => {
                    if message.contains(r#""character":99"#) {
                        continue;
                    }
                    let id = json_id(message)?;
                    write_frame(
                        &mut output,
                        &format!(
                            r#"{{"jsonrpc":"2.0","id":{id},"result":{{"isIncomplete":false,"items":[{{"label":"println!","documentation":"Print a line","textEdit":{{"range":{{"start":{{"line":0,"character":0}},"end":{{"line":0,"character":2}}}},"newText":"println!"}}}},{{"label":"print!","insertText":"print!"}}]}}}}"#
                        ),
                    )?;
                }
                Some("textDocument/hover") if initialized => {
                    if message.contains(r#""character":99"#) {
                        continue;
                    }
                    let id = json_id(message)?;
                    write_frame(
                        &mut output,
                        &format!(
                            r#"{{"jsonrpc":"2.0","id":{id},"result":{{"contents":{{"kind":"markdown","value":"`fn main()`\n\nMock hover"}}}}}}"#
                        ),
                    )?;
                }
                Some("textDocument/definition" | "textDocument/references") if initialized => {
                    if message.contains(r#""character":99"#) {
                        continue;
                    }
                    let id = json_id(message)?;
                    let uri = json_string(message, "uri").ok_or_else(|| {
                        io::Error::new(io::ErrorKind::InvalidData, "missing document URI")
                    })?;
                    let location = format!(
                        r#"{{"uri":"{uri}","range":{{"start":{{"line":0,"character":0}},"end":{{"line":0,"character":2}}}}}}"#
                    );
                    let result = if method == Some("textDocument/references") {
                        format!("[{location},{location}]")
                    } else {
                        location
                    };
                    write_frame(
                        &mut output,
                        &format!(r#"{{"jsonrpc":"2.0","id":{id},"result":{result}}}"#),
                    )?;
                }
                Some("textDocument/documentSymbol") if initialized => {
                    let id = json_id(message)?;
                    write_frame(
                        &mut output,
                        &format!(
                            r#"{{"jsonrpc":"2.0","id":{id},"result":[{{"name":"main","detail":"fn()","kind":12,"range":{{"start":{{"line":0,"character":0}},"end":{{"line":0,"character":12}}}},"selectionRange":{{"start":{{"line":0,"character":3}},"end":{{"line":0,"character":7}}}},"children":[{{"name":"inner","kind":13,"range":{{"start":{{"line":0,"character":3}},"end":{{"line":0,"character":8}}}},"selectionRange":{{"start":{{"line":0,"character":3}},"end":{{"line":0,"character":8}}}}}}]}}]}}"#
                        ),
                    )?;
                }
                Some("workspace/symbol") if initialized => {
                    if message.contains(r#""query":"""#) {
                        continue;
                    }
                    let id = json_id(message)?;
                    write_frame(
                        &mut output,
                        &format!(
                            r#"{{"jsonrpc":"2.0","id":{id},"result":[{{"name":"main","kind":12,"location":{{"uri":"file:///tmp/alpine/mock.rs","range":{{"start":{{"line":0,"character":0}},"end":{{"line":0,"character":4}}}}}},"containerName":"mock"}}]}}"#
                        ),
                    )?;
                }
                Some("textDocument/formatting") if initialized => {
                    let id = json_id(message)?;
                    write_frame(
                        &mut output,
                        &format!(
                            r#"{{"jsonrpc":"2.0","id":{id},"result":[{{"range":{{"start":{{"line":0,"character":0}},"end":{{"line":0,"character":2}}}},"newText":"pub fn"}}]}}"#
                        ),
                    )?;
                }
                Some("textDocument/rename") if initialized => {
                    if message.contains(r#""newName":"never_respond""#) {
                        continue;
                    }
                    let id = json_id(message)?;
                    let uri = json_string(message, "uri").ok_or_else(|| {
                        io::Error::new(io::ErrorKind::InvalidData, "missing document URI")
                    })?;
                    write_frame(
                        &mut output,
                        &format!(
                            r#"{{"jsonrpc":"2.0","id":{id},"result":{{"changes":{{"{uri}":[{{"range":{{"start":{{"line":0,"character":3}},"end":{{"line":0,"character":7}}}},"newText":"renamed"}}]}}}}}}"#
                        ),
                    )?;
                }
                Some("test/echo") if initialized => {
                    let id = json_id(message)?;
                    write_frame(
                        &mut output,
                        &format!(r#"{{"jsonrpc":"2.0","id":{id},"result":{{"ok":true}}}}"#),
                    )?;
                }
                Some("test/stderr") if initialized => {
                    let id = json_id(message)?;
                    writeln!(io::stderr().lock(), "mock diagnostic")?;
                    write_frame(
                        &mut output,
                        &format!(r#"{{"jsonrpc":"2.0","id":{id},"result":null}}"#),
                    )?;
                }
                Some("test/server-request") if initialized => {
                    let id = json_id(message)?;
                    write_frame(
                        &mut output,
                        &format!(r#"{{"jsonrpc":"2.0","id":{id},"result":null}}"#),
                    )?;
                    write_frame(
                        &mut output,
                        r#"{"jsonrpc":"2.0","id":0,"method":"workspace/diagnostic/refresh"}"#,
                    )?;
                }
                Some("test/notification") if initialized => {
                    write_frame(
                        &mut output,
                        r#"{"jsonrpc":"2.0","method":"test/unrelated-notification"}"#,
                    )?;
                }
                Some("test/slow") if initialized => {}
                Some("$/cancelRequest") if initialized => {
                    let id = json_id(message)?;
                    write_frame(
                        &mut output,
                        &format!(r#"{{"jsonrpc":"2.0","id":{id},"result":{{"late":true}}}}"#),
                    )?;
                }
                Some("test/crash") if initialized => process::exit(7),
                Some("test/block") if initialized => thread::sleep(Duration::from_secs(5)),
                Some("test/flood-stderr") if initialized => {
                    let mut stderr = io::stderr().lock();
                    let bytes = [b'x'; 16_384];
                    while stderr.write_all(&bytes).is_ok() {}
                    loop {
                        thread::sleep(Duration::from_secs(5));
                    }
                }
                Some("test/shutdown-without-ack") if initialized => acknowledge_shutdown = false,
                Some("test/shutdown-exit-error") if initialized => successful_exit = false,
                Some("shutdown") if initialized => {
                    if !acknowledge_shutdown {
                        return Ok(());
                    }
                    let id = json_id(message)?;
                    write_frame(
                        &mut output,
                        &format!(r#"{{"jsonrpc":"2.0","id":{id},"result":null}}"#),
                    )?;
                }
                Some("exit") if initialized => {
                    if !successful_exit {
                        process::exit(7);
                    }
                    return Ok(());
                }
                None if initialized && message == r#"{"jsonrpc":"2.0","id":0,"result":null}"# => {
                    write_frame(
                        &mut output,
                        r#"{"jsonrpc":"2.0","method":"test/server-request-acknowledged"}"#,
                    )?;
                }
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "invalid mock language-server lifecycle",
                    ));
                }
            }
        }
    }
}

fn take_frame(buffered: &mut Vec<u8>) -> io::Result<Option<Vec<u8>>> {
    let Some(header_end) = buffered.windows(4).position(|bytes| bytes == b"\r\n\r\n") else {
        return Ok(None);
    };
    let header = std::str::from_utf8(&buffered[..header_end])
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "non-UTF-8 header"))?;
    let length = header
        .strip_prefix("Content-Length: ")
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing content length"))?
        .parse::<usize>()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid content length"))?;
    let body_start = header_end + 4;
    let frame_end = body_start
        .checked_add(length)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "frame length overflow"))?;
    if buffered.len() < frame_end {
        return Ok(None);
    }
    let body = buffered[body_start..frame_end].to_vec();
    buffered.drain(..frame_end);
    Ok(Some(body))
}

fn json_string<'a>(message: &'a str, key: &str) -> Option<&'a str> {
    let prefix = format!(r#""{key}":""#);
    let value = message.split_once(&prefix)?.1;
    value.split_once('"').map(|(value, _)| value)
}

fn json_id(message: &str) -> io::Result<u64> {
    let value = message
        .split_once(r#""id":"#)
        .map(|(_, value)| value)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing request ID"))?;
    let digits = value
        .bytes()
        .take_while(u8::is_ascii_digit)
        .map(char::from)
        .collect::<String>();
    digits
        .parse()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid request ID"))
}

fn json_number(message: &str, key: &str) -> io::Result<u64> {
    let prefix = format!(r#""{key}":"#);
    let value = message
        .split_once(&prefix)
        .map(|(_, value)| value)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing JSON number"))?;
    let digits = value
        .bytes()
        .take_while(u8::is_ascii_digit)
        .map(char::from)
        .collect::<String>();
    digits
        .parse()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid JSON number"))
}

fn write_diagnostics(output: &mut impl Write, message: &str, empty: bool) -> io::Result<()> {
    let uri = json_string(message, "uri")
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing document URI"))?;
    let version = json_number(message, "version")?;
    let diagnostics = if empty {
        "[]"
    } else {
        r#"[{"range":{"start":{"line":0,"character":0},"end":{"line":1,"character":0}},"severity":1,"message":"mock broken"}]"#
    };
    write_frame(
        output,
        &format!(
            r#"{{"jsonrpc":"2.0","method":"textDocument/publishDiagnostics","params":{{"uri":"{uri}","version":{version},"diagnostics":{diagnostics}}}}}"#
        ),
    )
}

fn write_frame(output: &mut impl Write, body: &str) -> io::Result<()> {
    write!(output, "Content-Length: {}\r\n\r\n{body}", body.len())?;
    output.flush()
}
