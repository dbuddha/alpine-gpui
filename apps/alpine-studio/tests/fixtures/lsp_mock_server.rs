use std::{
    io::{self, Read, Write},
    process,
    thread,
    time::Duration,
};

fn main() {
    if run().is_err() {
        process::exit(2);
    }
}

fn run() -> io::Result<()> {
    let mut input = io::stdin().lock();
    let mut output = io::stdout().lock();
    let mut buffered = Vec::new();
    let mut chunk = [0_u8; 4_096];
    let mut initialized = false;

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
                    let id = json_id(message)?;
                    write_frame(
                        &mut output,
                        &format!(
                            r#"{{"jsonrpc":"2.0","id":{id},"result":{{"capabilities":{{}}}}}}"#
                        ),
                    )?;
                }
                Some("initialized") => initialized = true,
                Some("textDocument/didOpen") if initialized => {
                    write_diagnostics(&mut output, message, false)?;
                    write_frame(
                        &mut output,
                        r#"{"jsonrpc":"2.0","method":"test/unrelated-notification"}"#,
                    )?;
                }
                Some("textDocument/didChange") if initialized => {
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
                Some("shutdown") if initialized => {
                    let id = json_id(message)?;
                    write_frame(
                        &mut output,
                        &format!(r#"{{"jsonrpc":"2.0","id":{id},"result":null}}"#),
                    )?;
                }
                Some("exit") if initialized => return Ok(()),
                None
                    if initialized
                        && message
                            == r#"{"jsonrpc":"2.0","id":0,"result":null}"# =>
                {
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
