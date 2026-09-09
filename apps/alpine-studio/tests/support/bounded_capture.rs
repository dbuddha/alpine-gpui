//! Bounded Unix stdio capture for native test children, not a shipping runtime.

use std::{
    error::Error,
    fmt,
    io::{self, Read as _},
    os::{fd::OwnedFd, unix::net::UnixStream},
    process::{Child, Command, ExitStatus, Output, Stdio},
    thread,
    time::{Duration, Instant},
};

pub(super) const OUTPUT_LIMIT: usize = 262_144;
const CHUNK_BYTES: usize = 8_192;
const POLL_INTERVAL: Duration = Duration::from_millis(10);
const CLEANUP_BUDGET: Duration = Duration::from_millis(250);
const CLEANUP_POLL_INTERVAL: Duration = Duration::from_millis(2);

#[derive(Debug)]
pub(super) struct CaptureFailure {
    cause: io::Error,
    pub(super) observed_exit: Option<ExitStatus>,
    pub(super) stdout_eof: bool,
    pub(super) stderr_eof: bool,
    pub(super) cleanup_error: Option<io::Error>,
    stdout_bytes: usize,
    stderr_bytes: usize,
    stdout_tail: String,
    stderr_tail: String,
}

impl fmt::Display for CaptureFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}; observed_exit={:?}; stdout_eof={}; stderr_eof={}; stdout_bytes={}; stderr_bytes={}; cleanup_error={:?}; stdout_tail={:?}; stderr_tail={:?}",
            self.cause,
            self.observed_exit,
            self.stdout_eof,
            self.stderr_eof,
            self.stdout_bytes,
            self.stderr_bytes,
            self.cleanup_error,
            self.stdout_tail,
            self.stderr_tail,
        )
    }
}

impl Error for CaptureFailure {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.cause)
    }
}

struct StreamCapture {
    stream: UnixStream,
    bytes: Vec<u8>,
    eof: bool,
}

impl StreamCapture {
    fn new(stream: UnixStream) -> io::Result<Self> {
        stream.set_nonblocking(true)?;
        Ok(Self {
            stream,
            bytes: Vec::new(),
            eof: false,
        })
    }

    fn read_chunk(&mut self) -> io::Result<bool> {
        if self.eof {
            return Ok(false);
        }
        let mut chunk = [0; CHUNK_BYTES];
        match self.stream.read(&mut chunk) {
            Ok(0) => {
                self.eof = true;
                Ok(true)
            }
            Ok(count) => {
                let required = self.bytes.len() + count;
                if required > OUTPUT_LIMIT {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "native child exceeded the per-stream output limit",
                    ));
                }
                if required > self.bytes.capacity() {
                    let capacity = self
                        .bytes
                        .capacity()
                        .saturating_mul(2)
                        .max(required)
                        .clamp(CHUNK_BYTES, OUTPUT_LIMIT);
                    self.bytes
                        .try_reserve_exact(capacity - self.bytes.len())
                        .map_err(|_| {
                            io::Error::new(
                                io::ErrorKind::OutOfMemory,
                                "native output capture allocation failed",
                            )
                        })?;
                }
                self.bytes.extend_from_slice(&chunk[..count]);
                Ok(true)
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) =>
            {
                Ok(false)
            }
            Err(error) => Err(error),
        }
    }

    fn tail(&self) -> String {
        let start = self.bytes.len().saturating_sub(4_096);
        String::from_utf8_lossy(&self.bytes[start..]).into_owned()
    }
}

/// Capture until the direct child exits AND both streams reach EOF.
/// Owned piped stdin closes immediately; this helper has no input feeder.
/// Error cleanup has a separate bounded grace and never grants late success.
pub(super) fn run(command: &mut Command, timeout: Duration) -> io::Result<Output> {
    if timeout.is_zero() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "native capture requires a nonzero deadline",
        ));
    }
    let deadline = Instant::now().checked_add(timeout).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "native capture deadline overflow",
        )
    })?;
    let (stdout_reader, stdout_writer) = UnixStream::pair()?;
    let (stderr_reader, stderr_writer) = UnixStream::pair()?;
    let mut stdout = StreamCapture::new(stdout_reader)?;
    let mut stderr = StreamCapture::new(stderr_reader)?;
    command
        .stdout(Stdio::from(OwnedFd::from(stdout_writer)))
        .stderr(Stdio::from(OwnedFd::from(stderr_writer)));
    let spawned = command.spawn();
    // Command retains supplied descriptors even after spawn. Release its
    // writer copies on success AND failure, or EOF cannot be observed.
    command.stdout(Stdio::null()).stderr(Stdio::null());
    let mut child = spawned?;
    drop(child.stdin.take());
    let mut observed_exit = None;
    let result = collect_until(
        &mut child,
        &mut observed_exit,
        &mut stdout,
        &mut stderr,
        deadline,
    );
    match result {
        Ok(output) => Ok(output),
        Err(cause) => {
            let kind = cause.kind();
            let cleanup_error = stop_owned_child(&mut child, observed_exit).err();
            Err(io::Error::new(
                kind,
                CaptureFailure {
                    cause,
                    observed_exit,
                    stdout_eof: stdout.eof,
                    stderr_eof: stderr.eof,
                    cleanup_error,
                    stdout_bytes: stdout.bytes.len(),
                    stderr_bytes: stderr.bytes.len(),
                    stdout_tail: stdout.tail(),
                    stderr_tail: stderr.tail(),
                },
            ))
        }
    }
}

fn collect_until(
    child: &mut Child,
    observed_exit: &mut Option<ExitStatus>,
    stdout: &mut StreamCapture,
    stderr: &mut StreamCapture,
    deadline: Instant,
) -> io::Result<Output> {
    loop {
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "native capture deadline elapsed before child exit and stream EOF",
            ));
        }
        // One bounded read per stream avoids starving stderr or the deadline.
        let stdout_progress = stdout.read_chunk()?;
        let stderr_progress = stderr.read_chunk()?;
        if observed_exit.is_none() {
            *observed_exit = child.try_wait()?;
        }
        if let Some(status) = *observed_exit
            && stdout.eof
            && stderr.eof
            && Instant::now() < deadline
        {
            return Ok(Output {
                status,
                stdout: std::mem::take(&mut stdout.bytes),
                stderr: std::mem::take(&mut stderr.bytes),
            });
        }
        if !stdout_progress && !stderr_progress {
            thread::sleep(POLL_INTERVAL.min(deadline.saturating_duration_since(Instant::now())));
        }
    }
}

fn stop_owned_child(child: &mut Child, observed_exit: Option<ExitStatus>) -> io::Result<()> {
    if observed_exit.is_some() || child.try_wait()?.is_some() {
        return Ok(());
    }
    // Only a fresh Ok(None) permits signaling this unreaped owned child.
    child.kill()?;
    let deadline = Instant::now() + CLEANUP_BUDGET;
    loop {
        if child.try_wait()?.is_some() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "owned native child did not reap within the cleanup budget",
            ));
        }
        thread::sleep(CLEANUP_POLL_INTERVAL);
    }
}
