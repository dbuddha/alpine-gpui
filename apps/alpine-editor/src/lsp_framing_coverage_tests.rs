use super::*;

fn limits(
    header_bytes: usize,
    message_bytes: usize,
    batch_frames: usize,
    batch_bytes: usize,
) -> Result<LspFrameLimits, LspFrameError> {
    LspFrameLimits::new(header_bytes, message_bytes, batch_frames, batch_bytes)
}

fn encoded(body: &[u8]) -> Vec<u8> {
    let mut bytes = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
    bytes.extend_from_slice(body);
    bytes
}

#[test]
fn defaults_observers_and_empty_admission_are_exact() -> Result<(), LspFrameError> {
    assert_eq!(DEFAULT_LSP_HEADER_BYTES, 8_192);
    assert_eq!(DEFAULT_LSP_MESSAGE_BYTES, 16_777_216);
    assert_eq!(DEFAULT_LSP_BATCH_FRAMES, 32);
    assert_eq!(DEFAULT_LSP_BATCH_BYTES, 16_777_216);
    assert_eq!(
        LspFrameLimits::default(),
        limits(8_192, 16_777_216, 32, 16_777_216)?
    );

    let mut framer = LspFramer::new(limits(64, 8, 2, 8)?);
    let empty = framer.ingest(b"")?;
    assert_eq!(empty.consumed(), 0);
    assert_eq!(empty.body_bytes(), 0);
    assert!(empty.frames().is_empty());

    let partial = framer.ingest(b"Content-Length: 2\r\n\r\nx")?;
    assert_eq!(partial.consumed(), 22);
    assert!(partial.frames().is_empty());
    let snapshot = framer.snapshot();
    assert_eq!(snapshot.phase(), LspFramePhase::Body);
    assert_eq!(snapshot.buffered_bytes(), 1);
    assert_eq!(snapshot.retained_bytes(), 2);
    assert_eq!(snapshot.peak_buffered_bytes(), 21);
    assert_eq!(snapshot.peak_retained_bytes(), 64);
    assert_eq!(snapshot.frames_emitted(), 0);
    assert_eq!(snapshot.body_bytes_emitted(), 0);
    assert!(!snapshot.poisoned());

    let completed = framer.ingest(b"y")?;
    assert_eq!(completed.frames()[0].sequence(), 1);
    assert_eq!(completed.frames()[0].body(), b"xy");
    let second = framer.ingest(&encoded(b"z"))?;
    assert_eq!(second.frames()[0].sequence(), 2);
    assert_eq!(second.frames()[0].body(), b"z");
    framer.finish()
}

#[test]
fn error_messages_preserve_every_variant_and_value() {
    let cases = [
        (LspFrameError::InvalidLimits, "LSP frame limits are invalid"),
        (
            LspFrameError::Poisoned,
            "LSP framer is poisoned after an earlier failure",
        ),
        (LspFrameError::NonAsciiHeader, "LSP header is not ASCII"),
        (
            LspFrameError::HeaderTooLarge { limit: 7 },
            "LSP header exceeds its 7-byte limit",
        ),
        (
            LspFrameError::MalformedHeader,
            "LSP header syntax is malformed",
        ),
        (
            LspFrameError::UnsupportedHeader,
            "LSP header field is unsupported",
        ),
        (
            LspFrameError::MissingContentLength,
            "LSP header has no Content-Length",
        ),
        (
            LspFrameError::DuplicateContentLength,
            "LSP header repeats Content-Length",
        ),
        (
            LspFrameError::InvalidContentLength,
            "LSP Content-Length is not a bounded decimal byte count",
        ),
        (LspFrameError::EmptyBody, "LSP message body is empty"),
        (
            LspFrameError::BodyTooLarge {
                declared: 9,
                limit: 8,
            },
            "LSP body declares 9 bytes, above its 8-byte limit",
        ),
        (
            LspFrameError::UnsupportedContentType,
            "LSP content type is not UTF-8 JSON-RPC",
        ),
        (
            LspFrameError::AllocationFailed(LspFramePhase::Header),
            "LSP Header buffer allocation failed",
        ),
        (
            LspFrameError::SequenceExhausted,
            "LSP frame sequence exhausted",
        ),
        (
            LspFrameError::CounterOverflow,
            "LSP frame accounting overflowed",
        ),
        (
            LspFrameError::InvalidState,
            "LSP framer reached an invalid state",
        ),
        (
            LspFrameError::UnexpectedEof {
                phase: LspFramePhase::Body,
                buffered: 3,
                expected: Some(5),
            },
            "LSP stream ended in Body after 3 bytes with expected length Some(5)",
        ),
    ];
    for (error, expected) in cases {
        assert_eq!(error.to_string(), expected);
        assert!(std::error::Error::source(&error).is_none());
    }
}

#[test]
fn batch_equality_and_invalid_emit_states_are_discriminating() -> Result<(), LspFrameError> {
    let mut stream = encoded(b"ab");
    stream.extend_from_slice(&encoded(b"cd"));
    let mut exact = LspFramer::new(limits(64, 2, 2, 4)?);
    let batch = exact.ingest(&stream)?;
    assert_eq!(batch.consumed(), stream.len());
    assert_eq!(batch.body_bytes(), 4);
    assert_eq!(batch.frames().len(), 2);
    assert_eq!(batch.frames()[1].sequence(), 2);

    let mut batch = LspFrameBatch {
        consumed: 0,
        body_bytes: 0,
        frames: Vec::new(),
    };
    let mut header = LspFramer::new(limits(64, 2, 2, 4)?);
    assert_eq!(
        header.emit_frame(&mut batch),
        Err(LspFrameError::InvalidState)
    );

    let mut incomplete = LspFramer::new(limits(64, 2, 2, 4)?);
    incomplete.state = ReadState::Body {
        expected: 2,
        bytes: vec![b'x'],
    };
    assert_eq!(
        incomplete.emit_frame(&mut batch),
        Err(LspFrameError::InvalidState)
    );

    let mut over = LspFramer::new(limits(64, 2, 2, 4)?);
    over.state = ReadState::Body {
        expected: 2,
        bytes: b"xy".to_vec(),
    };
    batch.body_bytes = 3;
    assert_eq!(
        over.emit_frame(&mut batch),
        Err(LspFrameError::InvalidState)
    );

    let mut equal = LspFramer::new(limits(64, 2, 2, 4)?);
    equal.state = ReadState::Body {
        expected: 2,
        bytes: b"xy".to_vec(),
    };
    batch.body_bytes = 2;
    equal.emit_frame(&mut batch)?;
    assert_eq!(batch.body_bytes(), 4);

    let mut full_header = LspFramer::new(limits(4, 2, 2, 4)?);
    full_header.state = ReadState::Header {
        bytes: b"full".to_vec(),
        delimiter_match: 0,
    };
    assert_eq!(
        full_header.ingest(b"x"),
        Err(LspFrameError::HeaderTooLarge { limit: 4 })
    );

    let mut complete_body = LspFramer::new(limits(64, 2, 2, 4)?);
    complete_body.state = ReadState::Body {
        expected: 2,
        bytes: b"xy".to_vec(),
    };
    assert_eq!(complete_body.ingest(b"z"), Err(LspFrameError::InvalidState));
    Ok(())
}

#[test]
fn reserve_growth_and_reuse_are_exact() -> Result<(), LspFrameError> {
    let mut bytes = Vec::new();
    reserve_bounded(&mut bytes, 1, 128, LspFramePhase::Header)?;
    assert_eq!(bytes.capacity(), 64);
    bytes.push(b'x');
    reserve_bounded(&mut bytes, 1, 128, LspFramePhase::Header)?;
    assert_eq!(bytes.capacity(), 64);
    reserve_bounded(&mut bytes, 64, 128, LspFramePhase::Header)?;
    assert_eq!(bytes.capacity(), 128);
    assert_eq!(
        reserve_bounded(&mut bytes, 128, 128, LspFramePhase::Header),
        Err(LspFrameError::InvalidState)
    );
    assert_eq!(
        validate_capacity(129, 128),
        Err(LspFrameError::InvalidState)
    );
    validate_capacity(128, 128)?;
    Ok(())
}

#[test]
fn malformed_line_axes_content_parameters_and_trimming_are_independent() {
    assert_eq!(parse_header(b"", 8), Err(LspFrameError::MalformedHeader));
    assert_eq!(
        parse_header(b"\r\n\r\n", 8),
        Err(LspFrameError::MissingContentLength)
    );
    for header in [
        b"\r\nContent-Length: 1\r\n\r\n".as_slice(),
        b"Content-Length: 1\rX\r\n\r\n".as_slice(),
        b"Content-Length: 1\nX\r\n\r\n".as_slice(),
        b"Content-Length: 1\r\n\r\n\r\n".as_slice(),
    ] {
        assert_eq!(parse_header(header, 8), Err(LspFrameError::MalformedHeader));
    }

    assert!(!supported_content_type(
        b"application/vscode-jsonrpc; charset=utf-8; charset=utf8"
    ));
    assert!(!supported_content_type(
        b"application/vscode-jsonrpc; boundary=x"
    ));
    assert!(!supported_content_type(
        b"application/vscode-jsonrpc; charset"
    ));
    assert_eq!(trim_ascii_space(b"\t  value \t"), b"value");
    assert_eq!(trim_ascii_space(b" \t "), b"");
}
