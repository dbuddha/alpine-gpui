//! Differential library-test controls for streaming snapshot operations.

use super::*;

fn independent_fingerprint(text: &str, range: Range<usize>) -> (usize, u64, u64) {
    let mut first = 0xcbf2_9ce4_8422_2325_u64;
    let mut second = 0x6eed_0e9d_a4d9_4a4f_u64;
    for byte in text.as_bytes()[range.clone()].iter().copied() {
        first ^= u64::from(byte);
        first = first.wrapping_mul(0x0000_0100_0000_01b3);
        second ^= u64::from(byte).wrapping_add(0x9e37_79b9_7f4a_7c15);
        second = second.rotate_left(27).wrapping_mul(0x3c79_ac49_2ba7_b653);
    }
    (range.len(), first, second)
}

#[test]
fn ranges_fingerprints_and_equality_are_independently_discriminated() -> Result<(), TextError> {
    let text = "Alpine\nGPUI\nfast";
    let value = Buffer::new(text).snapshot();
    assert_eq!(value.line_byte_range(0), Ok(0..7));
    assert_eq!(value.line_byte_range(1), Ok(7..12));
    assert_eq!(value.line_byte_range(2), Ok(12..16));
    assert!(value.line_byte_range(3).is_err());

    let range = 2..15;
    let (bytes, first, second) = independent_fingerprint(text, range.clone());
    let actual = value.fingerprint(range.clone())?;
    assert_eq!(
        (actual.bytes(), actual.first(), actual.second()),
        (bytes, first, second)
    );

    let equal = Buffer::new("xxpine\nGPUI\nfaszz").snapshot();
    assert_eq!(value.range_eq(range.clone(), &equal, 2..15), Ok(true));
    assert_eq!(value.range_eq(range.clone(), &equal, 2..14), Ok(false));
    assert_eq!(value.range_eq(2..14, &equal, 2..15), Ok(false));
    Ok(())
}
