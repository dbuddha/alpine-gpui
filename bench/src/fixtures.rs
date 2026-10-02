//! Deterministic plain-text fixtures. Each line is a seven-digit line number,
//! a space, pseudo-random words cut to 63 bytes, and LF: identical bytes for
//! every app, and the line number shows the scroll position in a capture.

use crate::workload::Fixture;
use std::fs::{self, File};
use std::io::{BufWriter, Read as _, Write as _};
use std::path::{Path, PathBuf};

pub const LINE_BYTES: usize = 64;

const WORDS: [&str; 32] = [
    "alpine", "editor", "frame", "glyph", "buffer", "cursor", "scroll", "metal", "layer", "range",
    "token", "parse", "index", "query", "split", "merge", "cache", "bound", "queue", "event",
    "paint", "shape", "trace", "patch", "store", "lexer", "slice", "theme", "panel", "focus",
    "align", "quiet",
];

/// A 64-bit linear congruential generator; the high bits pick words.
struct Words(u64);

impl Words {
    fn next_index(&mut self) -> usize {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        usize::try_from(self.0 >> 59).unwrap_or(0)
    }
}

fn line(words: &mut Words, number: u64) -> [u8; LINE_BYTES] {
    let mut text = format!("{:07} ", number % 10_000_000);
    while text.len() < LINE_BYTES - 1 {
        text.push_str(WORDS.get(words.next_index()).copied().unwrap_or("x"));
        text.push(' ');
    }
    let mut bytes = [b' '; LINE_BYTES];
    if let Some(prefix) = text.as_bytes().get(..LINE_BYTES - 1) {
        bytes[..LINE_BYTES - 1].copy_from_slice(prefix);
    }
    bytes[LINE_BYTES - 1] = b'\n';
    bytes
}

/// Streams the fixture's bytes, one line at a time.
pub fn generate(fixture: Fixture, lines: u64, mut sink: impl FnMut(&[u8])) {
    let mut words = Words(fixture.seed());
    for number in 1..=lines {
        sink(&line(&mut words, number));
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Fnv64(u64);

impl Fnv64 {
    pub fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }

    pub fn update(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 = (self.0 ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3);
        }
    }

    pub fn hex(self) -> String {
        format!("{:016x}", self.0)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FixtureFile {
    pub fixture: Fixture,
    pub path: PathBuf,
    pub bytes: u64,
    pub fnv64: String,
}

/// Writes the fixture unless a file of the right size exists, then hashes
/// the file on disk so the run records what the apps actually opened.
pub fn ensure(dir: &Path, fixture: Fixture) -> Result<FixtureFile, String> {
    fs::create_dir_all(dir).map_err(|error| format!("create {}: {error}", dir.display()))?;
    let path = dir.join(fixture.file_name());
    let expected = fixture.lines() * 64;
    let current = fs::metadata(&path).map(|meta| meta.len()).ok();
    if current != Some(expected) {
        let partial = path.with_extension("partial");
        let file = File::create(&partial)
            .map_err(|error| format!("create {}: {error}", partial.display()))?;
        let mut writer = BufWriter::new(file);
        let mut failure = None;
        generate(fixture, fixture.lines(), |bytes| {
            if failure.is_none() {
                failure = writer.write_all(bytes).err();
            }
        });
        if let Some(error) = failure {
            return Err(format!("write {}: {error}", partial.display()));
        }
        writer
            .flush()
            .map_err(|error| format!("flush {}: {error}", partial.display()))?;
        fs::rename(&partial, &path)
            .map_err(|error| format!("rename {}: {error}", path.display()))?;
    }
    let (bytes, fnv64) = hash_file(&path)?;
    Ok(FixtureFile {
        fixture,
        path,
        bytes,
        fnv64,
    })
}

pub fn hash_file(path: &Path) -> Result<(u64, String), String> {
    let mut file = File::open(path).map_err(|error| format!("open {}: {error}", path.display()))?;
    let mut hash = Fnv64::new();
    let mut total = 0_u64;
    let mut buffer = vec![0_u8; 1 << 20];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("read {}: {error}", path.display()))?;
        if read == 0 {
            break;
        }
        hash.update(buffer.get(..read).unwrap_or_default());
        total += u64::try_from(read).unwrap_or(0);
    }
    Ok((total, hash.hex()))
}

#[cfg(test)]
mod tests {
    use super::{Fnv64, LINE_BYTES, ensure, generate};
    use crate::workload::Fixture;

    fn render(fixture: Fixture, lines: u64) -> Vec<u8> {
        let mut bytes = Vec::new();
        generate(fixture, lines, |line| bytes.extend_from_slice(line));
        bytes
    }

    #[test]
    fn generation_is_deterministic_and_seeded_per_fixture() {
        assert_eq!(render(Fixture::Idle, 50), render(Fixture::Idle, 50));
        assert_ne!(render(Fixture::Idle, 50), render(Fixture::Typing, 50));
    }

    #[test]
    fn every_line_is_sixty_four_ascii_bytes_with_its_number() -> Result<(), String> {
        let bytes = render(Fixture::Caret, 120);
        assert_eq!(bytes.len(), 120 * LINE_BYTES);
        for (index, line) in bytes.chunks(LINE_BYTES).enumerate() {
            assert_eq!(line.last(), Some(&b'\n'));
            assert!(line.iter().all(|byte| byte.is_ascii() && *byte != b'\t'));
            let text = std::str::from_utf8(line).map_err(|error| error.to_string())?;
            assert!(text.starts_with(&format!("{:07} ", index + 1)), "{text}");
        }
        Ok(())
    }

    #[test]
    fn fnv64_matches_the_reference_vector() {
        let mut hash = Fnv64::new();
        hash.update(b"a");
        assert_eq!(hash.hex(), "af63dc4c8601ec8c");
    }

    #[test]
    fn ensure_writes_once_and_hashes_the_file() -> Result<(), String> {
        let dir = crate::test_dir("fixtures")?;
        let first = ensure(&dir, Fixture::Idle)?;
        assert_eq!(first.bytes, Fixture::Idle.lines() * 64);
        let mut expected = Fnv64::new();
        generate(Fixture::Idle, Fixture::Idle.lines(), |line| {
            expected.update(line);
        });
        assert_eq!(first.fnv64, expected.hex());
        let second = ensure(&dir, Fixture::Idle)?;
        assert_eq!(first, second);
        std::fs::remove_dir_all(&dir).map_err(|error| error.to_string())?;
        Ok(())
    }
}
