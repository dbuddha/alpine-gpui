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
const MAX_DOCUMENT_TEXT_BYTES: usize = 33_554_432;
const MAX_RECOVERY_TEXT_BYTES: usize = 67_108_864;
const MAX_RECOVERY_BYTES: usize = 67_371_008;
const MAX_RECOVERY_READ_BYTES: u64 = 67_371_009;
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

/// A stable corruption class for a retained Alpine Studio recovery journal.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecoveryCorrupt {
    /// The fixed recovery header is missing or malformed.
    Header,
    /// The journal uses an unsupported schema version.
    Version(u16),
    /// A declared or retained byte length violates the recovery budget.
    Length,
    /// The retained payload does not match its recorded checksum.
    Checksum,
    /// The journal ends before a required field or payload is complete.
    Truncated,
    /// A retained path or document payload is not valid UTF-8.
    Utf8,
    /// More than one recovery document refers to the same tab.
    DuplicateTab,
    /// A recovery document refers to a tab outside the retained session.
    InvalidTab,
    /// The embedded session state violates its versioned contract.
    Session,
    /// Bytes remain after the complete recovery payload.
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
        let completed_generation = self.shared.completed_generation.load(Ordering::Acquire);
        let last_error = self.shared.last_error.lock().ok().and_then(|error| *error);
        RecoveryStatus {
            published_generation: self.shared.published_generation.load(Ordering::Acquire),
            completed_generation,
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
    validate_encoded_file_length(encoded_bytes)?;
    let encoded_bytes = usize::try_from(encoded_bytes)
        .map_err(|_| RecoveryError::Corrupt(RecoveryCorrupt::Length))?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(encoded_bytes.saturating_add(1))
        .map_err(|_| RecoveryError::AllocationFailed)?;
    map_io(
        file.take(MAX_RECOVERY_READ_BYTES).read_to_end(&mut bytes),
        "read",
    )?;
    validate_buffered_length(bytes.len())?;
    decode(&bytes).map_err(RecoveryError::Corrupt)
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
    enforce_document_count(request.documents.len(), RecoveryError::Invalid)?;
    let mut seen = [false; DOCUMENT_CAPACITY];
    let mut total = 0_usize;
    for document in &request.documents {
        let tab = usize::from(document.tab);
        if tab >= request.session.tabs.len() || seen[tab] {
            return Err(RecoveryError::Invalid);
        }
        seen[tab] = true;
        total = checked_text_total(total, document.base.len_bytes(), document.local.len_bytes())
            .ok_or(RecoveryError::Invalid)?;
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
    let encoded_length = encoded_length(payload.len()).ok_or(RecoveryError::Invalid)?;
    let payload_length = u32::try_from(payload.len()).map_err(|_| RecoveryError::Invalid)?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(encoded_length)
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
    enforce_document_count(count, RecoveryCorrupt::Length)?;
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
        total =
            checked_text_total(total, base.len(), local.len()).ok_or(RecoveryCorrupt::Length)?;
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
        enforce_document_text_length(length, RecoveryCorrupt::Length)?;
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

fn validate_encoded_file_length(length: u64) -> Result<(), RecoveryError> {
    if length > MAX_RECOVERY_BYTES as u64 {
        return Err(RecoveryError::Corrupt(RecoveryCorrupt::Length));
    }
    Ok(())
}

fn validate_buffered_length(length: usize) -> Result<(), RecoveryError> {
    if length > MAX_RECOVERY_BYTES {
        return Err(RecoveryError::Corrupt(RecoveryCorrupt::Length));
    }
    Ok(())
}

fn enforce_document_count<E>(count: usize, error: E) -> Result<(), E> {
    if count > DOCUMENT_CAPACITY {
        return Err(error);
    }
    Ok(())
}

fn enforce_document_text_length<E>(length: usize, error: E) -> Result<(), E> {
    if length > MAX_DOCUMENT_TEXT_BYTES {
        return Err(error);
    }
    Ok(())
}

const fn encoded_length(payload_length: usize) -> Option<usize> {
    let Some(length) = payload_length.checked_add(HEADER_BYTES) else {
        return None;
    };
    if length > MAX_RECOVERY_BYTES {
        return None;
    }
    Some(length)
}

const fn checked_text_total(total: usize, base: usize, local: usize) -> Option<usize> {
    if base > MAX_DOCUMENT_TEXT_BYTES || local > MAX_DOCUMENT_TEXT_BYTES {
        return None;
    }
    let Some(total) = total.checked_add(base) else {
        return None;
    };
    let Some(total) = total.checked_add(local) else {
        return None;
    };
    if total > MAX_RECOVERY_TEXT_BYTES {
        return None;
    }
    Some(total)
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
    atomic_replace_with_sequence(path, bytes, write, &mut || {
        TEMPORARY_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    })
}

fn atomic_replace_with_sequence(
    path: &Path,
    bytes: &[u8],
    write: &mut RecoveryWriter<'_>,
    next_sequence: &mut dyn FnMut() -> u64,
) -> Result<(), RecoveryError> {
    let file_name = path.file_name().ok_or(RecoveryError::Invalid)?;
    let parent = path.parent().ok_or(RecoveryError::Invalid)?;
    let mut created = None;
    for _ in 0..TEMPORARY_ATTEMPTS {
        let temporary = temporary_path(parent, file_name, next_sequence());
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
        sync_parent_directory(parent)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn temporary_path(parent: &Path, file_name: &std::ffi::OsStr, sequence: u64) -> PathBuf {
    let mut name = OsString::from(".");
    name.push(file_name);
    name.push(format!(
        ".alpine-recovery-{}-{sequence}",
        std::process::id()
    ));
    parent.join(name)
}

#[cfg(unix)]
fn sync_parent_directory(parent: &Path) -> Result<(), RecoveryError> {
    let directory = map_io(File::open(parent), "open-directory")?;
    map_io(directory.sync_all(), "sync-directory")
}

#[cfg(not(unix))]
fn sync_parent_directory(_parent: &Path) -> Result<(), RecoveryError> {
    Ok(())
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
        SESSION_NODE_CAPACITY, SESSION_PANE_CAPACITY, SessionAxis, SessionFileTree, SessionNode,
        SessionPane, SessionPanes, SessionTab,
    };
    use alpine_text::{Buffer, ByteOffset, Selection};
    use std::time::{Duration, Instant};

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
            workspace: Some(std::env::temp_dir().join("alpine")),
            tabs: vec![
                SessionTab {
                    path: None,
                    view: view(0),
                },
                SessionTab {
                    path: Some(std::env::temp_dir().join("alpine").join("main.rs")),
                    view: view(1),
                },
            ],
            active_tab: 1,
            panes: SessionPanes {
                nodes,
                panes,
                active_pane: 1,
            },
            file_tree: SessionFileTree::default(),
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

    fn test_root(label: &str) -> Result<PathBuf, io::Error> {
        let root = std::env::temp_dir().join(format!(
            "alpine-recovery-{label}-{}-{}",
            std::process::id(),
            TEMPORARY_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root)?;
        Ok(root)
    }

    fn wrap_payload(payload: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(HEADER_BYTES + payload.len());
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&VERSION.to_le_bytes());
        bytes.extend_from_slice(
            &u32::try_from(payload.len())
                .unwrap_or(u32::MAX)
                .to_le_bytes(),
        );
        bytes.extend_from_slice(&crc32(payload).to_le_bytes());
        bytes.extend_from_slice(payload);
        bytes
    }

    fn session_payload() -> Result<Vec<u8>, RecoveryError> {
        let session =
            session::encode_for_recovery(&session_state()).map_err(|_| RecoveryError::Invalid)?;
        let mut payload = Vec::new();
        put_u32(&mut payload, session.len())?;
        payload.extend_from_slice(&session);
        Ok(payload)
    }

    fn append_document(payload: &mut Vec<u8>, tab: u8, base: &[u8], local: &[u8]) {
        payload.push(tab);
        payload.extend_from_slice(&u32::try_from(base.len()).unwrap_or(u32::MAX).to_le_bytes());
        payload.extend_from_slice(base);
        payload.extend_from_slice(&u32::try_from(local.len()).unwrap_or(u32::MAX).to_le_bytes());
        payload.extend_from_slice(local);
    }

    fn wait_for_generation(
        coordinator: &RecoveryCoordinator,
        generation: u64,
        timeout: Duration,
    ) -> RecoveryStatus {
        let deadline = Instant::now() + timeout;
        loop {
            let status = coordinator.status();
            if status.completed_generation >= generation || Instant::now() >= deadline {
                return status;
            }
            thread::yield_now();
        }
    }

    #[test]
    fn limits_and_checked_totals_enforce_exact_boundaries_without_allocating() {
        assert_eq!(MAX_DOCUMENT_TEXT_BYTES, 33_554_432);
        assert_eq!(MAX_RECOVERY_TEXT_BYTES, 67_108_864);
        assert_eq!(MAX_RECOVERY_BYTES, 67_371_008);
        assert_eq!(MAX_RECOVERY_READ_BYTES, 67_371_009);
        assert_eq!(
            validate_encoded_file_length(MAX_RECOVERY_BYTES as u64),
            Ok(())
        );
        assert_eq!(
            validate_encoded_file_length(MAX_RECOVERY_BYTES as u64 + 1),
            Err(RecoveryError::Corrupt(RecoveryCorrupt::Length))
        );
        assert_eq!(validate_buffered_length(MAX_RECOVERY_BYTES), Ok(()));
        assert_eq!(
            validate_buffered_length(MAX_RECOVERY_BYTES + 1),
            Err(RecoveryError::Corrupt(RecoveryCorrupt::Length))
        );
        assert_eq!(enforce_document_count(DOCUMENT_CAPACITY, 7), Ok(()));
        assert_eq!(enforce_document_count(DOCUMENT_CAPACITY + 1, 7), Err(7));
        assert_eq!(
            enforce_document_text_length(MAX_DOCUMENT_TEXT_BYTES, 7),
            Ok(())
        );
        assert_eq!(
            enforce_document_text_length(MAX_DOCUMENT_TEXT_BYTES + 1, 7),
            Err(7)
        );
        assert_eq!(
            encoded_length(MAX_RECOVERY_BYTES - HEADER_BYTES),
            Some(MAX_RECOVERY_BYTES)
        );
        assert_eq!(encoded_length(MAX_RECOVERY_BYTES - HEADER_BYTES + 1), None);
        assert_eq!(encoded_length(usize::MAX), None);
        assert_eq!(
            checked_text_total(0, MAX_DOCUMENT_TEXT_BYTES, MAX_DOCUMENT_TEXT_BYTES),
            Some(MAX_RECOVERY_TEXT_BYTES)
        );
        assert_eq!(checked_text_total(0, MAX_DOCUMENT_TEXT_BYTES + 1, 0), None);
        assert_eq!(checked_text_total(0, 0, MAX_DOCUMENT_TEXT_BYTES + 1), None);
        assert_eq!(checked_text_total(MAX_RECOVERY_TEXT_BYTES, 1, 0), None);
        assert_eq!(checked_text_total(usize::MAX, 1, 0), None);
        assert_eq!(checked_text_total(usize::MAX - 1, 1, 1), None);
    }

    #[test]
    fn errors_have_specific_human_readable_diagnostics() {
        let corrupt = [
            RecoveryCorrupt::Header,
            RecoveryCorrupt::Version(7),
            RecoveryCorrupt::Length,
            RecoveryCorrupt::Checksum,
            RecoveryCorrupt::Truncated,
            RecoveryCorrupt::Utf8,
            RecoveryCorrupt::DuplicateTab,
            RecoveryCorrupt::InvalidTab,
            RecoveryCorrupt::Session,
            RecoveryCorrupt::TrailingBytes,
        ];
        for error in corrupt {
            assert!(!error.to_string().is_empty());
        }
        let errors = [
            RecoveryError::AllocationFailed,
            RecoveryError::Corrupt(RecoveryCorrupt::Checksum),
            RecoveryError::Invalid,
            RecoveryError::Disconnected,
            RecoveryError::WorkerPanicked,
            RecoveryError::Io {
                operation: "read",
                kind: io::ErrorKind::PermissionDenied,
            },
        ];
        for error in errors {
            assert!(!error.to_string().is_empty());
        }
        assert!(RecoveryCorrupt::Version(7).to_string().contains('7'));
        assert!(errors[5].to_string().contains("read"));
    }

    #[test]
    fn structural_identity_ignores_view_motion_but_detects_every_durable_axis()
    -> Result<(), Box<dyn std::error::Error>> {
        let state = session_state();
        let mut view_only = state.clone();
        view_only.tabs[0].view.scroll_y = 99.0;
        view_only.tabs[0].view.selection = Selection::caret(ByteOffset::new(1));
        view_only.panes.panes[0]
            .as_mut()
            .ok_or("pane")?
            .view
            .scroll_y = 42.0;
        assert!(session_structure_eq(&state, &view_only));

        let mut variants = Vec::new();
        let mut changed = state.clone();
        changed.workspace = None;
        variants.push(changed);
        let mut changed = state.clone();
        changed.active_tab = 0;
        variants.push(changed);
        let mut changed = state.clone();
        changed.tabs.pop();
        variants.push(changed);
        let mut changed = state.clone();
        changed.tabs[1].path = None;
        variants.push(changed);
        let mut changed = state.clone();
        changed.panes.nodes[0] = SessionNode::Empty;
        variants.push(changed);
        let mut changed = state.clone();
        changed.panes.active_pane = 0;
        variants.push(changed);
        let mut changed = state.clone();
        changed.panes.panes[1].as_mut().ok_or("pane")?.tab = 0;
        variants.push(changed);
        for changed in variants {
            assert!(!session_structure_eq(&state, &changed));
        }
        Ok(())
    }

    #[test]
    fn recovery_identity_covers_authority_and_each_buffer_revision() -> Result<(), RecoveryError> {
        let baseline = request("local", 4);
        let identity = RecoveryIdentity::from_request(&baseline)?;
        assert!(identity.matches(&baseline));

        let mut changed = request("local", 5);
        assert!(!identity.matches(&changed));
        changed.authority_revision = 4;
        changed.documents.push(RecoverySnapshot {
            tab: 0,
            base: Buffer::new("").snapshot(),
            local: Buffer::new("dirty").snapshot(),
        });
        assert!(!identity.matches(&changed));

        let mut identity = RecoveryIdentity::from_request(&baseline)?;
        identity.documents[0].0 = 0;
        assert!(!identity.matches(&baseline));
        identity.documents[0] = (1, 1, 0);
        assert!(!identity.matches(&baseline));
        identity.documents[0] = (1, 0, 1);
        assert!(!identity.matches(&baseline));
        Ok(())
    }

    #[test]
    fn request_validation_rejects_invalid_session_capacity_tab_and_duplicate() {
        let mut invalid = request("local", 1);
        invalid.session.active_tab = 9;
        assert_eq!(validate_request(&invalid), Err(RecoveryError::Invalid));

        let mut over_capacity = request("local", 1);
        over_capacity.documents = (0..=DOCUMENT_CAPACITY)
            .map(|_| RecoverySnapshot {
                tab: 0,
                base: Buffer::new("").snapshot(),
                local: Buffer::new("").snapshot(),
            })
            .collect();
        assert_eq!(
            validate_request(&over_capacity),
            Err(RecoveryError::Invalid)
        );

        let mut invalid_tab = request("local", 1);
        invalid_tab.documents[0].tab = 2;
        assert_eq!(validate_request(&invalid_tab), Err(RecoveryError::Invalid));

        let mut duplicate = request("local", 1);
        duplicate.documents.push(RecoverySnapshot {
            tab: 1,
            base: Buffer::new("").snapshot(),
            local: Buffer::new("other").snapshot(),
        });
        assert_eq!(validate_request(&duplicate), Err(RecoveryError::Invalid));
        assert_eq!(validate_request(&request("local", 1)), Ok(()));
    }

    #[test]
    fn malformed_envelopes_are_classified_before_payload_decode()
    -> Result<(), Box<dyn std::error::Error>> {
        let valid = encode(&request("local", 1))?;
        assert_eq!(decode(&[]), Err(RecoveryCorrupt::Header));
        let mut wrong_magic = valid.clone();
        wrong_magic[0] ^= 1;
        assert_eq!(decode(&wrong_magic), Err(RecoveryCorrupt::Header));
        let mut wrong_version = valid.clone();
        wrong_version[8..10].copy_from_slice(&2_u16.to_le_bytes());
        assert_eq!(decode(&wrong_version), Err(RecoveryCorrupt::Version(2)));
        let mut wrong_length = valid.clone();
        wrong_length[10..14].copy_from_slice(&0_u32.to_le_bytes());
        assert_eq!(decode(&wrong_length), Err(RecoveryCorrupt::Length));
        let mut wrong_checksum = valid;
        wrong_checksum[14] ^= 1;
        assert_eq!(decode(&wrong_checksum), Err(RecoveryCorrupt::Checksum));
        Ok(())
    }

    #[test]
    fn malformed_payloads_report_session_tab_text_and_trailing_failures()
    -> Result<(), Box<dyn std::error::Error>> {
        assert_eq!(decode(&wrap_payload(&[])), Err(RecoveryCorrupt::Truncated));

        let mut invalid_session = session_payload()?;
        invalid_session[4] ^= 1;
        assert_eq!(
            decode(&wrap_payload(&invalid_session)),
            Err(RecoveryCorrupt::Session)
        );

        let mut too_many = session_payload()?;
        too_many.push(u8::try_from(DOCUMENT_CAPACITY + 1).unwrap_or(u8::MAX));
        assert_eq!(
            decode(&wrap_payload(&too_many)),
            Err(RecoveryCorrupt::Length)
        );

        let mut invalid_tab = session_payload()?;
        invalid_tab.push(1);
        append_document(&mut invalid_tab, 2, b"", b"");
        assert_eq!(
            decode(&wrap_payload(&invalid_tab)),
            Err(RecoveryCorrupt::InvalidTab)
        );

        let mut duplicate = session_payload()?;
        duplicate.push(2);
        append_document(&mut duplicate, 1, b"", b"a");
        append_document(&mut duplicate, 1, b"", b"b");
        assert_eq!(
            decode(&wrap_payload(&duplicate)),
            Err(RecoveryCorrupt::DuplicateTab)
        );

        let mut invalid_utf8 = session_payload()?;
        invalid_utf8.push(1);
        append_document(&mut invalid_utf8, 1, &[0xff], b"");
        assert_eq!(
            decode(&wrap_payload(&invalid_utf8)),
            Err(RecoveryCorrupt::Utf8)
        );

        let mut trailing = session_payload()?;
        trailing.extend_from_slice(&[0, 7]);
        assert_eq!(
            decode(&wrap_payload(&trailing)),
            Err(RecoveryCorrupt::TrailingBytes)
        );
        Ok(())
    }

    #[test]
    fn reader_and_integer_codec_reject_truncation_overflow_and_oversized_text() {
        let mut reader = Reader::new(&[7, 1, 0, 0, 0, b'x']);
        assert_eq!(reader.u8(), Ok(7));
        assert_eq!(reader.u32(), Ok(1));
        assert_eq!(reader.text(), Err(RecoveryCorrupt::Truncated));
        assert!(!reader.is_empty());
        assert_eq!(reader.u8(), Ok(b'x'));
        assert!(reader.is_empty());

        let oversized_length =
            (u32::try_from(MAX_DOCUMENT_TEXT_BYTES).unwrap_or(u32::MAX) + 1).to_le_bytes();
        let mut oversized = Reader::new(&oversized_length);
        assert_eq!(oversized.text(), Err(RecoveryCorrupt::Length));
        let mut truncated = Reader::new(&[1, 2, 3]);
        assert_eq!(truncated.u32(), Err(RecoveryCorrupt::Truncated));
        truncated.cursor = usize::MAX;
        assert_eq!(truncated.take(1), Err(RecoveryCorrupt::Truncated));

        let mut bytes = Vec::new();
        assert_eq!(put_u32(&mut bytes, 513), Ok(()));
        assert_eq!(bytes, 513_u32.to_le_bytes());
        #[cfg(target_pointer_width = "64")]
        assert_eq!(
            put_u32(&mut bytes, u32::MAX as usize + 1),
            Err(RecoveryError::Invalid)
        );
    }

    #[test]
    fn path_load_preserves_specific_failures() -> Result<(), Box<dyn std::error::Error>> {
        let root = test_root("load")?;
        #[cfg(unix)]
        assert!(matches!(
            load(&root),
            Err(RecoveryError::Io {
                operation: "read",
                ..
            })
        ));
        let session_path = root.join("state").join("session-v1.bin");
        assert_eq!(
            path_for_session(&session_path),
            root.join("state").join("recovery-v1.bin")
        );
        let path = root.join("recovery.bin");
        assert!(matches!(
            load(&path),
            Err(RecoveryError::Io {
                operation: "open",
                kind: io::ErrorKind::NotFound
            })
        ));
        fs::write(&path, b"corrupt")?;
        assert_eq!(
            load(&path),
            Err(RecoveryError::Corrupt(RecoveryCorrupt::Header))
        );
        let oversized = root.join("oversized.bin");
        File::create(&oversized)?.set_len(MAX_RECOVERY_BYTES as u64 + 1)?;
        assert_eq!(
            load(&oversized),
            Err(RecoveryError::Corrupt(RecoveryCorrupt::Length))
        );
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn atomic_replace_retries_collisions_and_reports_exhaustion()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = test_root("collision")?;
        let path = root.join("recovery.bin");
        let collision = temporary_path(&root, path.file_name().ok_or("file name")?, 7);
        fs::write(&collision, b"occupied")?;
        let mut sequences = [7_u64, 8].into_iter();
        let mut next = || sequences.next().unwrap_or(8);
        let mut write = |file: &mut File, bytes: &[u8]| file.write_all(bytes);
        atomic_replace_with_sequence(&path, b"new", &mut write, &mut next)?;
        assert_eq!(fs::read(&path)?, b"new");
        assert_eq!(fs::read(&collision)?, b"occupied");

        let exhausted = root.join("exhausted.bin");
        let occupied = temporary_path(&root, exhausted.file_name().ok_or("file name")?, 9);
        fs::write(&occupied, b"occupied")?;
        let mut same = || 9;
        assert_eq!(
            atomic_replace_with_sequence(&exhausted, b"new", &mut write, &mut same),
            Err(RecoveryError::Io {
                operation: "create-temporary",
                kind: io::ErrorKind::AlreadyExists,
            })
        );
        assert!(!exhausted.exists());

        let missing_parent = root.join("missing").join("recovery.bin");
        let mut next = || 10;
        assert!(matches!(
            atomic_replace_with_sequence(&missing_parent, b"new", &mut write, &mut next),
            Err(RecoveryError::Io {
                operation: "create-temporary",
                kind: io::ErrorKind::NotFound
            })
        ));
        assert_eq!(
            atomic_replace(Path::new("/"), b"new"),
            Err(RecoveryError::Invalid)
        );
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn coordinator_retries_identical_state_after_worker_failure()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = test_root("worker-failure")?;
        let blocked_parent = root.join("not-a-directory");
        fs::write(&blocked_parent, b"file")?;
        let mut coordinator = RecoveryCoordinator::new(blocked_parent.join("recovery.bin"))?;
        let first = coordinator.publish(request("dirty", 1))?;
        let status = wait_for_generation(&coordinator, first, Duration::from_secs(2));
        assert_eq!(status.completed_generation, first);
        assert!(matches!(
            status.last_error,
            Some(RecoveryError::Io {
                operation: "create-directory",
                kind: io::ErrorKind::AlreadyExists
            })
        ));
        let second = coordinator.publish(request("dirty", 1))?;
        assert!(second > first);
        let status = coordinator.shutdown()?;
        assert_eq!(status.completed_generation, second);
        assert!(status.last_error.is_some());
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn disconnected_signal_panicked_worker_and_wait_timeout_are_structured() {
        let new_shared = || {
            Arc::new(Shared {
                latest: Mutex::new(None),
                identity: Mutex::new(None),
                last_error: Mutex::new(None),
                published_generation: AtomicU64::new(0),
                completed_generation: AtomicU64::new(0),
            })
        };
        let (signal, receiver) = mpsc::sync_channel(1);
        drop(receiver);
        let disconnected = RecoveryCoordinator {
            shared: new_shared(),
            signal: Some(signal),
            worker: None,
        };
        assert_eq!(
            disconnected.publish(request("dirty", 1)),
            Err(RecoveryError::Disconnected)
        );

        let waiting = RecoveryCoordinator {
            shared: new_shared(),
            signal: None,
            worker: None,
        };
        assert_eq!(
            wait_for_generation(&waiting, 1, Duration::from_millis(1)),
            RecoveryStatus {
                published_generation: 0,
                completed_generation: 0,
                last_error: None,
            }
        );

        let worker = thread::spawn(|| std::panic::resume_unwind(Box::new(())));
        let mut panicked = RecoveryCoordinator {
            shared: new_shared(),
            signal: None,
            worker: Some(worker),
        };
        assert_eq!(panicked.shutdown(), Err(RecoveryError::WorkerPanicked));
    }

    #[test]
    fn coordinator_drop_drains_the_last_published_generation()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = test_root("drop")?;
        let path = root.join("recovery.bin");
        {
            let coordinator = RecoveryCoordinator::new(path.clone())?;
            assert_eq!(coordinator.publish(request("latest", 1))?, 1);
        }
        assert_eq!(&*load(&path)?.documents[0].local, "latest");
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn crc_and_io_mapping_have_stable_external_evidence() {
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
        let mapped = map_io::<()>(
            Err(io::Error::new(io::ErrorKind::BrokenPipe, "injected")),
            "read",
        );
        assert_eq!(
            mapped,
            Err(RecoveryError::Io {
                operation: "read",
                kind: io::ErrorKind::BrokenPipe,
            })
        );
    }

    #[cfg(unix)]
    #[test]
    fn directory_sync_reports_a_missing_parent() -> Result<(), Box<dyn std::error::Error>> {
        let root = test_root("directory-sync")?;
        let missing = root.join("missing");
        assert_eq!(
            sync_parent_directory(&missing),
            Err(RecoveryError::Io {
                operation: "open-directory",
                kind: io::ErrorKind::NotFound,
            })
        );
        fs::remove_dir_all(root)?;
        Ok(())
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
