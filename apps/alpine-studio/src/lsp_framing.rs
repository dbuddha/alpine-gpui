//! Bounded byte framing for local Language Server Protocol streams.

use std::{fmt, mem};

const HEADER_TERMINATOR: &[u8; 4] = b"\r\n\r\n";
pub(crate) const DEFAULT_LSP_HEADER_BYTES: usize = 8 * 1_024;
pub(crate) const DEFAULT_LSP_MESSAGE_BYTES: usize = 16 * 1_024 * 1_024;
pub(crate) const DEFAULT_LSP_BATCH_FRAMES: usize = 32;
pub(crate) const DEFAULT_LSP_BATCH_BYTES: usize = 16 * 1_024 * 1_024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct LspFrameLimits {
    header_bytes: usize,
    message_bytes: usize,
    batch_frames: usize,
    batch_bytes: usize,
}

impl LspFrameLimits {
    pub(crate) const fn new(
        header_bytes: usize,
        message_bytes: usize,
        batch_frames: usize,
        batch_bytes: usize,
    ) -> Result<Self, LspFrameError> {
        if header_bytes < HEADER_TERMINATOR.len()
            || message_bytes == 0
            || batch_frames == 0
            || batch_bytes < message_bytes
        {
            return Err(LspFrameError::InvalidLimits);
        }
        Ok(Self {
            header_bytes,
            message_bytes,
            batch_frames,
            batch_bytes,
        })
    }
}

impl Default for LspFrameLimits {
    fn default() -> Self {
        Self {
            header_bytes: DEFAULT_LSP_HEADER_BYTES,
            message_bytes: DEFAULT_LSP_MESSAGE_BYTES,
            batch_frames: DEFAULT_LSP_BATCH_FRAMES,
            batch_bytes: DEFAULT_LSP_BATCH_BYTES,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LspFramePhase {
    Header,
    Body,
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct LspFrame {
    sequence: u64,
    body: Box<[u8]>,
}

impl LspFrame {
    pub(crate) const fn sequence(&self) -> u64 {
        self.sequence
    }

    pub(crate) fn body(&self) -> &[u8] {
        &self.body
    }
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct LspFrameBatch {
    consumed: usize,
    body_bytes: usize,
    frames: Vec<LspFrame>,
}

impl LspFrameBatch {
    pub(crate) const fn consumed(&self) -> usize {
        self.consumed
    }

    pub(crate) const fn body_bytes(&self) -> usize {
        self.body_bytes
    }

    pub(crate) fn frames(&self) -> &[LspFrame] {
        &self.frames
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct LspFramerSnapshot {
    phase: LspFramePhase,
    buffered_bytes: usize,
    retained_bytes: usize,
    peak_buffered_bytes: usize,
    peak_retained_bytes: usize,
    frames_emitted: u64,
    body_bytes_emitted: u64,
    poisoned: bool,
}

impl LspFramerSnapshot {
    pub(crate) const fn phase(self) -> LspFramePhase {
        self.phase
    }

    pub(crate) const fn buffered_bytes(self) -> usize {
        self.buffered_bytes
    }

    pub(crate) const fn retained_bytes(self) -> usize {
        self.retained_bytes
    }

    pub(crate) const fn peak_buffered_bytes(self) -> usize {
        self.peak_buffered_bytes
    }

    pub(crate) const fn peak_retained_bytes(self) -> usize {
        self.peak_retained_bytes
    }

    pub(crate) const fn frames_emitted(self) -> u64 {
        self.frames_emitted
    }

    pub(crate) const fn body_bytes_emitted(self) -> u64 {
        self.body_bytes_emitted
    }

    pub(crate) const fn poisoned(self) -> bool {
        self.poisoned
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LspFrameError {
    InvalidLimits,
    Poisoned,
    NonAsciiHeader,
    HeaderTooLarge {
        limit: usize,
    },
    MalformedHeader,
    UnsupportedHeader,
    MissingContentLength,
    DuplicateContentLength,
    InvalidContentLength,
    EmptyBody,
    BodyTooLarge {
        declared: usize,
        limit: usize,
    },
    UnsupportedContentType,
    AllocationFailed(LspFramePhase),
    SequenceExhausted,
    CounterOverflow,
    InvalidState,
    UnexpectedEof {
        phase: LspFramePhase,
        buffered: usize,
        expected: Option<usize>,
    },
}

impl fmt::Display for LspFrameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLimits => formatter.write_str("LSP frame limits are invalid"),
            Self::Poisoned => {
                formatter.write_str("LSP framer is poisoned after an earlier failure")
            }
            Self::NonAsciiHeader => formatter.write_str("LSP header is not ASCII"),
            Self::HeaderTooLarge { limit } => {
                write!(formatter, "LSP header exceeds its {limit}-byte limit")
            }
            Self::MalformedHeader => formatter.write_str("LSP header syntax is malformed"),
            Self::UnsupportedHeader => formatter.write_str("LSP header field is unsupported"),
            Self::MissingContentLength => formatter.write_str("LSP header has no Content-Length"),
            Self::DuplicateContentLength => {
                formatter.write_str("LSP header repeats Content-Length")
            }
            Self::InvalidContentLength => {
                formatter.write_str("LSP Content-Length is not a bounded decimal byte count")
            }
            Self::EmptyBody => formatter.write_str("LSP message body is empty"),
            Self::BodyTooLarge { declared, limit } => write!(
                formatter,
                "LSP body declares {declared} bytes, above its {limit}-byte limit"
            ),
            Self::UnsupportedContentType => {
                formatter.write_str("LSP content type is not UTF-8 JSON-RPC")
            }
            Self::AllocationFailed(phase) => {
                write!(formatter, "LSP {phase:?} buffer allocation failed")
            }
            Self::SequenceExhausted => formatter.write_str("LSP frame sequence exhausted"),
            Self::CounterOverflow => formatter.write_str("LSP frame accounting overflowed"),
            Self::InvalidState => formatter.write_str("LSP framer reached an invalid state"),
            Self::UnexpectedEof {
                phase,
                buffered,
                expected,
            } => write!(
                formatter,
                "LSP stream ended in {phase:?} after {buffered} bytes with expected length {expected:?}"
            ),
        }
    }
}

impl std::error::Error for LspFrameError {}

enum ReadState {
    Header {
        bytes: Vec<u8>,
        delimiter_match: usize,
    },
    Body {
        expected: usize,
        bytes: Vec<u8>,
    },
}

impl ReadState {
    fn phase(&self) -> LspFramePhase {
        match self {
            Self::Header { .. } => LspFramePhase::Header,
            Self::Body { .. } => LspFramePhase::Body,
        }
    }

    fn buffered_bytes(&self) -> usize {
        match self {
            Self::Header { bytes, .. } | Self::Body { bytes, .. } => bytes.len(),
        }
    }

    fn retained_bytes(&self) -> usize {
        match self {
            Self::Header { bytes, .. } | Self::Body { bytes, .. } => bytes.capacity(),
        }
    }
}

pub(crate) struct LspFramer {
    limits: LspFrameLimits,
    state: ReadState,
    peak_buffered_bytes: usize,
    peak_retained_bytes: usize,
    frames_emitted: u64,
    body_bytes_emitted: u64,
    poisoned: bool,
}

impl LspFramer {
    pub(crate) const fn new(limits: LspFrameLimits) -> Self {
        Self {
            limits,
            state: ReadState::Header {
                bytes: Vec::new(),
                delimiter_match: 0,
            },
            peak_buffered_bytes: 0,
            peak_retained_bytes: 0,
            frames_emitted: 0,
            body_bytes_emitted: 0,
            poisoned: false,
        }
    }

    pub(crate) fn ingest(&mut self, input: &[u8]) -> Result<LspFrameBatch, LspFrameError> {
        if self.poisoned {
            return Err(LspFrameError::Poisoned);
        }
        let result = self.ingest_inner(input);
        if result.is_err() {
            self.poisoned = true;
        }
        result
    }

    fn ingest_inner(&mut self, input: &[u8]) -> Result<LspFrameBatch, LspFrameError> {
        let mut batch = LspFrameBatch {
            consumed: 0,
            body_bytes: 0,
            frames: Vec::new(),
        };

        while batch.consumed < input.len() {
            if batch.frames.len() == self.limits.batch_frames {
                break;
            }
            if let ReadState::Body { expected, .. } = &self.state {
                let projected = batch
                    .body_bytes
                    .checked_add(*expected)
                    .ok_or(LspFrameError::CounterOverflow)?;
                if !batch.frames.is_empty() && projected > self.limits.batch_bytes {
                    break;
                }
            }

            let mut body_expected = None;
            let mut body_complete = false;
            match &mut self.state {
                ReadState::Header {
                    bytes,
                    delimiter_match,
                } => {
                    let byte = input[batch.consumed];
                    if !byte.is_ascii() {
                        return Err(LspFrameError::NonAsciiHeader);
                    }
                    if bytes.len() == self.limits.header_bytes {
                        return Err(LspFrameError::HeaderTooLarge {
                            limit: self.limits.header_bytes,
                        });
                    }
                    reserve_bounded(bytes, 1, self.limits.header_bytes, LspFramePhase::Header)?;
                    bytes.push(byte);
                    batch.consumed += 1;
                    *delimiter_match = advance_delimiter(*delimiter_match, byte);
                    if *delimiter_match == HEADER_TERMINATOR.len() {
                        body_expected = Some(parse_header(bytes, self.limits.message_bytes)?);
                    } else if bytes.len() == self.limits.header_bytes {
                        return Err(LspFrameError::HeaderTooLarge {
                            limit: self.limits.header_bytes,
                        });
                    }
                }
                ReadState::Body { expected, bytes } => {
                    let remaining = expected
                        .checked_sub(bytes.len())
                        .ok_or(LspFrameError::InvalidState)?;
                    let available = input.len() - batch.consumed;
                    let take = remaining.min(available);
                    reserve_bounded(bytes, take, *expected, LspFramePhase::Body)?;
                    bytes.extend_from_slice(&input[batch.consumed..batch.consumed + take]);
                    batch.consumed += take;
                    body_complete = bytes.len() == *expected;
                }
            }
            self.observe_buffer();

            if let Some(expected) = body_expected {
                self.state = ReadState::Body {
                    expected,
                    bytes: Vec::new(),
                };
                continue;
            }
            if body_complete {
                self.emit_frame(&mut batch)?;
            }
        }
        Ok(batch)
    }

    fn emit_frame(&mut self, batch: &mut LspFrameBatch) -> Result<(), LspFrameError> {
        let expected = match &self.state {
            ReadState::Body { expected, bytes } if bytes.len() == *expected => *expected,
            _ => return Err(LspFrameError::InvalidState),
        };
        let sequence = self
            .frames_emitted
            .checked_add(1)
            .ok_or(LspFrameError::SequenceExhausted)?;
        let expected_u64 = u64::try_from(expected).map_err(|_| LspFrameError::CounterOverflow)?;
        let emitted_bytes = self
            .body_bytes_emitted
            .checked_add(expected_u64)
            .ok_or(LspFrameError::CounterOverflow)?;
        let batch_bytes = batch
            .body_bytes
            .checked_add(expected)
            .ok_or(LspFrameError::CounterOverflow)?;
        if batch_bytes > self.limits.batch_bytes {
            return Err(LspFrameError::InvalidState);
        }
        batch
            .frames
            .try_reserve(1)
            .map_err(|_| LspFrameError::AllocationFailed(LspFramePhase::Body))?;

        let state = mem::replace(
            &mut self.state,
            ReadState::Header {
                bytes: Vec::new(),
                delimiter_match: 0,
            },
        );
        let ReadState::Body { bytes, .. } = state else {
            return Err(LspFrameError::InvalidState);
        };
        batch.frames.push(LspFrame {
            sequence,
            body: bytes.into_boxed_slice(),
        });
        batch.body_bytes = batch_bytes;
        self.frames_emitted = sequence;
        self.body_bytes_emitted = emitted_bytes;
        Ok(())
    }

    pub(crate) fn finish(&mut self) -> Result<(), LspFrameError> {
        if self.poisoned {
            return Err(LspFrameError::Poisoned);
        }
        let error = match &self.state {
            ReadState::Header { bytes, .. } if bytes.is_empty() => return Ok(()),
            ReadState::Header { bytes, .. } => LspFrameError::UnexpectedEof {
                phase: LspFramePhase::Header,
                buffered: bytes.len(),
                expected: None,
            },
            ReadState::Body { expected, bytes } => LspFrameError::UnexpectedEof {
                phase: LspFramePhase::Body,
                buffered: bytes.len(),
                expected: Some(*expected),
            },
        };
        self.poisoned = true;
        Err(error)
    }

    pub(crate) fn snapshot(&self) -> LspFramerSnapshot {
        LspFramerSnapshot {
            phase: self.state.phase(),
            buffered_bytes: self.state.buffered_bytes(),
            retained_bytes: self.state.retained_bytes(),
            peak_buffered_bytes: self.peak_buffered_bytes,
            peak_retained_bytes: self.peak_retained_bytes,
            frames_emitted: self.frames_emitted,
            body_bytes_emitted: self.body_bytes_emitted,
            poisoned: self.poisoned,
        }
    }

    fn observe_buffer(&mut self) {
        self.peak_buffered_bytes = self.peak_buffered_bytes.max(self.state.buffered_bytes());
        self.peak_retained_bytes = self.peak_retained_bytes.max(self.state.retained_bytes());
    }
}

fn reserve_bounded(
    bytes: &mut Vec<u8>,
    additional: usize,
    limit: usize,
    phase: LspFramePhase,
) -> Result<(), LspFrameError> {
    let required = bytes
        .len()
        .checked_add(additional)
        .ok_or(LspFrameError::CounterOverflow)?;
    if required > limit {
        return Err(LspFrameError::InvalidState);
    }
    if required <= bytes.capacity() {
        return Ok(());
    }
    let grown = if bytes.capacity() == 0 {
        64
    } else {
        bytes.capacity().saturating_mul(2)
    };
    let target = grown.min(limit).max(required);
    bytes
        .try_reserve_exact(target - bytes.capacity())
        .map_err(|_| LspFrameError::AllocationFailed(phase))?;
    if bytes.capacity() > limit {
        return Err(LspFrameError::InvalidState);
    }
    Ok(())
}

fn advance_delimiter(current: usize, byte: u8) -> usize {
    if byte == HEADER_TERMINATOR[current] {
        current + 1
    } else {
        usize::from(byte == HEADER_TERMINATOR[0])
    }
}

fn parse_header(bytes: &[u8], message_limit: usize) -> Result<usize, LspFrameError> {
    let payload_bytes = bytes
        .len()
        .checked_sub(HEADER_TERMINATOR.len())
        .ok_or(LspFrameError::MalformedHeader)?;
    let payload = &bytes[..payload_bytes];
    if payload.is_empty() {
        return Err(LspFrameError::MissingContentLength);
    }

    let mut content_length = None;
    let mut content_type = false;
    let mut start = 0;
    loop {
        let separator = payload[start..]
            .windows(2)
            .position(|window| window == b"\r\n")
            .map(|relative| start + relative);
        let end = separator.unwrap_or(payload.len());
        let line = &payload[start..end];
        if line.is_empty() || line.contains(&b'\r') || line.contains(&b'\n') {
            return Err(LspFrameError::MalformedHeader);
        }
        parse_header_line(line, &mut content_length, &mut content_type, message_limit)?;
        let Some(end) = separator else {
            break;
        };
        start = end + 2;
        if start == payload.len() {
            return Err(LspFrameError::MalformedHeader);
        }
    }
    content_length.ok_or(LspFrameError::MissingContentLength)
}

fn parse_header_line(
    line: &[u8],
    content_length: &mut Option<usize>,
    content_type: &mut bool,
    message_limit: usize,
) -> Result<(), LspFrameError> {
    let separator = line
        .iter()
        .position(|byte| *byte == b':')
        .ok_or(LspFrameError::MalformedHeader)?;
    let name = &line[..separator];
    let value = trim_ascii_space(&line[separator + 1..]);
    if name.is_empty() || value.is_empty() {
        return Err(LspFrameError::MalformedHeader);
    }
    if name.eq_ignore_ascii_case(b"Content-Length") {
        if content_length.is_some() {
            return Err(LspFrameError::DuplicateContentLength);
        }
        let declared = parse_decimal(value)?;
        if declared == 0 {
            return Err(LspFrameError::EmptyBody);
        }
        if declared > message_limit {
            return Err(LspFrameError::BodyTooLarge {
                declared,
                limit: message_limit,
            });
        }
        *content_length = Some(declared);
        return Ok(());
    }
    if name.eq_ignore_ascii_case(b"Content-Type") {
        if *content_type || !supported_content_type(value) {
            return Err(LspFrameError::UnsupportedContentType);
        }
        *content_type = true;
        return Ok(());
    }
    Err(LspFrameError::UnsupportedHeader)
}

fn parse_decimal(value: &[u8]) -> Result<usize, LspFrameError> {
    let mut parsed = 0usize;
    for byte in value {
        if !byte.is_ascii_digit() {
            return Err(LspFrameError::InvalidContentLength);
        }
        parsed = parsed
            .checked_mul(10)
            .and_then(|current| current.checked_add(usize::from(*byte - b'0')))
            .ok_or(LspFrameError::InvalidContentLength)?;
    }
    Ok(parsed)
}

fn supported_content_type(value: &[u8]) -> bool {
    let mut parts = value.split(|byte| *byte == b';');
    if !parts.next().is_some_and(|mime| {
        trim_ascii_space(mime).eq_ignore_ascii_case(b"application/vscode-jsonrpc")
    }) {
        return false;
    }
    let mut charset = false;
    for parameter in parts {
        let parameter = trim_ascii_space(parameter);
        let Some(separator) = parameter.iter().position(|byte| *byte == b'=') else {
            return false;
        };
        let name = trim_ascii_space(&parameter[..separator]);
        let value = trim_ascii_space(&parameter[separator + 1..]);
        if charset || !name.eq_ignore_ascii_case(b"charset") {
            return false;
        }
        if !value.eq_ignore_ascii_case(b"utf-8") && !value.eq_ignore_ascii_case(b"utf8") {
            return false;
        }
        charset = true;
    }
    true
}

fn trim_ascii_space(mut value: &[u8]) -> &[u8] {
    while value
        .first()
        .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
    {
        value = &value[1..];
    }
    while value
        .last()
        .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
    {
        value = &value[..value.len() - 1];
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits(
        header: usize,
        message: usize,
        frames: usize,
        batch: usize,
    ) -> Result<LspFrameLimits, LspFrameError> {
        LspFrameLimits::new(header, message, frames, batch)
    }

    fn frame(body: &[u8]) -> Vec<u8> {
        let mut frame = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
        frame.extend_from_slice(body);
        frame
    }

    #[test]
    fn fragmented_and_pipelined_frames_preserve_exact_bytes_and_accounting()
    -> Result<(), LspFrameError> {
        let first_body = br#"{"jsonrpc":"2.0","id":1,"result":null}"#;
        let second_body = "{\"jsonrpc\":\"2.0\",\"method\":\"window/logMessage\",\"params\":{\"message\":\"雪\"}}".as_bytes();
        let mut stream = format!(
            "content-length: {}\r\nCONTENT-TYPE: application/vscode-jsonrpc; charset=utf8\r\n\r\n",
            first_body.len()
        )
        .into_bytes();
        stream.extend_from_slice(first_body);
        stream.extend_from_slice(&frame(second_body));

        let mut framer = LspFramer::new(LspFrameLimits::default());
        let mut bodies = Vec::new();
        for byte in &stream {
            let batch = framer.ingest(std::slice::from_ref(byte))?;
            assert_eq!(batch.consumed(), 1);
            bodies.extend(batch.frames().iter().map(|frame| frame.body().to_vec()));
        }
        assert_eq!(bodies, [first_body.to_vec(), second_body.to_vec()]);
        let snapshot = framer.snapshot();
        assert_eq!(snapshot.phase(), LspFramePhase::Header);
        assert_eq!(snapshot.buffered_bytes(), 0);
        assert_eq!(snapshot.retained_bytes(), 0);
        assert_eq!(snapshot.frames_emitted(), 2);
        assert_eq!(
            snapshot.body_bytes_emitted(),
            (first_body.len() + second_body.len()) as u64
        );
        assert!(snapshot.peak_buffered_bytes() <= DEFAULT_LSP_HEADER_BYTES);
        assert!(snapshot.peak_retained_bytes() <= DEFAULT_LSP_MESSAGE_BYTES);
        assert!(!snapshot.poisoned());
        framer.finish()
    }

    #[test]
    fn every_two_chunk_split_reconstructs_one_exact_frame() -> Result<(), LspFrameError> {
        let body = br#"{"jsonrpc":"2.0","id":"bounded","result":[1,2,3]}"#;
        let encoded = frame(body);
        for split in 0..=encoded.len() {
            let mut framer = LspFramer::new(LspFrameLimits::default());
            let first = framer.ingest(&encoded[..split])?;
            assert_eq!(first.consumed(), split);
            let second = framer.ingest(&encoded[split..])?;
            let decoded = first
                .frames()
                .iter()
                .chain(second.frames())
                .collect::<Vec<_>>();
            assert_eq!(decoded.len(), 1);
            assert_eq!(decoded[0].sequence(), 1);
            assert_eq!(decoded[0].body(), body);
            framer.finish()?;
        }
        Ok(())
    }

    #[test]
    fn frame_and_byte_batch_limits_apply_backpressure_without_losing_state()
    -> Result<(), LspFrameError> {
        let first = frame(b"one");
        let second = frame(b"two");
        let mut stream = first.clone();
        stream.extend_from_slice(&second);

        let mut by_frame = LspFramer::new(limits(128, 8, 1, 8)?);
        let first_batch = by_frame.ingest(&stream)?;
        assert_eq!(first_batch.consumed(), first.len());
        assert_eq!(first_batch.frames()[0].body(), b"one");
        let second_batch = by_frame.ingest(&stream[first_batch.consumed()..])?;
        assert_eq!(second_batch.frames()[0].body(), b"two");

        let mut by_bytes = LspFramer::new(limits(128, 4, 4, 4)?);
        let first_batch = by_bytes.ingest(&stream)?;
        assert_eq!(first_batch.body_bytes(), 3);
        assert_eq!(first_batch.frames().len(), 1);
        assert!(first_batch.consumed() > first.len());
        assert!(first_batch.consumed() < stream.len());
        assert_eq!(by_bytes.snapshot().phase(), LspFramePhase::Body);
        let second_batch = by_bytes.ingest(&stream[first_batch.consumed()..])?;
        assert_eq!(second_batch.frames()[0].body(), b"two");
        by_bytes.finish()
    }

    #[test]
    fn malformed_headers_fail_closed_and_poison_the_stream() {
        let cases: &[(&[u8], LspFrameError)] = &[
            (b"Content-Type: application/vscode-jsonrpc\r\n\r\n", LspFrameError::MissingContentLength),
            (b"Content-Length: 1\r\nContent-Length: 1\r\n\r\nx", LspFrameError::DuplicateContentLength),
            (b"Content-Length: -1\r\n\r\n", LspFrameError::InvalidContentLength),
            (b"Content-Length: 0\r\n\r\n", LspFrameError::EmptyBody),
            (b"X-Alpine: 1\r\nContent-Length: 1\r\n\r\nx", LspFrameError::UnsupportedHeader),
            (b"Content-Length: 1\r\nContent-Type: application/json; charset=utf-8\r\n\r\nx", LspFrameError::UnsupportedContentType),
            (b"Content-Length: 1\r\nContent-Type: application/vscode-jsonrpc; charset=latin1\r\n\r\nx", LspFrameError::UnsupportedContentType),
            (b"Content-Length 1\r\n\r\nx", LspFrameError::MalformedHeader),
            (b"Content-Length:\r\n\r\n", LspFrameError::MalformedHeader),
        ];
        for (input, expected) in cases {
            let mut framer = LspFramer::new(LspFrameLimits::default());
            assert_eq!(framer.ingest(input), Err(*expected));
            assert!(framer.snapshot().poisoned());
            assert_eq!(framer.ingest(b"ignored"), Err(LspFrameError::Poisoned));
            assert_eq!(framer.finish(), Err(LspFrameError::Poisoned));
        }

        let mut non_ascii = b"Content-Length: 1\r\nX: ".to_vec();
        non_ascii.push(0xff);
        let mut framer = LspFramer::new(LspFrameLimits::default());
        assert_eq!(
            framer.ingest(&non_ascii),
            Err(LspFrameError::NonAsciiHeader)
        );
    }

    #[test]
    fn declared_and_retained_memory_are_independently_bounded() -> Result<(), LspFrameError> {
        let mut oversized = LspFramer::new(limits(128, 4, 1, 4)?);
        assert_eq!(
            oversized.ingest(b"Content-Length: 5\r\n\r\n"),
            Err(LspFrameError::BodyTooLarge {
                declared: 5,
                limit: 4
            })
        );

        let mut header = LspFramer::new(limits(8, 16, 1, 16)?);
        assert_eq!(
            header.ingest(b"Content-"),
            Err(LspFrameError::HeaderTooLarge { limit: 8 })
        );

        let declared = 1_048_576;
        let mut partial = format!("Content-Length: {declared}\r\n\r\n").into_bytes();
        partial.push(b'x');
        let mut framer = LspFramer::new(limits(128, declared, 1, declared)?);
        let batch = framer.ingest(&partial)?;
        assert!(batch.frames().is_empty());
        let snapshot = framer.snapshot();
        assert_eq!(snapshot.buffered_bytes(), 1);
        assert!(snapshot.retained_bytes() <= 64);
        assert!(snapshot.retained_bytes() < declared);
        Ok(())
    }

    #[test]
    fn clean_and_truncated_eof_have_deterministic_terminal_results() -> Result<(), LspFrameError> {
        let mut clean = LspFramer::new(LspFrameLimits::default());
        clean.finish()?;

        let mut header = LspFramer::new(LspFrameLimits::default());
        assert!(header.ingest(b"Content-Len")?.frames().is_empty());
        assert_eq!(
            header.finish(),
            Err(LspFrameError::UnexpectedEof {
                phase: LspFramePhase::Header,
                buffered: 11,
                expected: None
            })
        );

        let mut body = LspFramer::new(LspFrameLimits::default());
        assert!(
            body.ingest(b"Content-Length: 4\r\n\r\nab")?
                .frames()
                .is_empty()
        );
        assert_eq!(
            body.finish(),
            Err(LspFrameError::UnexpectedEof {
                phase: LspFramePhase::Body,
                buffered: 2,
                expected: Some(4)
            })
        );
        Ok(())
    }

    #[test]
    fn invalid_limits_content_length_overflow_and_counters_are_structured() {
        assert_eq!(limits(3, 1, 1, 1), Err(LspFrameError::InvalidLimits));
        assert_eq!(limits(4, 0, 1, 1), Err(LspFrameError::InvalidLimits));
        assert_eq!(limits(4, 2, 0, 2), Err(LspFrameError::InvalidLimits));
        assert_eq!(limits(4, 2, 1, 1), Err(LspFrameError::InvalidLimits));

        let mut overflow = LspFramer::new(LspFrameLimits::default());
        assert_eq!(
            overflow.ingest(b"Content-Length: 999999999999999999999999999999999\r\n\r\n"),
            Err(LspFrameError::InvalidContentLength)
        );

        let encoded = frame(b"x");
        let mut sequence = LspFramer::new(LspFrameLimits::default());
        sequence.frames_emitted = u64::MAX;
        assert_eq!(
            sequence.ingest(&encoded),
            Err(LspFrameError::SequenceExhausted)
        );

        let mut bytes = LspFramer::new(LspFrameLimits::default());
        bytes.body_bytes_emitted = u64::MAX;
        assert_eq!(bytes.ingest(&encoded), Err(LspFrameError::CounterOverflow));
    }

    #[test]
    fn content_type_accepts_spec_and_backward_compatible_utf8_spellings()
    -> Result<(), LspFrameError> {
        for value in [
            "application/vscode-jsonrpc",
            "application/vscode-jsonrpc; charset=utf-8",
            "APPLICATION/VSCODE-JSONRPC ; CHARSET = UTF8",
        ] {
            let input = format!("Content-Length: 1\r\nContent-Type: {value}\r\n\r\nx");
            let mut framer = LspFramer::new(LspFrameLimits::default());
            let batch = framer.ingest(input.as_bytes())?;
            assert_eq!(batch.frames()[0].body(), b"x");
            framer.finish()?;
        }
        Ok(())
    }
}
