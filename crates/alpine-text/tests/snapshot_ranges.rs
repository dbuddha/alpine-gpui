//! Public streaming snapshot-range contract tests.

use alpine_text::{Buffer, TextError};

#[test]
fn line_ranges_fingerprints_and_exact_comparison_are_streaming_and_checked() -> Result<(), TextError>
{
    let first = Buffer::new("alpha\r\nbeta\n").snapshot();
    let equal = Buffer::new("alpha\r\nbeta\n").snapshot();
    let changed = Buffer::new("alpha\r\nzeta\n").snapshot();

    assert_eq!(first.line_count(), 3);
    assert_eq!(first.line_byte_range(0)?, 0..7);
    assert_eq!(first.line_byte_range(1)?, 7..12);
    assert_eq!(first.line_byte_range(2)?, 12..12);
    assert!(matches!(
        first.line_byte_range(3),
        Err(TextError::LineOutOfBounds {
            line: 3,
            line_count: 3
        })
    ));

    let identity = first.fingerprint(0..7)?;
    assert_eq!(identity.bytes(), 7);
    assert_ne!(identity.first(), 0);
    assert_ne!(identity.second(), 0);
    assert_eq!(identity, equal.fingerprint(0..7)?);
    assert!(first.range_eq(0..12, &equal, 0..12)?);
    assert!(!first.range_eq(7..12, &changed, 7..12)?);
    assert!(!first.range_eq(0..7, &equal, 0..6)?);
    Ok(())
}
