//! Differential mutation controls for streaming snapshot operations.

use std::ops::Range;

use alpine_text::{Buffer, BufferSnapshot, TextError};

fn independent_fingerprint(text: &str, range: Range<usize>) -> TextFingerprintParts {
    let mut first = 0xcbf2_9ce4_8422_2325_u64;
    let mut second = 0x6eed_0e9d_a4d9_4a4f_u64;
    for byte in text.as_bytes()[range.clone()].iter().copied() {
        first ^= u64::from(byte);
        first = first.wrapping_mul(0x0000_0100_0000_01b3);
        second ^= u64::from(byte).wrapping_add(0x9e37_79b9_7f4a_7c15);
        second = second.rotate_left(27).wrapping_mul(0x3c79_ac49_2ba7_b653);
    }
    TextFingerprintParts {
        bytes: range.len(),
        first,
        second,
    }
}

struct TextFingerprintParts {
    bytes: usize,
    first: u64,
    second: u64,
}

fn snapshot(text: &str) -> BufferSnapshot {
    Buffer::new(text).snapshot()
}

#[test]
fn streaming_identity_ranges_and_exact_comparison_are_discriminating() -> Result<(), TextError> {
    let text = "Alpine\nGPUI\nfast";
    let value = snapshot(text);
    assert_eq!(value.line_byte_range(0), Ok(0..7));
    assert_eq!(value.line_byte_range(1), Ok(7..12));
    assert_eq!(value.line_byte_range(2), Ok(12..16));
    assert!(value.line_byte_range(3).is_err());

    let range = 2..15;
    let expected = independent_fingerprint(text, range.clone());
    let actual = value.fingerprint(range.clone())?;
    assert_eq!(actual.bytes(), expected.bytes);
    assert_eq!(actual.first(), expected.first);
    assert_eq!(actual.second(), expected.second);
    assert_ne!(actual.first(), 1);
    assert_ne!(actual.second(), 1);

    let equal = snapshot("xxpine\nGPUI\nfaszz");
    assert_eq!(value.range_eq(range.clone(), &equal, 2..15), Ok(true));
    assert_eq!(value.range_eq(range.clone(), &equal, 2..14), Ok(false));
    assert_eq!(value.range_eq(2..14, &equal, 2..15), Ok(false));
    Ok(())
}
