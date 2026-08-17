//! Bounded asynchronous dirty-buffer recovery persistence.

use std::ffi::OsString;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

use alpine_text::BufferSnapshot;

use crate::session::{self, SessionState};

const MAGIC: &[u8; 8] = b"ALPNRCVR";
const VERSION: u16 = 1;
const HEADER_BYTES: usize = 18;
const DOCUMENT_CAPACITY: usize = 32;
const MAX_DOCUMENT_TEXT_BYTES: usize = 32 * 1_024 * 1_024;
const MAX_RECOVERY_TEXT_BYTES: usize = 64 * 1_024 * 1_024;
const MAX_RECOVERY_BYTES: usize = MAX_RECOVERY_TEXT_BYTES + 256 * 1_024;
const TEMPORARY_ATTEMPTS: usize = 16;
static TEMPORARY_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RecoveredDocument {
    pub(crate) tab: u8,
    pub(crate) base: Box<str>,
    pub(crate) local: Box<str>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RecoveryState {
    pub(crate) session: SessionState,
    pub(crate) documents: Vec<RecoveredDocument>,
}

#[derive(Clone)]
pub(crate) struct RecoverySnapshot {
    pub(crate) tab: u8,
    pub(crate) base: BufferSnapshot,
    pub(crate) local: BufferSnapshot,
}

pub(crate) struct RecoveryRequest {
    pub(crate) session: SessionState,
    pub(crate) documents: Vec<RecoverySnapshot>,
    pub(crate) authority_revision: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RecoveryCorrupt {
    Header,
    Version(u16),
    Length,
    Checksum,
    Truncated,
    Utf8,
    DuplicateTab,
    InvalidTab,
    Session,
    TrailingBytes,
}

impl fmt::Display for RecoveryCorrupt {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Header => formatter.write_str("recovery header is invalid"),
            Self::Version(version) => {
                write!(formatter, "recovery version {version} is unsupported")
            }
            Self::Length => formatter.write_str("recovery byte length is invalid"),
            Self::Checksum => formatter.write_str("recovery checksum does not match"),
            Self::Truncated => formatter.write_str("recovery payload is truncated"),
            Self::Utf8 => formatter.write_str("recovery text is not valid UTF-8"),
            Self::DuplicateTab => formatter.write_str("recovery contains a duplicate tab"),
            Self::InvalidTab => formatter.write_str("recovery references an invalid tab"),
            Self::Session => formatter.write_str("recovery session state is invalid"),
            Self::TrailingBytes => formatter.write_str("recovery contains trailing bytes"),
        }
    }
}

impl std::error::Error for RecoveryCorrupt {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RecoveryError {
    AllocationFailed,
    Corrupt(RecoveryCorrupt),
    Invalid,
    PendingDirty,
    Disconnected,
    WorkerPanicked,
    Io {
        operation: &'static str,
        kind: io::ErrorKind,
    },
}

impl fmt::Display for RecoveryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AllocationFailed => formatter.write_str("recovery allocation failed"),
            Self::Corrupt(error) => write!(formatter, "recovery is corrupt: {error}"),
            Self::Invalid => formatter.write_str("recovery state is invalid or exceeds its budget"),
            Self::PendingDirty => {
                formatter.write_str("an unresolved dirty recovery journal already exists")
            }
            Self::Disconnected => formatter.write_str("recovery worker is unavailable"),
            Self::WorkerPanicked => formatter.write_str("recovery worker terminated unexpectedly"),
            Self::Io { operation, kind } => {
                write!(formatter, "recovery {operation} failed with {kind:?}")
            }
        }
    }
}

impl std::error::Error for RecoveryError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RecoveryStatus {
    pub(crate) published_generation: u64,
    pub(crate) completed_generation: u64,
    pub(crate) last_error: Option<RecoveryError>,
}

struct Pending {
    generation: u64,
    request: RecoveryRequest,
}

struct Shared {
    latest: Mutex<Option<Pending>>,
    identity: Mutex<Option<RecoveryIdentity>>,
    last_error: Mutex<Option<RecoveryError>>,
    published_generation: AtomicU64,
    completed_generation: AtomicU64,
}

struct RecoveryIdentity {
    session: SessionState,
    documents: Vec<(u8, u64, u64)>,
    authority_revision: u64,
}

impl RecoveryIdentity {
    fn matches(&self, request: &RecoveryRequest) -> bool {
        session_structure_eq(&self.session, &request.session)
            && self.authority_revision == request.authority_revision
            && self.documents.len() == request.documents.len()
            && self.documents.iter().zip(&request.documents).all(
                |((tab, base, local), document)| {
                    *tab == document.tab
                        && *base == document.base.revision().get()
                        && *local == document.local.revision().get()
                },
            )
    }

    fn from_request(request: &RecoveryRequest) -> Result<Self, RecoveryError> {
        let mut documents = Vec::new();
        documents
            .try_reserve_exact(request.documents.len())
            .map_err(|_| RecoveryError::AllocationFailed)?;
        documents.extend(request.documents.iter().map(|document| {
            (
                document.tab,
                document.base.revision().get(),
                document.local.revision().get(),
            )
        }));
        Ok(Self {
            session: request.session.clone(),
            documents,
            authority_revision: request.authority_revision,
        })
    }
}

pub(crate) struct RecoveryCoordinator {
    shared: Arc<Shared>,
    signal: Option<SyncSender<()>>,
    worker: Option<JoinHandle<()>>,
}

impl RecoveryCoordinator {
    pub(crate) fn new(path: PathBuf) -> Result<Self, RecoveryError> {
        let shared = Arc::new(Shared {
            latest: Mutex::new(None),
            identity: Mutex::new(None),
            last_error: Mutex::new(None),
            published_generation: AtomicU64::new(0),
            completed_generation: AtomicU64::new(0),
        });
        let (signal, receiver) = mpsc::sync_channel(1);
        let worker_shared = Arc::clone(&shared);
        let worker = thread::Builder::new()
            .name(String::from("alpine-recovery"))
            .spawn(move || worker_loop(&path, &worker_shared, &receiver))
            .map_err(|error| io_error("spawn-worker", &error))?;
        Ok(Self {
            shared,
            signal: Some(signal),
            worker: Some(worker),
        })
    }

    pub(crate) fn publish(&self, request: RecoveryRequest) -> Result<u64, RecoveryError> {
        validate_request(&request)?;
        let mut identity = self
            .shared
            .identity
            .lock()
            .map_err(|_| RecoveryError::Disconnected)?;
        if identity
            .as_ref()
            .is_some_and(|identity| identity.matches(&request))
        {
            return Ok(self.shared.published_generation.load(Ordering::Acquire));
        }
        *identity = Some(RecoveryIdentity::from_request(&request)?);
        drop(identity);
        let generation = self
            .shared
            .published_generation
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                value.checked_add(1)
            })
            .map_err(|_| RecoveryError::Invalid)?
            .checked_add(1)
            .ok_or(RecoveryError::Invalid)?;
        let mut latest = self
            .shared
            .latest
            .lock()
            .map_err(|_| RecoveryError::Disconnected)?;
        *latest = Some(Pending {
            generation,
            request,
        });
        drop(latest);
        let signal = self.signal.as_ref().ok_or(RecoveryError::Disconnected)?;
        match signal.try_send(()) {
            Ok(()) | Err(TrySendError::Full(())) => Ok(generation),
            Err(TrySendError::Disconnected(())) => Err(RecoveryError::Disconnected),
        }
    }

    pub(crate) fn status(&self) -> RecoveryStatus {
        let last_error = self.shared.last_error.lock().ok().and_then(|error| *error);
        RecoveryStatus {
            published_generation: self.shared.published_generation.load(Ordering::Acquire),
            completed_generation: self.shared.completed_generation.load(Ordering::Acquire),
            last_error,
        }
    }

    pub(crate) fn shutdown(&mut self) -> Result<RecoveryStatus, RecoveryError> {
        self.signal.take();
        if self
            .worker
            .take()
            .is_some_and(|worker| worker.join().is_err())
        {
            return Err(RecoveryError::WorkerPanicked);
        }
        Ok(self.status())
    }
}

impl Drop for RecoveryCoordinator {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

pub(crate) fn path_for_session(session_path: &Path) -> PathBuf {
    let mut path = session_path.to_path_buf();
    path.set_file_name("recovery-v1.bin");
    path
}

pub(crate) fn load(path: &Path) -> Result<RecoveryState, RecoveryError> {
    let file = map_io(File::open(path), "open")?;
    let encoded_bytes = map_io(file.metadata(), "metadata")?.len();
    if encoded_bytes > u64::try_from(MAX_RECOVERY_BYTES).unwrap_or(u64::MAX) {
        return Err(RecoveryError::Corrupt(RecoveryCorrupt::Length));
    }
    let encoded_bytes = usize::try_from(encoded_bytes)
        .map_err(|_| RecoveryError::Corrupt(RecoveryCorrupt::Length))?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(encoded_bytes.saturating_add(1))
        .map_err(|_| RecoveryError::AllocationFailed)?;
    map_io(
        file.take(u64::try_from(MAX_RECOVERY_BYTES + 1).unwrap_or(u64::MAX))
            .read_to_end(&mut bytes),
        "read",
    )?;
    if bytes.len() > MAX_RECOVERY_BYTES {
        return Err(RecoveryError::Corrupt(RecoveryCorrupt::Length));
    }
    decode(&bytes).map_err(RecoveryError::Corrupt)
}

pub(crate) fn ensure_replaceable(path: &Path) -> Result<(), RecoveryError> {
    match load(path) {
        Ok(state) if state.documents.is_empty() => Ok(()),
        Ok(_) => Err(RecoveryError::PendingDirty),
        Err(RecoveryError::Io {
            kind: io::ErrorKind::NotFound,
            ..
        }) => Ok(()),
        Err(error) => Err(error),
    }
}

fn worker_loop(path: &Path, shared: &Shared, receiver: &Receiver<()>) {
    while receiver.recv().is_ok() {
        loop {
            let pending = shared
                .latest
                .lock()
                .ok()
                .and_then(|mut latest| latest.take());
            let Some(pending) = pending else {
                break;
            };
            let result = encode(&pending.request).and_then(|bytes| {
                let parent = path.parent().ok_or(RecoveryError::Invalid)?;
                map_io(fs::create_dir_all(parent), "create-directory")?;
                atomic_replace(path, &bytes)
            });
            let error = result.err();
            let failed = error.is_some();
            if let Ok(mut last_error) = shared.last_error.lock() {
                *last_error = error;
            }
            if failed && let Ok(mut identity) = shared.identity.lock() {
                *identity = None;
            }
            shared
                .completed_generation
                .store(pending.generation, Ordering::Release);
        }
    }
}

fn session_structure_eq(left: &SessionState, right: &SessionState) -> bool {
    left.workspace == right.workspace
        && left.active_tab == right.active_tab
        && left.tabs.len() == right.tabs.len()
        && left
            .tabs
            .iter()
            .zip(&right.tabs)
            .all(|(left, right)| left.path == right.path)
        && left.panes.nodes == right.panes.nodes
        && left.panes.active_pane == right.panes.active_pane
        && left
            .panes
            .panes
            .iter()
            .zip(&right.panes.panes)
            .all(|(left, right)| left.map(|pane| pane.tab) == right.map(|pane| pane.tab))
}

fn validate_request(request: &RecoveryRequest) -> Result<(), RecoveryError> {
    session::validate(&request.session).map_err(|_| RecoveryError::Invalid)?;
    if request.documents.len() > DOCUMENT_CAPACITY {
        return Err(RecoveryError::Invalid);
    }
    let mut seen = [false; DOCUMENT_CAPACITY];
    let mut total = 0_usize;
    for document in &request.documents {
        let tab = usize::from(document.tab);
        if tab >= request.session.tabs.len() || seen[tab] {
            return Err(RecoveryError::Invalid);
        }
        seen[tab] = true;
        let base = document.base.len_bytes();
        let local = document.local.len_bytes();
        if base > MAX_DOCUMENT_TEXT_BYTES || local > MAX_DOCUMENT_TEXT_BYTES {
            return Err(RecoveryError::Invalid);
        }
        total = total
            .checked_add(base)
            .and_then(|value| value.checked_add(local))
            .ok_or(RecoveryError::Invalid)?;
        if total > MAX_RECOVERY_TEXT_BYTES {
            return Err(RecoveryError::Invalid);
        }
    }
    Ok(())
}

fn encode(request: &RecoveryRequest) -> Result<Vec<u8>, RecoveryError> {
    validate_request(request)?;
    let session =
        session::encode_for_recovery(&request.session).map_err(|_| RecoveryError::Invalid)?;
    let mut payload = Vec::new();
    payload
        .try_reserve_exact(session.len().saturating_add(256))
        .map_err(|_| RecoveryError::AllocationFailed)?;
    put_u32(&mut payload, session.len())?;
    payload.extend_from_slice(&session);
    payload.push(u8::try_from(request.documents.len()).map_err(|_| RecoveryError::Invalid)?);
    for document in &request.documents {
        payload.push(document.tab);
        let base = document.base.text();
        let local = document.local.text();
        put_u32(&mut payload, base.len())?;
        payload.extend_from_slice(base.as_bytes());
        put_u32(&mut payload, local.len())?;
        payload.extend_from_slice(local.as_bytes());
    }
    if payload.len().saturating_add(HEADER_BYTES) > MAX_RECOVERY_BYTES {
        return Err(RecoveryError::Invalid);
    }
    let payload_length = u32::try_from(payload.len()).map_err(|_| RecoveryError::Invalid)?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(HEADER_BYTES + payload.len())
        .map_err(|_| RecoveryError::AllocationFailed)?;
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(&VERSION.to_le_bytes());
    bytes.extend_from_slice(&payload_length.to_le_bytes());
    bytes.extend_from_slice(&crc32(&payload).to_le_bytes());
    bytes.extend_from_slice(&payload);
    Ok(bytes)
}

fn decode(bytes: &[u8]) -> Result<RecoveryState, RecoveryCorrupt> {
    if bytes.len() < HEADER_BYTES || bytes.get(..8) != Some(MAGIC) {
        return Err(RecoveryCorrupt::Header);
    }
    let version = u16::from_le_bytes([bytes[8], bytes[9]]);
    if version != VERSION {
        return Err(RecoveryCorrupt::Version(version));
    }
    let payload_length = u32::from_le_bytes([bytes[10], bytes[11], bytes[12], bytes[13]]) as usize;
    if payload_length.saturating_add(HEADER_BYTES) != bytes.len() {
        return Err(RecoveryCorrupt::Length);
    }
    let expected = u32::from_le_bytes([bytes[14], bytes[15], bytes[16], bytes[17]]);
    let payload = &bytes[HEADER_BYTES..];
    if crc32(payload) != expected {
        return Err(RecoveryCorrupt::Checksum);
    }
    let mut reader = Reader::new(payload);
    let session_length = reader.u32()?;
    let session = session::decode_for_recovery(reader.take(session_length)?)
        .map_err(|_| RecoveryCorrupt::Session)?;
    let count = usize::from(reader.u8()?);
    if count > DOCUMENT_CAPACITY {
        return Err(RecoveryCorrupt::Length);
    }
    let mut documents = Vec::new();
    documents
        .try_reserve_exact(count)
        .map_err(|_| RecoveryCorrupt::Length)?;
    let mut seen = [false; DOCUMENT_CAPACITY];
    let mut total = 0_usize;
    for _ in 0..count {
        let tab = reader.u8()?;
        let tab_index = usize::from(tab);
        if tab_index >= session.tabs.len() {
            return Err(RecoveryCorrupt::InvalidTab);
        }
        if seen[tab_index] {
            return Err(RecoveryCorrupt::DuplicateTab);
        }
        seen[tab_index] = true;
        let base = reader.text()?;
        let local = reader.text()?;
        total = total
            .checked_add(base.len())
            .and_then(|value| value.checked_add(local.len()))
            .ok_or(RecoveryCorrupt::Length)?;
        if total > MAX_RECOVERY_TEXT_BYTES {
            return Err(RecoveryCorrupt::Length);
        }
        documents.push(RecoveredDocument { tab, base, local });
    }
    if !reader.is_empty() {
        return Err(RecoveryCorrupt::TrailingBytes);
    }
    Ok(RecoveryState { session, documents })
}

struct Reader<'a> {
    bytes: &'a [u8],
    cursor: usize,
}

impl<'a> Reader<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, cursor: 0 }
    }

    fn u8(&mut self) -> Result<u8, RecoveryCorrupt> {
        let byte = *self
            .bytes
            .get(self.cursor)
            .ok_or(RecoveryCorrupt::Truncated)?;
        self.cursor += 1;
        Ok(byte)
    }

    fn u32(&mut self) -> Result<usize, RecoveryCorrupt> {
        let bytes = self.take(4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize)
    }

    fn text(&mut self) -> Result<Box<str>, RecoveryCorrupt> {
        let length = self.u32()?;
        if length > MAX_DOCUMENT_TEXT_BYTES {
            return Err(RecoveryCorrupt::Length);
        }
        let bytes = self.take(length)?;
        let text = std::str::from_utf8(bytes).map_err(|_| RecoveryCorrupt::Utf8)?;
        Ok(Box::from(text))
    }

    fn take(&mut self, length: usize) -> Result<&'a [u8], RecoveryCorrupt> {
        let end = self
            .cursor
            .checked_add(length)
            .ok_or(RecoveryCorrupt::Truncated)?;
        let bytes = self
            .bytes
            .get(self.cursor..end)
            .ok_or(RecoveryCorrupt::Truncated)?;
        self.cursor = end;
        Ok(bytes)
    }

    const fn is_empty(&self) -> bool {
        self.cursor == self.bytes.len()
    }
}

fn put_u32(bytes: &mut Vec<u8>, value: usize) -> Result<(), RecoveryError> {
    bytes.extend_from_slice(
        &u32::try_from(value)
            .map_err(|_| RecoveryError::Invalid)?
            .to_le_bytes(),
    );
    Ok(())
}

fn atomic_replace(path: &Path, bytes: &[u8]) -> Result<(), RecoveryError> {
    let mut write = |file: &mut File, value: &[u8]| file.write_all(value);
    atomic_replace_with(path, bytes, &mut write)
}

type RecoveryWriter<'a> = dyn FnMut(&mut File, &[u8]) -> io::Result<()> + 'a;

fn atomic_replace_with(
    path: &Path,
    bytes: &[u8],
    write: &mut RecoveryWriter<'_>,
) -> Result<(), RecoveryError> {
    let file_name = path.file_name().ok_or(RecoveryError::Invalid)?;
    let parent = path.parent().ok_or(RecoveryError::Invalid)?;
    let mut created = None;
    for _ in 0..TEMPORARY_ATTEMPTS {
        let sequence = TEMPORARY_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let mut name = OsString::from(".");
        name.push(file_name);
        name.push(format!(
            ".alpine-recovery-{}-{sequence}",
            std::process::id()
        ));
        let temporary = parent.join(name);
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(&temporary) {
            Ok(file) => {
                created = Some((temporary, file));
                break;
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(io_error("create-temporary", &error)),
        }
    }
    let (temporary, mut file) = created.ok_or(RecoveryError::Io {
        operation: "create-temporary",
        kind: io::ErrorKind::AlreadyExists,
    })?;
    let result = (|| {
        map_io(write(&mut file, bytes), "write")?;
        map_io(file.flush(), "flush")?;
        map_io(file.sync_all(), "sync-file")?;
        drop(file);
        map_io(fs::rename(&temporary, path), "replace")?;
        let directory = map_io(File::open(parent), "open-directory")?;
        map_io(directory.sync_all(), "sync-directory")
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = 0_u32.wrapping_sub(crc & 1);
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

fn map_io<T>(result: io::Result<T>, operation: &'static str) -> Result<T, RecoveryError> {
    result.map_err(|error| io_error(operation, &error))
}

fn io_error(operation: &'static str, error: &io::Error) -> RecoveryError {
    RecoveryError::Io {
        operation,
        kind: error.kind(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::documents::DocumentViewState;
    use crate::session::{
        SESSION_NODE_CAPACITY, SESSION_PANE_CAPACITY, SessionAxis, SessionNode, SessionPane,
        SessionPanes, SessionTab,
    };
    use alpine_text::{Buffer, ByteOffset, Selection};

    fn session_state() -> SessionState {
        let mut nodes = [SessionNode::Empty; SESSION_NODE_CAPACITY];
        nodes[0] = SessionNode::Split {
            axis: SessionAxis::Columns,
            first: 1,
            second: 2,
        };
        nodes[1] = SessionNode::Leaf { pane: 0 };
        nodes[2] = SessionNode::Leaf { pane: 1 };
        let view = |offset| DocumentViewState {
            selection: Selection::caret(ByteOffset::new(offset)),
            scroll_y: 0.0,
        };
        let mut panes = [None; SESSION_PANE_CAPACITY];
        panes[0] = Some(SessionPane {
            tab: 0,
            view: view(0),
        });
        panes[1] = Some(SessionPane {
            tab: 1,
            view: view(1),
        });
        SessionState {
            workspace: Some(PathBuf::from("/tmp/alpine")),
            tabs: vec![
                SessionTab {
                    path: None,
                    view: view(0),
                },
                SessionTab {
                    path: Some(PathBuf::from("/tmp/alpine/main.rs")),
                    view: view(1),
                },
            ],
            active_tab: 1,
            panes: SessionPanes {
                nodes,
                panes,
                active_pane: 1,
            },
        }
    }

    fn request(local: &str, authority_revision: u64) -> RecoveryRequest {
        RecoveryRequest {
            session: session_state(),
            documents: vec![RecoverySnapshot {
                tab: 1,
                base: Buffer::new("base").snapshot(),
                local: Buffer::new(local).snapshot(),
            }],
            authority_revision,
        }
    }

    #[test]
    fn codec_round_trips_exact_session_base_and_local_text()
    -> Result<(), Box<dyn std::error::Error>> {
        let encoded = encode(&request("local\ntext", 1))?;
        let decoded = decode(&encoded)?;
        assert_eq!(decoded.session, session_state());
        assert_eq!(decoded.documents.len(), 1);
        assert_eq!(decoded.documents[0].tab, 1);
        assert_eq!(&*decoded.documents[0].base, "base");
        assert_eq!(&*decoded.documents[0].local, "local\ntext");
        Ok(())
    }

    #[test]
    fn corruption_equal_text_and_duplicate_tabs_are_classified_exactly()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut encoded = encode(&request("local", 1))?;
        encoded[HEADER_BYTES] ^= 1;
        assert_eq!(decode(&encoded), Err(RecoveryCorrupt::Checksum));
        assert_eq!(
            decode(&encoded[..HEADER_BYTES - 1]),
            Err(RecoveryCorrupt::Header)
        );

        let clean = RecoveryRequest {
            session: session_state(),
            documents: vec![RecoverySnapshot {
                tab: 1,
                base: Buffer::new("same").snapshot(),
                local: Buffer::new("same").snapshot(),
            }],
            authority_revision: 1,
        };
        let clean = decode(&encode(&clean)?)?;
        assert_eq!(clean.documents[0].base, clean.documents[0].local);
        let mut duplicate = request("local", 1);
        duplicate.documents.push(RecoverySnapshot {
            tab: 1,
            base: Buffer::new("base").snapshot(),
            local: Buffer::new("other").snapshot(),
        });
        assert_eq!(validate_request(&duplicate), Err(RecoveryError::Invalid));
        Ok(())
    }

    #[test]
    fn unresolved_dirty_recovery_blocks_replacement_but_clean_state_does_not()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = std::env::temp_dir().join(format!(
            "alpine-recovery-replacement-{}-{}",
            std::process::id(),
            TEMPORARY_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root)?;
        let path = root.join("recovery.bin");
        atomic_replace(&path, &encode(&request("dirty", 1))?)?;
        assert_eq!(ensure_replaceable(&path), Err(RecoveryError::PendingDirty));
        let clean = RecoveryRequest {
            session: session_state(),
            documents: Vec::new(),
            authority_revision: 2,
        };
        atomic_replace(&path, &encode(&clean)?)?;
        assert_eq!(ensure_replaceable(&path), Ok(()));
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn coordinator_coalesces_and_drains_the_latest_generation()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = std::env::temp_dir().join(format!(
            "alpine-recovery-{}-{}",
            std::process::id(),
            TEMPORARY_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root)?;
        let path = root.join("recovery.bin");
        let mut coordinator = RecoveryCoordinator::new(path.clone())?;
        let first = coordinator.publish(request("first", 1))?;
        assert_eq!(coordinator.publish(request("first", 1))?, first);
        let second = coordinator.publish(request("second", 2))?;
        assert!(second > first);
        let status = coordinator.shutdown()?;
        assert_eq!(status.published_generation, second);
        assert_eq!(status.completed_generation, second);
        assert_eq!(status.last_error, None);
        assert_eq!(&*load(&path)?.documents[0].local, "second");
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn failed_atomic_write_preserves_the_prior_journal_and_cleans_temporary_state()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = std::env::temp_dir().join(format!(
            "alpine-recovery-failure-{}-{}",
            std::process::id(),
            TEMPORARY_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root)?;
        let path = root.join("recovery.bin");
        atomic_replace(&path, b"known-good")?;
        let mut fail = |file: &mut File, _value: &[u8]| {
            file.write_all(b"partial")?;
            Err(io::Error::new(io::ErrorKind::WriteZero, "injected"))
        };
        assert_eq!(
            atomic_replace_with(&path, b"replacement", &mut fail),
            Err(RecoveryError::Io {
                operation: "write",
                kind: io::ErrorKind::WriteZero,
            })
        );
        assert_eq!(fs::read(&path)?, b"known-good");
        assert_eq!(fs::read_dir(&root)?.count(), 1);
        fs::remove_dir_all(root)?;
        Ok(())
    }
}
