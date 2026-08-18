//! Bounded, crash-safe local workspace session persistence.

use std::ffi::{OsStr, OsString};
use std::fmt;
use std::fs::{self, File, OpenOptions};
#[cfg(any(test, all(target_os = "macos", target_arch = "aarch64")))]
use std::io::Read;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(any(test, all(target_os = "macos", target_arch = "aarch64")))]
use alpine_text::{ByteOffset, Selection};

use crate::documents::DocumentViewState;

pub(crate) const SESSION_NODE_CAPACITY: usize = 7;
pub(crate) const SESSION_PANE_CAPACITY: usize = 4;
const SESSION_TAB_CAPACITY: usize = 32;
const MAX_PATH_BYTES: usize = 4_096;
const MAX_RETAINED_PATH_BYTES: usize = 65_536;
const MAX_SESSION_BYTES: usize = 131_072;
const MAX_READ_BYTES: u64 = 131_073;
const HEADER_BYTES: usize = 18;
const MAGIC: &[u8; 8] = b"ALPNSESS";
const VERSION: u16 = 1;
const TEMPORARY_ATTEMPTS: usize = 16;
static TEMPORARY_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SessionAxis {
    Columns,
    Rows,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SessionNode {
    Empty,
    Leaf {
        pane: u8,
    },
    Split {
        axis: SessionAxis,
        first: u8,
        second: u8,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SessionPane {
    pub(crate) tab: u8,
    pub(crate) view: DocumentViewState,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SessionPanes {
    pub(crate) nodes: [SessionNode; SESSION_NODE_CAPACITY],
    pub(crate) panes: [Option<SessionPane>; SESSION_PANE_CAPACITY],
    pub(crate) active_pane: u8,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SessionTab {
    pub(crate) path: Option<PathBuf>,
    pub(crate) view: DocumentViewState,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SessionState {
    pub(crate) workspace: Option<PathBuf>,
    pub(crate) tabs: Vec<SessionTab>,
    pub(crate) active_tab: u8,
    pub(crate) panes: SessionPanes,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SessionInvalid {
    NoTabs,
    TooManyTabs,
    ActiveTab,
    ScratchPosition,
    DuplicatePath,
    RelativePath,
    PathTooLong,
    PathBudget,
    InvalidView,
    EmptyRoot,
    NodeReference,
    NodeCycle,
    UnreachableNode,
    PaneReference,
    DuplicatePane,
    MissingPane,
    ActivePane,
}

impl fmt::Display for SessionInvalid {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::NoTabs => "session has no document tabs",
            Self::TooManyTabs => "session exceeds the document tab limit",
            Self::ActiveTab => "session active tab is invalid",
            Self::ScratchPosition => "session scratch tab position is invalid",
            Self::DuplicatePath => "session contains a duplicate document path",
            Self::RelativePath => "session contains a relative path",
            Self::PathTooLong => "session path exceeds its byte limit",
            Self::PathBudget => "session paths exceed their retained byte budget",
            Self::InvalidView => "session document view is invalid",
            Self::EmptyRoot => "session pane root is empty",
            Self::NodeReference => "session pane node reference is invalid",
            Self::NodeCycle => "session pane graph contains a cycle or alias",
            Self::UnreachableNode => "session pane graph retains unreachable nodes",
            Self::PaneReference => "session pane reference is invalid",
            Self::DuplicatePane => "session pane is referenced more than once",
            Self::MissingPane => "session pane is occupied but unreachable",
            Self::ActivePane => "session active pane is invalid",
        };
        formatter.write_str(message)
    }
}

#[cfg(any(test, all(target_os = "macos", target_arch = "aarch64")))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SessionCorrupt {
    Header,
    Version(u16),
    Length,
    Checksum,
    Truncated,
    Tag,
    TrailingBytes,
    Invalid(SessionInvalid),
}

#[cfg(any(test, all(target_os = "macos", target_arch = "aarch64")))]
impl fmt::Display for SessionCorrupt {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Header => formatter.write_str("session header is invalid"),
            Self::Version(version) => write!(formatter, "session version {version} is unsupported"),
            Self::Length => formatter.write_str("session byte length is invalid"),
            Self::Checksum => formatter.write_str("session checksum does not match"),
            Self::Truncated => formatter.write_str("session payload is truncated"),
            Self::Tag => formatter.write_str("session payload contains an unknown tag"),
            Self::TrailingBytes => formatter.write_str("session payload contains trailing bytes"),
            Self::Invalid(error) => write!(formatter, "session state is invalid: {error}"),
        }
    }
}

#[cfg(any(test, all(target_os = "macos", target_arch = "aarch64")))]
impl std::error::Error for SessionCorrupt {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SessionError {
    #[cfg(any(test, all(target_os = "macos", target_arch = "aarch64")))]
    MissingHome,
    AllocationFailed,
    Invalid(SessionInvalid),
    #[cfg(any(test, all(target_os = "macos", target_arch = "aarch64")))]
    Corrupt(SessionCorrupt),
    Io {
        operation: &'static str,
        kind: io::ErrorKind,
    },
}

impl fmt::Display for SessionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            #[cfg(any(test, all(target_os = "macos", target_arch = "aarch64")))]
            Self::MissingHome => formatter.write_str("session home directory is unavailable"),
            Self::AllocationFailed => formatter.write_str("session allocation failed"),
            Self::Invalid(error) => write!(formatter, "cannot save invalid session: {error}"),
            #[cfg(any(test, all(target_os = "macos", target_arch = "aarch64")))]
            Self::Corrupt(error) => write!(formatter, "cannot restore corrupt session: {error}"),
            Self::Io { operation, kind } => {
                write!(formatter, "session {operation} failed with {kind:?}")
            }
        }
    }
}

impl std::error::Error for SessionError {}

#[cfg(any(test, all(target_os = "macos", target_arch = "aarch64")))]
pub(crate) fn default_path() -> Result<PathBuf, SessionError> {
    default_path_from_home(std::env::var_os("HOME"))
}

#[cfg(any(test, all(target_os = "macos", target_arch = "aarch64")))]
fn default_path_from_home(home: Option<OsString>) -> Result<PathBuf, SessionError> {
    let home = home.filter(|value| !value.is_empty());
    home.map(PathBuf::from)
        .map(|path| {
            path.join("Library")
                .join("Application Support")
                .join("Alpine Studio")
                .join("session-v1.bin")
        })
        .ok_or(SessionError::MissingHome)
}

pub(crate) fn save(path: &Path, state: &SessionState) -> Result<(), SessionError> {
    validate(state).map_err(SessionError::Invalid)?;
    let encoded = encode(state)?;
    let parent = path.parent().ok_or(SessionError::Io {
        operation: "resolve-parent",
        kind: io::ErrorKind::InvalidInput,
    })?;
    map_io(fs::create_dir_all(parent), "create-directory")?;
    atomic_replace(path, &encoded)
}

#[cfg(any(test, all(target_os = "macos", target_arch = "aarch64")))]
pub(crate) fn load(path: &Path) -> Result<SessionState, SessionError> {
    let file = map_io(File::open(path), "open")?;
    let encoded_bytes = map_io(file.metadata(), "metadata")?.len();
    if encoded_bytes > u64::try_from(MAX_SESSION_BYTES).unwrap_or(u64::MAX) {
        return Err(SessionError::Corrupt(SessionCorrupt::Length));
    }
    let encoded_bytes = usize::try_from(encoded_bytes)
        .map_err(|_| SessionError::Corrupt(SessionCorrupt::Length))?;
    let bytes = read_bounded(file, encoded_bytes)?;
    decode(&bytes).map_err(SessionError::Corrupt)
}

#[cfg(any(test, all(target_os = "macos", target_arch = "aarch64")))]
fn read_bounded(reader: impl Read, encoded_bytes: usize) -> Result<Vec<u8>, SessionError> {
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(encoded_bytes)
        .map_err(|_| SessionError::AllocationFailed)?;
    map_io(reader.take(MAX_READ_BYTES).read_to_end(&mut bytes), "read")?;
    if bytes.len() > MAX_SESSION_BYTES {
        return Err(SessionError::Corrupt(SessionCorrupt::Length));
    }
    Ok(bytes)
}

pub(crate) fn validate(state: &SessionState) -> Result<(), SessionInvalid> {
    if state.tabs.is_empty() {
        return Err(SessionInvalid::NoTabs);
    }
    if state.tabs.len() > SESSION_TAB_CAPACITY {
        return Err(SessionInvalid::TooManyTabs);
    }
    if usize::from(state.active_tab) >= state.tabs.len() {
        return Err(SessionInvalid::ActiveTab);
    }
    let mut retained_path_bytes = validate_optional_path(state.workspace.as_deref())?;
    for (index, tab) in state.tabs.iter().enumerate() {
        if tab.path.is_none() && index != 0 {
            return Err(SessionInvalid::ScratchPosition);
        }
        if let Some(path) = tab.path.as_deref() {
            if !path.is_absolute() {
                return Err(SessionInvalid::RelativePath);
            }
            if state.tabs[..index]
                .iter()
                .any(|previous| previous.path.as_deref() == Some(path))
            {
                return Err(SessionInvalid::DuplicatePath);
            }
        }
        retained_path_bytes = retained_path_bytes
            .checked_add(validate_optional_path(tab.path.as_deref())?)
            .ok_or(SessionInvalid::PathBudget)?;
        validate_view(tab.view)?;
    }
    if retained_path_bytes > MAX_RETAINED_PATH_BYTES {
        return Err(SessionInvalid::PathBudget);
    }
    validate_panes(&state.panes, state.tabs.len())?;
    let active_pane = state.panes.panes[usize::from(state.panes.active_pane)]
        .ok_or(SessionInvalid::ActivePane)?;
    if active_pane.tab != state.active_tab {
        return Err(SessionInvalid::ActivePane);
    }
    Ok(())
}

fn validate_optional_path(path: Option<&Path>) -> Result<usize, SessionInvalid> {
    let Some(path) = path else {
        return Ok(0);
    };
    if !path.is_absolute() {
        return Err(SessionInvalid::RelativePath);
    }
    let bytes = os_bytes(path.as_os_str());
    if bytes.is_empty() || bytes.len() > MAX_PATH_BYTES {
        return Err(SessionInvalid::PathTooLong);
    }
    Ok(bytes.len())
}

fn validate_view(view: DocumentViewState) -> Result<(), SessionInvalid> {
    if !view.scroll_y.is_finite() || view.scroll_y < 0.0 {
        return Err(SessionInvalid::InvalidView);
    }
    Ok(())
}

pub(crate) fn validate_panes(panes: &SessionPanes, tab_count: usize) -> Result<(), SessionInvalid> {
    if panes.nodes[0] == SessionNode::Empty {
        return Err(SessionInvalid::EmptyRoot);
    }
    let active = usize::from(panes.active_pane);
    if active >= SESSION_PANE_CAPACITY || panes.panes[active].is_none() {
        return Err(SessionInvalid::ActivePane);
    }
    for pane in panes.panes.iter().flatten() {
        if usize::from(pane.tab) >= tab_count {
            return Err(SessionInvalid::PaneReference);
        }
        validate_view(pane.view)?;
    }
    let mut visited_nodes = [false; SESSION_NODE_CAPACITY];
    let mut visited_panes = [false; SESSION_PANE_CAPACITY];
    visit_node(0, panes, &mut visited_nodes, &mut visited_panes)?;
    for (index, node) in panes.nodes.iter().enumerate() {
        if *node != SessionNode::Empty && !visited_nodes[index] {
            return Err(SessionInvalid::UnreachableNode);
        }
    }
    for (index, pane) in panes.panes.iter().enumerate() {
        if pane.is_some() != visited_panes[index] {
            return Err(SessionInvalid::MissingPane);
        }
    }
    Ok(())
}

fn visit_node(
    index: usize,
    panes: &SessionPanes,
    visited_nodes: &mut [bool; SESSION_NODE_CAPACITY],
    visited_panes: &mut [bool; SESSION_PANE_CAPACITY],
) -> Result<(), SessionInvalid> {
    if index >= SESSION_NODE_CAPACITY {
        return Err(SessionInvalid::NodeReference);
    }
    if visited_nodes[index] {
        return Err(SessionInvalid::NodeCycle);
    }
    visited_nodes[index] = true;
    match panes.nodes[index] {
        SessionNode::Empty => Err(SessionInvalid::NodeReference),
        SessionNode::Leaf { pane } => {
            let pane = usize::from(pane);
            if pane >= SESSION_PANE_CAPACITY || panes.panes[pane].is_none() {
                return Err(SessionInvalid::PaneReference);
            }
            if visited_panes[pane] {
                return Err(SessionInvalid::DuplicatePane);
            }
            visited_panes[pane] = true;
            Ok(())
        }
        SessionNode::Split { first, second, .. } => {
            let first = usize::from(first);
            let second = usize::from(second);
            if first == second {
                return Err(SessionInvalid::NodeReference);
            }
            visit_node(first, panes, visited_nodes, visited_panes)?;
            visit_node(second, panes, visited_nodes, visited_panes)
        }
    }
}

fn encode(state: &SessionState) -> Result<Vec<u8>, SessionError> {
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(MAX_SESSION_BYTES)
        .map_err(|_| SessionError::AllocationFailed)?;
    bytes.resize(HEADER_BYTES, 0);
    put_path(&mut bytes, state.workspace.as_deref())?;
    put_u8(
        &mut bytes,
        u8::try_from(state.tabs.len())
            .map_err(|_| SessionError::Invalid(SessionInvalid::TooManyTabs))?,
    );
    for tab in &state.tabs {
        put_path(&mut bytes, tab.path.as_deref())?;
        put_view(&mut bytes, tab.view)?;
    }
    put_u8(&mut bytes, state.active_tab);
    for node in state.panes.nodes {
        match node {
            SessionNode::Empty => put_u8(&mut bytes, 0),
            SessionNode::Leaf { pane } => {
                put_u8(&mut bytes, 1);
                put_u8(&mut bytes, pane);
            }
            SessionNode::Split {
                axis,
                first,
                second,
            } => {
                put_u8(&mut bytes, 2);
                put_u8(
                    &mut bytes,
                    match axis {
                        SessionAxis::Columns => 0,
                        SessionAxis::Rows => 1,
                    },
                );
                put_u8(&mut bytes, first);
                put_u8(&mut bytes, second);
            }
        }
    }
    for pane in state.panes.panes {
        if let Some(pane) = pane {
            put_u8(&mut bytes, 1);
            put_u8(&mut bytes, pane.tab);
            put_view(&mut bytes, pane.view)?;
        } else {
            put_u8(&mut bytes, 0);
        }
    }
    put_u8(&mut bytes, state.panes.active_pane);
    ensure_encoded_size(bytes.len())?;
    let payload_len = bytes
        .len()
        .checked_sub(HEADER_BYTES)
        .and_then(|length| u32::try_from(length).ok())
        .ok_or(SessionError::Invalid(SessionInvalid::PathBudget))?;
    let checksum = crc32(&bytes[HEADER_BYTES..]);
    bytes[..8].copy_from_slice(MAGIC);
    bytes[8..10].copy_from_slice(&VERSION.to_le_bytes());
    bytes[10..14].copy_from_slice(&payload_len.to_le_bytes());
    bytes[14..18].copy_from_slice(&checksum.to_le_bytes());
    Ok(bytes)
}

fn encoded_size_exceeds_limit(length: usize) -> bool {
    length > MAX_SESSION_BYTES
}

fn ensure_encoded_size(length: usize) -> Result<(), SessionError> {
    if encoded_size_exceeds_limit(length) {
        Err(SessionError::Invalid(SessionInvalid::PathBudget))
    } else {
        Ok(())
    }
}

#[cfg(any(test, all(target_os = "macos", target_arch = "aarch64")))]
fn header_is_truncated(length: usize) -> bool {
    length < HEADER_BYTES
}

#[cfg(any(test, all(target_os = "macos", target_arch = "aarch64")))]
fn decode_axis(tag: u8) -> Result<SessionAxis, SessionCorrupt> {
    match tag {
        0 => Ok(SessionAxis::Columns),
        1 => Ok(SessionAxis::Rows),
        _ => Err(SessionCorrupt::Tag),
    }
}

#[cfg(any(test, all(target_os = "macos", target_arch = "aarch64")))]
fn decode(bytes: &[u8]) -> Result<SessionState, SessionCorrupt> {
    if header_is_truncated(bytes.len()) || bytes.get(..8) != Some(MAGIC) {
        return Err(SessionCorrupt::Header);
    }
    let version = u16::from_le_bytes([bytes[8], bytes[9]]);
    if version != VERSION {
        return Err(SessionCorrupt::Version(version));
    }
    let payload_len = u32::from_le_bytes([bytes[10], bytes[11], bytes[12], bytes[13]]) as usize;
    if payload_len + HEADER_BYTES != bytes.len() {
        return Err(SessionCorrupt::Length);
    }
    let expected = u32::from_le_bytes([bytes[14], bytes[15], bytes[16], bytes[17]]);
    let payload = &bytes[HEADER_BYTES..];
    if crc32(payload) != expected {
        return Err(SessionCorrupt::Checksum);
    }
    let mut reader = Reader::new(payload);
    let workspace = reader.path()?;
    let tab_count = usize::from(reader.u8()?);
    let mut tabs = Vec::new();
    tabs.try_reserve_exact(tab_count)
        .map_err(|_| SessionCorrupt::Length)?;
    for _ in 0..tab_count {
        tabs.push(SessionTab {
            path: reader.path()?,
            view: reader.view()?,
        });
    }
    let active_tab = reader.u8()?;
    let mut nodes = [SessionNode::Empty; SESSION_NODE_CAPACITY];
    for node in &mut nodes {
        *node = match reader.u8()? {
            0 => SessionNode::Empty,
            1 => SessionNode::Leaf { pane: reader.u8()? },
            2 => {
                let axis = decode_axis(reader.u8()?)?;
                SessionNode::Split {
                    axis,
                    first: reader.u8()?,
                    second: reader.u8()?,
                }
            }
            _ => return Err(SessionCorrupt::Tag),
        };
    }
    let mut panes = [None; SESSION_PANE_CAPACITY];
    for pane in &mut panes {
        *pane = match reader.u8()? {
            0 => None,
            1 => Some(SessionPane {
                tab: reader.u8()?,
                view: reader.view()?,
            }),
            _ => return Err(SessionCorrupt::Tag),
        };
    }
    let state = SessionState {
        workspace,
        tabs,
        active_tab,
        panes: SessionPanes {
            nodes,
            panes,
            active_pane: reader.u8()?,
        },
    };
    if !reader.is_empty() {
        return Err(SessionCorrupt::TrailingBytes);
    }
    validate(&state).map_err(SessionCorrupt::Invalid)?;
    Ok(state)
}

fn put_path(bytes: &mut Vec<u8>, path: Option<&Path>) -> Result<(), SessionError> {
    let Some(path) = path else {
        put_u8(bytes, 0);
        return Ok(());
    };
    let path = os_bytes(path.as_os_str());
    let length = u16::try_from(path.len())
        .map_err(|_| SessionError::Invalid(SessionInvalid::PathTooLong))?;
    put_u8(bytes, 1);
    bytes.extend_from_slice(&length.to_le_bytes());
    bytes.extend_from_slice(path);
    Ok(())
}

fn put_view(bytes: &mut Vec<u8>, view: DocumentViewState) -> Result<(), SessionError> {
    let anchor = u64::try_from(view.selection.anchor().get())
        .map_err(|_| SessionError::Invalid(SessionInvalid::InvalidView))?;
    let head = u64::try_from(view.selection.head().get())
        .map_err(|_| SessionError::Invalid(SessionInvalid::InvalidView))?;
    bytes.extend_from_slice(&anchor.to_le_bytes());
    bytes.extend_from_slice(&head.to_le_bytes());
    bytes.extend_from_slice(&view.scroll_y.to_bits().to_le_bytes());
    Ok(())
}

fn put_u8(bytes: &mut Vec<u8>, value: u8) {
    bytes.push(value);
}

#[cfg(any(test, all(target_os = "macos", target_arch = "aarch64")))]
struct Reader<'a> {
    bytes: &'a [u8],
    cursor: usize,
}

#[cfg(any(test, all(target_os = "macos", target_arch = "aarch64")))]
impl<'a> Reader<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, cursor: 0 }
    }

    fn u8(&mut self) -> Result<u8, SessionCorrupt> {
        let value = *self
            .bytes
            .get(self.cursor)
            .ok_or(SessionCorrupt::Truncated)?;
        self.cursor += 1;
        Ok(value)
    }

    fn u16(&mut self) -> Result<u16, SessionCorrupt> {
        let bytes = self.take(2)?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }

    fn u32(&mut self) -> Result<u32, SessionCorrupt> {
        let bytes = self.take(4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn u64(&mut self) -> Result<u64, SessionCorrupt> {
        let bytes = self.take(8)?;
        Ok(u64::from_le_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ]))
    }

    fn path(&mut self) -> Result<Option<PathBuf>, SessionCorrupt> {
        match self.u8()? {
            0 => Ok(None),
            1 => {
                let length = usize::from(self.u16()?);
                if length == 0 || length > MAX_PATH_BYTES {
                    return Err(SessionCorrupt::Invalid(SessionInvalid::PathTooLong));
                }
                let bytes = self.take(length)?;
                Ok(Some(PathBuf::from(os_string(bytes))))
            }
            _ => Err(SessionCorrupt::Tag),
        }
    }

    fn view(&mut self) -> Result<DocumentViewState, SessionCorrupt> {
        let anchor = usize::try_from(self.u64()?)
            .map_err(|_| SessionCorrupt::Invalid(SessionInvalid::InvalidView))?;
        let head = usize::try_from(self.u64()?)
            .map_err(|_| SessionCorrupt::Invalid(SessionInvalid::InvalidView))?;
        let view = DocumentViewState {
            selection: Selection::new(ByteOffset::new(anchor), ByteOffset::new(head)),
            scroll_y: f32::from_bits(self.u32()?),
        };
        validate_view(view).map_err(SessionCorrupt::Invalid)?;
        Ok(view)
    }

    fn take(&mut self, length: usize) -> Result<&'a [u8], SessionCorrupt> {
        let end = self
            .cursor
            .checked_add(length)
            .ok_or(SessionCorrupt::Truncated)?;
        let bytes = self
            .bytes
            .get(self.cursor..end)
            .ok_or(SessionCorrupt::Truncated)?;
        self.cursor = end;
        Ok(bytes)
    }

    const fn is_empty(&self) -> bool {
        self.cursor == self.bytes.len()
    }
}

fn atomic_replace(path: &Path, bytes: &[u8]) -> Result<(), SessionError> {
    let mut write = |file: &mut File, value: &[u8]| file.write_all(value);
    atomic_replace_with(path, bytes, &mut write)
}

type SessionWriter<'a> = dyn FnMut(&mut File, &[u8]) -> io::Result<()> + 'a;

fn atomic_replace_with(
    path: &Path,
    bytes: &[u8],
    write: &mut SessionWriter<'_>,
) -> Result<(), SessionError> {
    let mut next_sequence = || TEMPORARY_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    atomic_replace_with_sequence(path, bytes, write, &mut next_sequence)
}

type SequenceSource<'a> = dyn FnMut() -> u64 + 'a;

fn atomic_replace_with_sequence(
    path: &Path,
    bytes: &[u8],
    write: &mut SessionWriter<'_>,
    next_sequence: &mut SequenceSource<'_>,
) -> Result<(), SessionError> {
    let file_name = path.file_name().ok_or(SessionError::Io {
        operation: "temporary-name",
        kind: io::ErrorKind::InvalidInput,
    })?;
    let parent = path.parent().ok_or(SessionError::Io {
        operation: "resolve-parent",
        kind: io::ErrorKind::InvalidInput,
    })?;
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
    let (temporary, mut file) = created.ok_or(SessionError::Io {
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

fn temporary_path(parent: &Path, file_name: &OsStr, sequence: u64) -> PathBuf {
    let mut name = OsString::from(".");
    name.push(file_name);
    name.push(format!(".alpine-session-{}-{sequence}", std::process::id()));
    parent.join(name)
}

#[cfg(unix)]
fn sync_parent_directory(parent: &Path) -> Result<(), SessionError> {
    let directory = map_io(File::open(parent), "open-directory")?;
    map_io(directory.sync_all(), "sync-directory")
}

#[cfg(not(unix))]
fn sync_parent_directory(_parent: &Path) -> Result<(), SessionError> {
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

fn map_io<T>(result: io::Result<T>, operation: &'static str) -> Result<T, SessionError> {
    result.map_err(|error| io_error(operation, &error))
}

fn io_error(operation: &'static str, error: &io::Error) -> SessionError {
    SessionError::Io {
        operation,
        kind: error.kind(),
    }
}

#[cfg(unix)]
fn os_bytes(value: &OsStr) -> &[u8] {
    use std::os::unix::ffi::OsStrExt;
    value.as_bytes()
}

#[cfg(not(unix))]
#[cfg_attr(test, mutants::skip)] // The disabled adapter is qualified by Windows CI.
fn os_bytes(value: &OsStr) -> &[u8] {
    value.as_encoded_bytes()
}

#[cfg(all(unix, any(test, all(target_os = "macos", target_arch = "aarch64"))))]
fn os_string(value: &[u8]) -> OsString {
    use std::os::unix::ffi::OsStringExt;
    OsString::from_vec(value.to_vec())
}

#[cfg(all(
    not(unix),
    any(test, all(target_os = "macos", target_arch = "aarch64"))
))]
#[cfg_attr(test, mutants::skip)] // The disabled adapter is qualified by Windows CI.
fn os_string(value: &[u8]) -> OsString {
    String::from_utf8_lossy(value).into_owned().into()
}

#[cfg(test)]
mod mutation_boundary_tests {
    use super::*;

    #[test]
    fn encoded_size_and_header_boundaries_are_exact() {
        assert!(!encoded_size_exceeds_limit(MAX_SESSION_BYTES));
        assert!(encoded_size_exceeds_limit(MAX_SESSION_BYTES + 1));
        assert_eq!(ensure_encoded_size(MAX_SESSION_BYTES), Ok(()));
        assert_eq!(
            ensure_encoded_size(MAX_SESSION_BYTES + 1),
            Err(SessionError::Invalid(SessionInvalid::PathBudget))
        );
        assert!(header_is_truncated(HEADER_BYTES - 1));
        assert!(!header_is_truncated(HEADER_BYTES));
    }

    #[test]
    fn bounded_reader_accepts_the_limit_and_rejects_one_extra_byte() {
        let accepted = vec![0_u8; MAX_SESSION_BYTES];
        assert_eq!(
            read_bounded(accepted.as_slice(), MAX_SESSION_BYTES),
            Ok(accepted)
        );
        let oversized = vec![0_u8; MAX_SESSION_BYTES + 1];
        assert_eq!(
            read_bounded(oversized.as_slice(), MAX_SESSION_BYTES),
            Err(SessionError::Corrupt(SessionCorrupt::Length))
        );
    }

    #[test]
    fn every_axis_tag_has_an_independent_contract() {
        assert!(matches!(decode_axis(0), Ok(SessionAxis::Columns)));
        assert!(matches!(decode_axis(1), Ok(SessionAxis::Rows)));
        assert!(matches!(decode_axis(2), Err(SessionCorrupt::Tag)));
    }

    #[cfg(unix)]
    #[test]
    fn directory_sync_reports_an_unopenable_parent() {
        let missing = std::env::temp_dir().join(format!(
            "alpine-missing-session-parent-{}",
            std::process::id()
        ));
        assert!(sync_parent_directory(&missing).is_err());
    }

    #[test]
    fn os_string_codec_preserves_independent_known_bytes() {
        let value = OsString::from("alpine-session");
        assert_eq!(os_bytes(&value), b"alpine-session");
        assert_eq!(os_string(b"alpine-session"), value);
    }

    #[cfg(unix)]
    #[test]
    fn os_string_codec_preserves_non_utf8_bytes() {
        use std::os::unix::ffi::OsStringExt;

        let value = OsString::from_vec(vec![b'a', 0x80, b'z']);
        assert_eq!(os_string(os_bytes(&value)), value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error;

    fn view(offset: usize, scroll_y: f32) -> DocumentViewState {
        DocumentViewState {
            selection: Selection::caret(ByteOffset::new(offset)),
            scroll_y,
        }
    }

    fn absolute_root() -> PathBuf {
        let root = std::env::temp_dir().join("alpine-session-model");
        assert!(root.is_absolute());
        root
    }

    fn state() -> SessionState {
        let root = absolute_root();
        SessionState {
            workspace: Some(root.clone()),
            tabs: vec![
                SessionTab {
                    path: None,
                    view: view(0, 0.0),
                },
                SessionTab {
                    path: Some(root.join("src").join("main.rs")),
                    view: view(12, 24.0),
                },
            ],
            active_tab: 1,
            panes: SessionPanes {
                nodes: [
                    SessionNode::Split {
                        axis: SessionAxis::Columns,
                        first: 1,
                        second: 2,
                    },
                    SessionNode::Leaf { pane: 0 },
                    SessionNode::Leaf { pane: 1 },
                    SessionNode::Empty,
                    SessionNode::Empty,
                    SessionNode::Empty,
                    SessionNode::Empty,
                ],
                panes: [
                    Some(SessionPane {
                        tab: 0,
                        view: view(0, 5.0),
                    }),
                    Some(SessionPane {
                        tab: 1,
                        view: view(12, 24.0),
                    }),
                    None,
                    None,
                ],
                active_pane: 1,
            },
        }
    }

    #[test]
    fn codec_round_trips_exact_paths_tabs_panes_and_views() -> Result<(), Box<dyn Error>> {
        let state = state();
        let encoded = encode(&state)?;
        assert_eq!(&encoded[..8], MAGIC);
        assert_eq!(u16::from_le_bytes([encoded[8], encoded[9]]), VERSION);
        let payload_len = u32::from_le_bytes([encoded[10], encoded[11], encoded[12], encoded[13]]);
        assert_eq!(usize::try_from(payload_len)? + HEADER_BYTES, encoded.len());
        let checksum = u32::from_le_bytes([encoded[14], encoded[15], encoded[16], encoded[17]]);
        assert_eq!(checksum, crc32(&encoded[HEADER_BYTES..]));
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
        assert_eq!(decode(&encoded)?, state);

        let mut rows = state.clone();
        rows.panes.nodes[0] = SessionNode::Split {
            axis: SessionAxis::Rows,
            first: 1,
            second: 2,
        };
        assert_eq!(decode(&encode(&rows)?)?, rows);
        Ok(())
    }

    #[test]
    fn constants_default_path_and_error_contracts_are_exact() -> Result<(), Box<dyn Error>> {
        assert_eq!(SESSION_NODE_CAPACITY, 7);
        assert_eq!(SESSION_PANE_CAPACITY, 4);
        assert_eq!(SESSION_TAB_CAPACITY, 32);
        assert_eq!(MAX_PATH_BYTES, 4_096);
        assert_eq!(MAX_RETAINED_PATH_BYTES, 65_536);
        assert_eq!(MAX_SESSION_BYTES, 131_072);
        assert_eq!(MAX_READ_BYTES, 131_073);
        assert_eq!(HEADER_BYTES, 18);

        assert_eq!(default_path_from_home(None), Err(SessionError::MissingHome));
        assert_eq!(
            default_path_from_home(Some(OsString::new())),
            Err(SessionError::MissingHome)
        );
        let home = absolute_root();
        assert_eq!(
            default_path_from_home(Some(home.clone().into_os_string()))?,
            home.join("Library")
                .join("Application Support")
                .join("Alpine Studio")
                .join("session-v1.bin")
        );
        assert_eq!(
            default_path(),
            default_path_from_home(std::env::var_os("HOME"))
        );

        for error in [
            SessionInvalid::NoTabs,
            SessionInvalid::TooManyTabs,
            SessionInvalid::ActiveTab,
            SessionInvalid::ScratchPosition,
            SessionInvalid::DuplicatePath,
            SessionInvalid::RelativePath,
            SessionInvalid::PathTooLong,
            SessionInvalid::PathBudget,
            SessionInvalid::InvalidView,
            SessionInvalid::EmptyRoot,
            SessionInvalid::NodeReference,
            SessionInvalid::NodeCycle,
            SessionInvalid::UnreachableNode,
            SessionInvalid::PaneReference,
            SessionInvalid::DuplicatePane,
            SessionInvalid::MissingPane,
            SessionInvalid::ActivePane,
        ] {
            assert!(!error.to_string().is_empty());
        }
        for error in [
            SessionCorrupt::Header,
            SessionCorrupt::Version(7),
            SessionCorrupt::Length,
            SessionCorrupt::Checksum,
            SessionCorrupt::Truncated,
            SessionCorrupt::Tag,
            SessionCorrupt::TrailingBytes,
            SessionCorrupt::Invalid(SessionInvalid::NoTabs),
        ] {
            assert!(!error.to_string().is_empty());
            assert!(Error::source(&error).is_none());
        }
        for error in [
            SessionError::MissingHome,
            SessionError::AllocationFailed,
            SessionError::Invalid(SessionInvalid::NoTabs),
            SessionError::Corrupt(SessionCorrupt::Header),
            SessionError::Io {
                operation: "read",
                kind: io::ErrorKind::NotFound,
            },
        ] {
            assert!(!error.to_string().is_empty());
            assert!(Error::source(&error).is_none());
        }
        Ok(())
    }

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "independent tab, path, view, and graph guards share one canonical fixture"
    )]
    fn tab_path_view_and_graph_validation_boundaries_are_independent() {
        let mut invalid = state();
        invalid.tabs.clear();
        assert_eq!(validate(&invalid), Err(SessionInvalid::NoTabs));

        let mut maximum = state();
        maximum.tabs = (0_u8..32)
            .map(|index| SessionTab {
                path: Some(absolute_root().join(format!("{index}.rs"))),
                view: view(usize::from(index), f32::from(index)),
            })
            .collect();
        maximum.active_tab = 0;
        maximum.panes.panes[0] = Some(SessionPane {
            tab: 0,
            view: view(0, 0.0),
        });
        maximum.panes.nodes = [
            SessionNode::Leaf { pane: 0 },
            SessionNode::Empty,
            SessionNode::Empty,
            SessionNode::Empty,
            SessionNode::Empty,
            SessionNode::Empty,
            SessionNode::Empty,
        ];
        maximum.panes.panes[1] = None;
        maximum.panes.active_pane = 0;
        assert_eq!(validate(&maximum), Ok(()));
        maximum.tabs.push(SessionTab {
            path: Some(absolute_root().join("overflow.rs")),
            view: view(0, 0.0),
        });
        assert_eq!(validate(&maximum), Err(SessionInvalid::TooManyTabs));

        let mut invalid = state();
        invalid.active_tab = 2;
        assert_eq!(validate(&invalid), Err(SessionInvalid::ActiveTab));

        let mut invalid = state();
        invalid.tabs[1].path = None;
        assert_eq!(validate(&invalid), Err(SessionInvalid::ScratchPosition));
        let mut invalid = state();
        invalid.tabs.insert(
            0,
            SessionTab {
                path: None,
                view: view(0, 0.0),
            },
        );
        assert_eq!(validate(&invalid), Err(SessionInvalid::ScratchPosition));

        let mut invalid = state();
        invalid.workspace = Some(PathBuf::from("relative"));
        assert_eq!(validate(&invalid), Err(SessionInvalid::RelativePath));
        let mut invalid = state();
        invalid.tabs[1].path = Some(PathBuf::from("relative.rs"));
        assert_eq!(validate(&invalid), Err(SessionInvalid::RelativePath));
        let mut invalid = state();
        invalid.tabs.push(invalid.tabs[1].clone());
        assert_eq!(validate(&invalid), Err(SessionInvalid::DuplicatePath));

        let mut invalid = state();
        invalid.tabs[0].view.scroll_y = -1.0;
        assert_eq!(validate(&invalid), Err(SessionInvalid::InvalidView));
        let mut invalid = state();
        invalid.panes.panes[0] = Some(SessionPane {
            tab: 0,
            view: view(0, f32::INFINITY),
        });
        assert_eq!(validate(&invalid), Err(SessionInvalid::InvalidView));

        let mut invalid = state();
        invalid.panes.nodes[0] = SessionNode::Empty;
        assert_eq!(validate(&invalid), Err(SessionInvalid::EmptyRoot));
        let mut invalid = state();
        invalid.panes.active_pane = 4;
        assert_eq!(validate(&invalid), Err(SessionInvalid::ActivePane));
        let mut invalid = state();
        invalid.panes.active_pane = 2;
        assert_eq!(validate(&invalid), Err(SessionInvalid::ActivePane));
        let mut invalid = state();
        invalid.panes.panes[0] = Some(SessionPane {
            tab: 2,
            view: view(0, 5.0),
        });
        assert_eq!(validate(&invalid), Err(SessionInvalid::PaneReference));
        let mut invalid = state();
        invalid.panes.nodes[0] = SessionNode::Split {
            axis: SessionAxis::Rows,
            first: 7,
            second: 2,
        };
        assert_eq!(validate(&invalid), Err(SessionInvalid::NodeReference));
        let mut invalid = state();
        invalid.panes.nodes[0] = SessionNode::Split {
            axis: SessionAxis::Rows,
            first: 3,
            second: 2,
        };
        assert_eq!(validate(&invalid), Err(SessionInvalid::NodeReference));
        let mut invalid = state();
        invalid.panes.nodes[0] = SessionNode::Split {
            axis: SessionAxis::Rows,
            first: 1,
            second: 1,
        };
        assert_eq!(validate(&invalid), Err(SessionInvalid::NodeReference));
        let mut invalid = state();
        invalid.panes.nodes[1] = SessionNode::Leaf { pane: 4 };
        assert_eq!(validate(&invalid), Err(SessionInvalid::PaneReference));
        let mut invalid = state();
        invalid.panes.panes[0] = None;
        assert_eq!(validate(&invalid), Err(SessionInvalid::PaneReference));
        let mut invalid = state();
        invalid.panes.panes[2] = Some(SessionPane {
            tab: 0,
            view: view(0, 0.0),
        });
        assert_eq!(validate(&invalid), Err(SessionInvalid::MissingPane));
        let mut invalid = state();
        invalid.panes.panes[1] = Some(SessionPane {
            tab: 0,
            view: view(12, 24.0),
        });
        assert_eq!(validate(&invalid), Err(SessionInvalid::ActivePane));
    }

    #[cfg(unix)]
    #[test]
    fn path_and_aggregate_byte_limits_are_exact() {
        use std::os::unix::ffi::OsStringExt;

        let path = |length: usize, suffix: u8| {
            let mut bytes = vec![b'a'; length];
            bytes[0] = b'/';
            bytes[length - 1] = suffix;
            PathBuf::from(OsString::from_vec(bytes))
        };
        assert_eq!(
            validate_optional_path(Some(&path(MAX_PATH_BYTES, b'z'))),
            Ok(MAX_PATH_BYTES)
        );
        assert_eq!(
            validate_optional_path(Some(&path(MAX_PATH_BYTES + 1, b'z'))),
            Err(SessionInvalid::PathTooLong)
        );

        let mut maximum = state();
        maximum.workspace = Some(path(MAX_PATH_BYTES, b'A'));
        maximum.tabs = (0_u8..15)
            .map(|index| SessionTab {
                path: Some(path(MAX_PATH_BYTES, b'B' + index)),
                view: view(0, 0.0),
            })
            .collect();
        maximum.active_tab = 0;
        maximum.panes = SessionPanes {
            nodes: [
                SessionNode::Leaf { pane: 0 },
                SessionNode::Empty,
                SessionNode::Empty,
                SessionNode::Empty,
                SessionNode::Empty,
                SessionNode::Empty,
                SessionNode::Empty,
            ],
            panes: [
                Some(SessionPane {
                    tab: 0,
                    view: view(0, 0.0),
                }),
                None,
                None,
                None,
            ],
            active_pane: 0,
        };
        assert_eq!(validate(&maximum), Ok(()));
        maximum.tabs.push(SessionTab {
            path: Some(path(MAX_PATH_BYTES, b'Q')),
            view: view(0, 0.0),
        });
        assert_eq!(validate(&maximum), Err(SessionInvalid::PathBudget));
    }

    #[test]
    fn corruption_version_length_checksum_and_tags_fail_closed() -> Result<(), Box<dyn Error>> {
        let encoded = encode(&state())?;
        let mut corrupt = encoded.clone();
        corrupt[0] ^= 0x5a;
        assert_eq!(decode(&corrupt), Err(SessionCorrupt::Header));

        let mut corrupt = encoded.clone();
        corrupt[8..10].copy_from_slice(&2_u16.to_le_bytes());
        assert_eq!(decode(&corrupt), Err(SessionCorrupt::Version(2)));

        let mut corrupt = encoded.clone();
        corrupt[encoded.len() - 1] ^= 0x5a;
        assert_eq!(decode(&corrupt), Err(SessionCorrupt::Checksum));

        assert!(matches!(
            decode(&encoded[..encoded.len() - 1]),
            Err(SessionCorrupt::Length)
        ));

        let mut corrupt = encoded.clone();
        let mut reader = Reader::new(&corrupt[HEADER_BYTES..]);
        let _ = reader.path()?;
        let tab_count = reader.u8()?;
        for _ in 0..tab_count {
            let _ = reader.path()?;
            let _ = reader.view()?;
        }
        let _ = reader.u8()?;
        let first_node = HEADER_BYTES + reader.cursor;
        corrupt[first_node] = u8::MAX;
        let checksum = crc32(&corrupt[HEADER_BYTES..]);
        corrupt[14..18].copy_from_slice(&checksum.to_le_bytes());
        assert_eq!(decode(&corrupt), Err(SessionCorrupt::Tag));

        let mut corrupt = encoded.clone();
        let first_pane = first_node + 12;
        corrupt[first_pane] = u8::MAX;
        let checksum = crc32(&corrupt[HEADER_BYTES..]);
        corrupt[14..18].copy_from_slice(&checksum.to_le_bytes());
        assert_eq!(decode(&corrupt), Err(SessionCorrupt::Tag));

        let mut corrupt = encoded.clone();
        corrupt.push(0);
        let payload_len = u32::try_from(corrupt.len() - HEADER_BYTES)?;
        corrupt[10..14].copy_from_slice(&payload_len.to_le_bytes());
        let checksum = crc32(&corrupt[HEADER_BYTES..]);
        corrupt[14..18].copy_from_slice(&checksum.to_le_bytes());
        assert_eq!(decode(&corrupt), Err(SessionCorrupt::TrailingBytes));

        let mut corrupt = encoded.clone();
        corrupt[first_node + 1] = u8::MAX;
        let checksum = crc32(&corrupt[HEADER_BYTES..]);
        corrupt[14..18].copy_from_slice(&checksum.to_le_bytes());
        assert_eq!(decode(&corrupt), Err(SessionCorrupt::Tag));

        for length in 0..HEADER_BYTES {
            assert_eq!(decode(&encoded[..length]), Err(SessionCorrupt::Header));
        }
        Ok(())
    }

    #[test]
    fn reader_rejects_each_tag_length_truncation_and_view_boundary() -> Result<(), Box<dyn Error>> {
        let mut reader = Reader::new(&[2]);
        assert_eq!(reader.path(), Err(SessionCorrupt::Tag));

        let mut reader = Reader::new(&[1, 0, 0]);
        assert_eq!(
            reader.path(),
            Err(SessionCorrupt::Invalid(SessionInvalid::PathTooLong))
        );
        let oversized = u16::try_from(MAX_PATH_BYTES + 1)?.to_le_bytes();
        let mut bytes = vec![1, oversized[0], oversized[1]];
        let mut reader = Reader::new(&bytes);
        assert_eq!(
            reader.path(),
            Err(SessionCorrupt::Invalid(SessionInvalid::PathTooLong))
        );

        let maximum = u16::try_from(MAX_PATH_BYTES)?.to_le_bytes();
        bytes.clear();
        bytes.extend([1, maximum[0], maximum[1]]);
        bytes.resize(3 + MAX_PATH_BYTES, b'a');
        let mut reader = Reader::new(&bytes);
        assert!(reader.path()?.is_some());
        assert!(reader.is_empty());
        assert_eq!(reader.u8(), Err(SessionCorrupt::Truncated));

        let mut view_bytes = Vec::new();
        view_bytes.extend_from_slice(&0_u64.to_le_bytes());
        view_bytes.extend_from_slice(&0_u64.to_le_bytes());
        view_bytes.extend_from_slice(&f32::NAN.to_bits().to_le_bytes());
        let mut reader = Reader::new(&view_bytes);
        assert_eq!(
            reader.view(),
            Err(SessionCorrupt::Invalid(SessionInvalid::InvalidView))
        );

        let mut reader = Reader::new(&[1, 2, 3]);
        assert_eq!(reader.take(usize::MAX), Err(SessionCorrupt::Truncated));
        assert!(!reader.is_empty());
        Ok(())
    }

    #[test]
    fn pane_cycles_aliases_missing_nodes_and_invalid_views_are_rejected() {
        let mut invalid = state();
        invalid.panes.nodes[1] = SessionNode::Split {
            axis: SessionAxis::Rows,
            first: 0,
            second: 2,
        };
        assert_eq!(validate(&invalid), Err(SessionInvalid::NodeCycle));

        let mut invalid = state();
        invalid.panes.nodes[2] = SessionNode::Leaf { pane: 0 };
        assert_eq!(validate(&invalid), Err(SessionInvalid::DuplicatePane));

        let mut invalid = state();
        invalid.panes.nodes[6] = SessionNode::Leaf { pane: 2 };
        invalid.panes.panes[2] = Some(SessionPane {
            tab: 0,
            view: view(0, 0.0),
        });
        assert_eq!(validate(&invalid), Err(SessionInvalid::UnreachableNode));

        let mut invalid = state();
        invalid.tabs[0].view.scroll_y = f32::NAN;
        assert_eq!(validate(&invalid), Err(SessionInvalid::InvalidView));
    }

    #[cfg_attr(miri, ignore = "Miri isolation forbids filesystem syscalls")]
    #[test]
    fn atomic_replacement_round_trips_and_leaves_no_temporary_file() -> Result<(), Box<dyn Error>> {
        let root = std::env::temp_dir().join(format!(
            "alpine-session-test-{}-{}",
            std::process::id(),
            TEMPORARY_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root)?;
        let path = root.join("session.bin");
        save(&path, &state())?;
        assert_eq!(load(&path)?, state());
        assert_eq!(fs::read_dir(&root)?.count(), 1);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[cfg_attr(miri, ignore = "Miri isolation forbids filesystem syscalls")]
    #[test]
    fn failed_write_preserves_previous_session_and_cleans_temporary_file()
    -> Result<(), Box<dyn Error>> {
        let root = std::env::temp_dir().join(format!(
            "alpine-session-failure-{}-{}",
            std::process::id(),
            TEMPORARY_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root)?;
        let path = root.join("session.bin");
        let accepted = state();
        save(&path, &accepted)?;
        let replacement = encode(&accepted)?;
        let mut fail = |file: &mut File, bytes: &[u8]| {
            file.write_all(&bytes[..bytes.len() / 2])?;
            Err(io::Error::new(
                io::ErrorKind::StorageFull,
                "injected session write failure",
            ))
        };
        assert!(matches!(
            atomic_replace_with(&path, &replacement, &mut fail),
            Err(SessionError::Io {
                operation: "write",
                kind: io::ErrorKind::StorageFull,
            })
        ));
        assert_eq!(load(&path)?, accepted);
        assert_eq!(fs::read_dir(&root)?.count(), 1);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[cfg_attr(miri, ignore = "Miri isolation forbids filesystem syscalls")]
    #[test]
    fn load_size_and_temporary_collision_boundaries_are_exact() -> Result<(), Box<dyn Error>> {
        let root = std::env::temp_dir().join(format!(
            "alpine-session-boundaries-{}-{}",
            std::process::id(),
            TEMPORARY_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root)?;
        let oversized = root.join("oversized.bin");
        let file = File::create(&oversized)?;
        file.set_len(MAX_READ_BYTES)?;
        assert_eq!(
            load(&oversized),
            Err(SessionError::Corrupt(SessionCorrupt::Length))
        );
        file.set_len(u64::try_from(MAX_SESSION_BYTES)?)?;
        assert_eq!(
            load(&oversized),
            Err(SessionError::Corrupt(SessionCorrupt::Header))
        );

        let target = root.join("collision.bin");
        let file_name = target.file_name().ok_or("file name")?;
        let first = 70_001_u64;
        fs::write(temporary_path(&root, file_name, first), b"occupied")?;
        let sequences = [first, first + 1];
        let mut sequence_index = 0;
        let mut next = || {
            let value = sequences[sequence_index];
            sequence_index += 1;
            value
        };
        let mut writer = |file: &mut File, bytes: &[u8]| file.write_all(bytes);
        atomic_replace_with_sequence(&target, b"accepted", &mut writer, &mut next)?;
        assert_eq!(fs::read(&target)?, b"accepted");

        let start = 80_001_u64;
        for sequence in start..start + u64::try_from(TEMPORARY_ATTEMPTS)? {
            fs::write(temporary_path(&root, file_name, sequence), b"occupied")?;
        }
        let mut sequence = start;
        let mut next = || {
            let current = sequence;
            sequence += 1;
            current
        };
        assert_eq!(
            atomic_replace_with_sequence(&target, b"rejected", &mut writer, &mut next),
            Err(SessionError::Io {
                operation: "create-temporary",
                kind: io::ErrorKind::AlreadyExists,
            })
        );
        assert_eq!(fs::read(&target)?, b"accepted");

        let missing = root.join("missing").join("state.bin");
        let mut next = || 90_001;
        assert_eq!(
            atomic_replace_with_sequence(&missing, b"rejected", &mut writer, &mut next),
            Err(SessionError::Io {
                operation: "create-temporary",
                kind: io::ErrorKind::NotFound,
            })
        );
        fs::remove_dir_all(root)?;
        Ok(())
    }
}
