//! Crash-recoverable publication for bounded local Rust workspace edits.

use std::{
    error::Error,
    ffi::OsString,
    fmt,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use crate::{rust_diagnostics::WorkspaceEditIdentity, rust_workspace_edit::PreparedWorkspaceEdit};

const JOURNAL_MAGIC: &[u8; 8] = b"ALPWSE01";
const JOURNAL_VERSION: u32 = 1;
const MAX_JOURNAL_BYTES: usize = 512 * 1_024;
const MAX_PUBLICATION_FILES: usize = 32;
const MAX_PATH_BYTES: usize = 4_096;
const TEMPORARY_ATTEMPTS: usize = 16;
static PUBLICATION_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum JournalPhase {
    Preparing = 1,
    Prepared = 2,
    Committed = 3,
}

impl JournalPhase {
    fn decode(value: u8) -> Result<Self, WorkspaceEditPublicationError> {
        match value {
            1 => Ok(Self::Preparing),
            2 => Ok(Self::Prepared),
            3 => Ok(Self::Committed),
            _ => Err(WorkspaceEditPublicationError::CorruptJournal),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Journal {
    phase: JournalPhase,
    process_id: u32,
    sequence: u64,
    targets: Box<[PathBuf]>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PublicationEntry {
    target: PathBuf,
    stage: PathBuf,
    backup: PathBuf,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum JournalOpenDisposition {
    Retry,
    Fail(io::ErrorKind),
}

const fn journal_open_disposition(kind: io::ErrorKind) -> JournalOpenDisposition {
    if matches!(kind, io::ErrorKind::AlreadyExists) {
        JournalOpenDisposition::Retry
    } else {
        JournalOpenDisposition::Fail(kind)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct WorkspaceEditPublicationReport {
    pub(crate) files: usize,
    pub(crate) edits: usize,
    pub(crate) bytes_written: usize,
    pub(crate) cleanup_deferred: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum WorkspaceEditPublicationError {
    Empty,
    TooManyFiles,
    PathTooLong,
    InvalidPath,
    DuplicatePath,
    StaleFile,
    ArtifactCollision,
    JournalTooLarge,
    CorruptJournal,
    Io {
        operation: &'static str,
        kind: io::ErrorKind,
    },
    RollbackIncomplete {
        operation: &'static str,
        kind: io::ErrorKind,
    },
}

impl fmt::Display for WorkspaceEditPublicationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "Rust workspace edit publication failed: {self:?}"
        )
    }
}

impl Error for WorkspaceEditPublicationError {}

pub(crate) struct WorkspaceEditPublicationRequest {
    identity: WorkspaceEditIdentity,
    journal_path: PathBuf,
    prepared: PreparedWorkspaceEdit,
}

impl WorkspaceEditPublicationRequest {
    pub(crate) fn new(
        identity: WorkspaceEditIdentity,
        journal_path: PathBuf,
        prepared: PreparedWorkspaceEdit,
    ) -> Self {
        Self {
            identity,
            journal_path,
            prepared,
        }
    }

    pub(crate) fn execute(self) -> WorkspaceEditPublicationOutput {
        let result = publish(&self.journal_path, &self.prepared);
        WorkspaceEditPublicationOutput {
            identity: self.identity,
            prepared: self.prepared,
            result,
        }
    }
}

pub(crate) struct WorkspaceEditPublicationOutput {
    pub(crate) identity: WorkspaceEditIdentity,
    pub(crate) prepared: PreparedWorkspaceEdit,
    pub(crate) result: Result<WorkspaceEditPublicationReport, WorkspaceEditPublicationError>,
}

pub(crate) fn journal_path_for_session(
    session_path: &Path,
) -> Result<PathBuf, WorkspaceEditPublicationError> {
    let parent = session_path
        .parent()
        .ok_or(WorkspaceEditPublicationError::InvalidPath)?;
    Ok(parent.join("workspace-edit-v1.bin"))
}

pub(crate) fn recover_pending(journal_path: &Path) -> Result<bool, WorkspaceEditPublicationError> {
    let journal = match load_journal(journal_path) {
        Ok(journal) => journal,
        Err(WorkspaceEditPublicationError::Io {
            kind: io::ErrorKind::NotFound,
            ..
        }) => return Ok(false),
        Err(error) => return Err(error),
    };
    let entries = entries_for(&journal)?;
    match journal.phase {
        JournalPhase::Preparing => cleanup_preparing(journal_path, &entries)?,
        JournalPhase::Prepared => rollback(journal_path, &entries)?,
        JournalPhase::Committed => cleanup_committed(journal_path, &entries)?,
    }
    Ok(true)
}

fn publish(
    journal_path: &Path,
    prepared: &PreparedWorkspaceEdit,
) -> Result<WorkspaceEditPublicationReport, WorkspaceEditPublicationError> {
    publish_with_hook(journal_path, prepared, &mut |_| Ok(()))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PublicationStep {
    Backup(usize),
    Install(usize),
    CommitMarker,
    Cleanup,
}

type PublicationHook<'a> = dyn FnMut(PublicationStep) -> io::Result<()> + 'a;

fn publish_with_hook(
    journal_path: &Path,
    prepared: &PreparedWorkspaceEdit,
    hook: &mut PublicationHook<'_>,
) -> Result<WorkspaceEditPublicationReport, WorkspaceEditPublicationError> {
    let _ = recover_pending(journal_path)?;
    validate_prepared(prepared)?;
    let journal = next_journal(prepared)?;
    let entries = entries_for(&journal)?;
    preflight(prepared, &entries)?;
    write_journal(journal_path, &journal)?;
    if let Err(error) = stage_all(prepared, &entries) {
        return match cleanup_preparing(journal_path, &entries) {
            Ok(()) => Err(error),
            Err(rollback) => Err(rollback),
        };
    }

    let mut prepared_journal = journal.clone();
    prepared_journal.phase = JournalPhase::Prepared;
    if let Err(error) = write_journal(journal_path, &prepared_journal) {
        return match cleanup_preparing(journal_path, &entries) {
            Ok(()) => Err(error),
            Err(rollback) => Err(rollback),
        };
    }

    let apply_result = apply_entries(prepared, &entries, hook).and_then(|()| {
        hook(PublicationStep::CommitMarker).map_err(|error| io_error("commit-hook", &error))?;
        let mut committed = prepared_journal;
        committed.phase = JournalPhase::Committed;
        write_journal(journal_path, &committed)
    });
    if let Err(error) = apply_result {
        return match rollback(journal_path, &entries) {
            Ok(()) => Err(error),
            Err(
                WorkspaceEditPublicationError::Io { operation, kind }
                | WorkspaceEditPublicationError::RollbackIncomplete { operation, kind },
            ) => Err(WorkspaceEditPublicationError::RollbackIncomplete { operation, kind }),
            Err(other) => Err(other),
        };
    }

    let cleanup_deferred = hook(PublicationStep::Cleanup)
        .map_err(|error| io_error("cleanup-hook", &error))
        .and_then(|()| cleanup_committed(journal_path, &entries))
        .is_err();
    let bytes_written = prepared.files().iter().try_fold(0_usize, |bytes, file| {
        bytes
            .checked_add(file.replacement().len())
            .ok_or(WorkspaceEditPublicationError::JournalTooLarge)
    })?;
    Ok(WorkspaceEditPublicationReport {
        files: prepared.file_count(),
        edits: prepared.edit_count(),
        bytes_written,
        cleanup_deferred,
    })
}

fn validate_prepared(
    prepared: &PreparedWorkspaceEdit,
) -> Result<(), WorkspaceEditPublicationError> {
    if prepared.file_count() == 0 || prepared.edit_count() == 0 {
        return Err(WorkspaceEditPublicationError::Empty);
    }
    if prepared.file_count() > MAX_PUBLICATION_FILES {
        return Err(WorkspaceEditPublicationError::TooManyFiles);
    }
    if prepared
        .files()
        .windows(2)
        .any(|pair| pair[0].path() >= pair[1].path())
    {
        return Err(WorkspaceEditPublicationError::DuplicatePath);
    }
    for file in prepared.files() {
        validate_target_path(file.path())?;
    }
    Ok(())
}

fn validate_target_path(path: &Path) -> Result<(), WorkspaceEditPublicationError> {
    if !path.is_absolute() || path.parent().is_none() || path.file_name().is_none() {
        return Err(WorkspaceEditPublicationError::InvalidPath);
    }
    if path_bytes(path)?.len() > MAX_PATH_BYTES {
        return Err(WorkspaceEditPublicationError::PathTooLong);
    }
    Ok(())
}

fn next_journal(
    prepared: &PreparedWorkspaceEdit,
) -> Result<Journal, WorkspaceEditPublicationError> {
    next_journal_with_sequence(prepared, &PUBLICATION_SEQUENCE)
}

fn next_journal_with_sequence(
    prepared: &PreparedWorkspaceEdit,
    sequence_source: &AtomicU64,
) -> Result<Journal, WorkspaceEditPublicationError> {
    let process_id = std::process::id();
    for _ in 0..TEMPORARY_ATTEMPTS {
        let sequence = sequence_source.fetch_add(1, Ordering::Relaxed);
        let journal = Journal {
            phase: JournalPhase::Preparing,
            process_id,
            sequence,
            targets: prepared
                .files()
                .iter()
                .map(|file| file.path().to_path_buf())
                .collect::<Vec<_>>()
                .into_boxed_slice(),
        };
        let entries = entries_for(&journal)?;
        if entries
            .iter()
            .all(|entry| !entry.stage.exists() && !entry.backup.exists())
        {
            return Ok(journal);
        }
    }
    Err(WorkspaceEditPublicationError::ArtifactCollision)
}

fn entries_for(
    journal: &Journal,
) -> Result<Box<[PublicationEntry]>, WorkspaceEditPublicationError> {
    if journal.targets.is_empty() || journal.targets.len() > MAX_PUBLICATION_FILES {
        return Err(WorkspaceEditPublicationError::CorruptJournal);
    }
    let mut entries = Vec::new();
    entries
        .try_reserve_exact(journal.targets.len())
        .map_err(|_| WorkspaceEditPublicationError::JournalTooLarge)?;
    for (index, target) in journal.targets.iter().enumerate() {
        validate_target_path(target)?;
        let stage = artifact_path(target, journal.process_id, journal.sequence, index, "stage")?;
        let backup = artifact_path(
            target,
            journal.process_id,
            journal.sequence,
            index,
            "backup",
        )?;
        entries.push(PublicationEntry {
            target: target.clone(),
            stage,
            backup,
        });
    }
    if entries
        .windows(2)
        .any(|pair| pair[0].target >= pair[1].target)
    {
        return Err(WorkspaceEditPublicationError::DuplicatePath);
    }
    Ok(entries.into_boxed_slice())
}

fn artifact_path(
    target: &Path,
    process_id: u32,
    sequence: u64,
    index: usize,
    suffix: &str,
) -> Result<PathBuf, WorkspaceEditPublicationError> {
    let parent = target
        .parent()
        .ok_or(WorkspaceEditPublicationError::InvalidPath)?;
    let file_name = target
        .file_name()
        .ok_or(WorkspaceEditPublicationError::InvalidPath)?;
    let mut name = OsString::from(".");
    name.push(file_name);
    name.push(format!(
        ".alpine-workspace-edit-{process_id}-{sequence}-{index}.{suffix}"
    ));
    Ok(parent.join(name))
}

fn preflight(
    prepared: &PreparedWorkspaceEdit,
    entries: &[PublicationEntry],
) -> Result<(), WorkspaceEditPublicationError> {
    for (file, entry) in prepared.files().iter().zip(entries) {
        let metadata = fs::symlink_metadata(&entry.target)
            .map_err(|error| io_error("target-metadata", &error))?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(WorkspaceEditPublicationError::InvalidPath);
        }
        let canonical = fs::canonicalize(&entry.target)
            .map_err(|error| io_error("canonicalize-target", &error))?;
        if canonical != entry.target {
            return Err(WorkspaceEditPublicationError::StaleFile);
        }
        let bytes = fs::read(&entry.target).map_err(|error| io_error("read-target", &error))?;
        if bytes != file.original().as_bytes() {
            return Err(WorkspaceEditPublicationError::StaleFile);
        }
        if entry.stage.exists() || entry.backup.exists() {
            return Err(WorkspaceEditPublicationError::ArtifactCollision);
        }
    }
    Ok(())
}

fn stage_all(
    prepared: &PreparedWorkspaceEdit,
    entries: &[PublicationEntry],
) -> Result<(), WorkspaceEditPublicationError> {
    for (file, entry) in prepared.files().iter().zip(entries) {
        let permissions = fs::metadata(&entry.target)
            .map_err(|error| io_error("target-permissions", &error))?
            .permissions();
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut staged = options
            .open(&entry.stage)
            .map_err(|error| io_error("create-stage", &error))?;
        staged
            .set_permissions(permissions)
            .map_err(|error| io_error("stage-permissions", &error))?;
        staged
            .write_all(file.replacement().as_bytes())
            .map_err(|error| io_error("write-stage", &error))?;
        staged
            .flush()
            .map_err(|error| io_error("flush-stage", &error))?;
        staged
            .sync_all()
            .map_err(|error| io_error("sync-stage", &error))?;
        drop(staged);
        sync_parent(&entry.target)?;
    }
    Ok(())
}

fn apply_entries(
    prepared: &PreparedWorkspaceEdit,
    entries: &[PublicationEntry],
    hook: &mut PublicationHook<'_>,
) -> Result<(), WorkspaceEditPublicationError> {
    for (index, (file, entry)) in prepared.files().iter().zip(entries).enumerate() {
        let current =
            fs::read(&entry.target).map_err(|error| io_error("revalidate-target", &error))?;
        if current != file.original().as_bytes() {
            return Err(WorkspaceEditPublicationError::StaleFile);
        }
        fs::rename(&entry.target, &entry.backup)
            .map_err(|error| io_error("backup-target", &error))?;
        sync_parent(&entry.target)?;
        hook(PublicationStep::Backup(index)).map_err(|error| io_error("backup-hook", &error))?;
        fs::rename(&entry.stage, &entry.target)
            .map_err(|error| io_error("install-stage", &error))?;
        sync_parent(&entry.target)?;
        hook(PublicationStep::Install(index)).map_err(|error| io_error("install-hook", &error))?;
    }
    Ok(())
}

fn rollback(
    journal_path: &Path,
    entries: &[PublicationEntry],
) -> Result<(), WorkspaceEditPublicationError> {
    for entry in entries.iter().rev() {
        if entry.backup.exists() {
            if entry.target.exists() {
                fs::remove_file(&entry.target)
                    .map_err(|error| rollback_error("remove-installed", &error))?;
            }
            fs::rename(&entry.backup, &entry.target)
                .map_err(|error| rollback_error("restore-backup", &error))?;
        } else if !entry.target.exists() || !entry.stage.exists() {
            return Err(WorkspaceEditPublicationError::RollbackIncomplete {
                operation: "ambiguous-prepared-state",
                kind: io::ErrorKind::InvalidData,
            });
        }
        remove_if_exists(&entry.stage).map_err(|error| rollback_error("remove-stage", &error))?;
        sync_parent(&entry.target).map_err(|error| rollback_from_error("sync-rollback", &error))?;
    }
    remove_if_exists(journal_path).map_err(|error| rollback_error("remove-journal", &error))?;
    sync_parent(journal_path).map_err(|error| rollback_from_error("sync-journal", &error))
}

fn cleanup_preparing(
    journal_path: &Path,
    entries: &[PublicationEntry],
) -> Result<(), WorkspaceEditPublicationError> {
    for entry in entries {
        if entry.backup.exists() || !entry.target.exists() {
            return Err(WorkspaceEditPublicationError::RollbackIncomplete {
                operation: "ambiguous-preparing-state",
                kind: io::ErrorKind::InvalidData,
            });
        }
        remove_if_exists(&entry.stage).map_err(|error| rollback_error("remove-stage", &error))?;
        sync_parent(&entry.target)
            .map_err(|error| rollback_from_error("sync-preparing", &error))?;
    }
    remove_if_exists(journal_path).map_err(|error| rollback_error("remove-journal", &error))?;
    sync_parent(journal_path).map_err(|error| rollback_from_error("sync-journal", &error))
}

fn cleanup_committed(
    journal_path: &Path,
    entries: &[PublicationEntry],
) -> Result<(), WorkspaceEditPublicationError> {
    for entry in entries {
        remove_if_exists(&entry.stage)
            .map_err(|error| io_error("remove-committed-stage", &error))?;
        remove_if_exists(&entry.backup).map_err(|error| io_error("remove-backup", &error))?;
        sync_parent(&entry.target)?;
    }
    remove_if_exists(journal_path).map_err(|error| io_error("remove-journal", &error))?;
    sync_parent(journal_path)
}

fn remove_if_exists(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn write_journal(path: &Path, journal: &Journal) -> Result<(), WorkspaceEditPublicationError> {
    write_journal_with_sequence(path, journal, &PUBLICATION_SEQUENCE)
}

fn write_journal_with_sequence(
    path: &Path,
    journal: &Journal,
    sequence_source: &AtomicU64,
) -> Result<(), WorkspaceEditPublicationError> {
    let bytes = encode_journal(journal)?;
    let parent = path
        .parent()
        .ok_or(WorkspaceEditPublicationError::InvalidPath)?;
    fs::create_dir_all(parent).map_err(|error| io_error("create-journal-parent", &error))?;
    for _ in 0..TEMPORARY_ATTEMPTS {
        let sequence = sequence_source.fetch_add(1, Ordering::Relaxed);
        let temporary = journal_temporary_path(path, sequence)?;
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = match options.open(&temporary) {
            Ok(file) => file,
            Err(error) => match journal_open_disposition(error.kind()) {
                JournalOpenDisposition::Retry => continue,
                JournalOpenDisposition::Fail(kind) => {
                    return Err(WorkspaceEditPublicationError::Io {
                        operation: "create-journal",
                        kind,
                    });
                }
            },
        };
        let result = (|| {
            file.write_all(&bytes)
                .map_err(|error| io_error("write-journal", &error))?;
            file.flush()
                .map_err(|error| io_error("flush-journal", &error))?;
            file.sync_all()
                .map_err(|error| io_error("sync-journal", &error))?;
            drop(file);
            fs::rename(&temporary, path).map_err(|error| io_error("replace-journal", &error))?;
            sync_parent(path)
        })();
        if result.is_err() {
            let _ = remove_if_exists(&temporary);
        }
        return result;
    }
    Err(WorkspaceEditPublicationError::ArtifactCollision)
}

fn journal_temporary_path(
    path: &Path,
    sequence: u64,
) -> Result<PathBuf, WorkspaceEditPublicationError> {
    let parent = path
        .parent()
        .ok_or(WorkspaceEditPublicationError::InvalidPath)?;
    let file_name = path
        .file_name()
        .ok_or(WorkspaceEditPublicationError::InvalidPath)?;
    let mut name = OsString::from(".");
    name.push(file_name);
    name.push(format!(
        ".alpine-workspace-journal-{}-{sequence}",
        std::process::id()
    ));
    Ok(parent.join(name))
}

fn load_journal(path: &Path) -> Result<Journal, WorkspaceEditPublicationError> {
    let file = File::open(path).map_err(|error| io_error("open-journal", &error))?;
    let length = file
        .metadata()
        .map_err(|error| io_error("journal-metadata", &error))?
        .len();
    let length =
        usize::try_from(length).map_err(|_| WorkspaceEditPublicationError::JournalTooLarge)?;
    if length > MAX_JOURNAL_BYTES {
        return Err(WorkspaceEditPublicationError::JournalTooLarge);
    }
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(length)
        .map_err(|_| WorkspaceEditPublicationError::JournalTooLarge)?;
    file.take(u64::try_from(length).unwrap_or(u64::MAX))
        .read_to_end(&mut bytes)
        .map_err(|error| io_error("read-journal", &error))?;
    decode_journal(&bytes)
}

fn encode_journal(journal: &Journal) -> Result<Vec<u8>, WorkspaceEditPublicationError> {
    let mut bytes = Vec::new();
    bytes
        .try_reserve(journal_reserve_hint(journal.targets.len()))
        .map_err(|_| WorkspaceEditPublicationError::JournalTooLarge)?;
    bytes.extend_from_slice(JOURNAL_MAGIC);
    bytes.extend_from_slice(&JOURNAL_VERSION.to_le_bytes());
    bytes.push(journal.phase as u8);
    bytes.extend_from_slice(&journal.process_id.to_le_bytes());
    bytes.extend_from_slice(&journal.sequence.to_le_bytes());
    let count = u32::try_from(journal.targets.len())
        .map_err(|_| WorkspaceEditPublicationError::TooManyFiles)?;
    bytes.extend_from_slice(&count.to_le_bytes());
    for target in &journal.targets {
        let path = path_bytes(target)?;
        if path.len() > MAX_PATH_BYTES {
            return Err(WorkspaceEditPublicationError::PathTooLong);
        }
        let length =
            u32::try_from(path.len()).map_err(|_| WorkspaceEditPublicationError::PathTooLong)?;
        bytes.extend_from_slice(&length.to_le_bytes());
        bytes.extend_from_slice(&path);
    }
    let checksum = crc32(&bytes);
    bytes.extend_from_slice(&checksum.to_le_bytes());
    ensure_encoded_journal_size(bytes.len())?;
    Ok(bytes)
}

#[cfg_attr(test, mutants::skip)] // Capacity hints do not alter encoded bytes or admission.
fn journal_reserve_hint(target_count: usize) -> usize {
    MAX_JOURNAL_BYTES.min(512_usize.saturating_add(target_count.saturating_mul(64)))
}

#[cfg_attr(test, mutants::skip)] // Prior file and path caps make this final defense unreachable for valid journals.
const fn ensure_encoded_journal_size(length: usize) -> Result<(), WorkspaceEditPublicationError> {
    if length > MAX_JOURNAL_BYTES {
        Err(WorkspaceEditPublicationError::JournalTooLarge)
    } else {
        Ok(())
    }
}

fn decode_journal(bytes: &[u8]) -> Result<Journal, WorkspaceEditPublicationError> {
    if bytes.len() > MAX_JOURNAL_BYTES {
        return Err(WorkspaceEditPublicationError::JournalTooLarge);
    }
    let checksum_offset = bytes
        .len()
        .checked_sub(4)
        .ok_or(WorkspaceEditPublicationError::CorruptJournal)?;
    let (payload, checksum_bytes) = bytes.split_at(checksum_offset);
    let expected = u32::from_le_bytes(
        checksum_bytes
            .try_into()
            .map_err(|_| WorkspaceEditPublicationError::CorruptJournal)?,
    );
    if crc32(payload) != expected {
        return Err(WorkspaceEditPublicationError::CorruptJournal);
    }
    let mut reader = JournalReader::new(payload);
    if reader.take(JOURNAL_MAGIC.len())? != JOURNAL_MAGIC {
        return Err(WorkspaceEditPublicationError::CorruptJournal);
    }
    if reader.u32()? != JOURNAL_VERSION {
        return Err(WorkspaceEditPublicationError::CorruptJournal);
    }
    let phase = JournalPhase::decode(reader.u8()?)?;
    let process_id = reader.u32()?;
    let sequence = reader.u64()?;
    let count = usize::try_from(reader.u32()?)
        .map_err(|_| WorkspaceEditPublicationError::CorruptJournal)?;
    if count == 0 {
        return Err(WorkspaceEditPublicationError::CorruptJournal);
    }
    if count > MAX_PUBLICATION_FILES {
        return Err(WorkspaceEditPublicationError::TooManyFiles);
    }
    let mut targets = Vec::new();
    targets
        .try_reserve_exact(count)
        .map_err(|_| WorkspaceEditPublicationError::JournalTooLarge)?;
    for _ in 0..count {
        let length = usize::try_from(reader.u32()?)
            .map_err(|_| WorkspaceEditPublicationError::CorruptJournal)?;
        if length == 0 {
            return Err(WorkspaceEditPublicationError::CorruptJournal);
        }
        if length > MAX_PATH_BYTES {
            return Err(WorkspaceEditPublicationError::PathTooLong);
        }
        targets.push(path_from_bytes(reader.take(length)?)?);
    }
    if !reader.is_empty() {
        return Err(WorkspaceEditPublicationError::CorruptJournal);
    }
    let journal = Journal {
        phase,
        process_id,
        sequence,
        targets: targets.into_boxed_slice(),
    };
    let _ = entries_for(&journal)?;
    Ok(journal)
}

struct JournalReader<'a> {
    bytes: &'a [u8],
    cursor: usize,
}

impl<'a> JournalReader<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, cursor: 0 }
    }

    fn take(&mut self, length: usize) -> Result<&'a [u8], WorkspaceEditPublicationError> {
        let end = self
            .cursor
            .checked_add(length)
            .ok_or(WorkspaceEditPublicationError::CorruptJournal)?;
        let value = self
            .bytes
            .get(self.cursor..end)
            .ok_or(WorkspaceEditPublicationError::CorruptJournal)?;
        self.cursor = end;
        Ok(value)
    }

    fn u8(&mut self) -> Result<u8, WorkspaceEditPublicationError> {
        self.take(1)?
            .first()
            .copied()
            .ok_or(WorkspaceEditPublicationError::CorruptJournal)
    }

    fn u32(&mut self) -> Result<u32, WorkspaceEditPublicationError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().map_err(
            |_| WorkspaceEditPublicationError::CorruptJournal,
        )?))
    }

    fn u64(&mut self) -> Result<u64, WorkspaceEditPublicationError> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().map_err(
            |_| WorkspaceEditPublicationError::CorruptJournal,
        )?))
    }

    const fn is_empty(&self) -> bool {
        self.cursor == self.bytes.len()
    }
}

fn path_bytes(path: &Path) -> Result<Vec<u8>, WorkspaceEditPublicationError> {
    if path.as_os_str().is_empty() {
        return Err(WorkspaceEditPublicationError::InvalidPath);
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        Ok(path.as_os_str().as_bytes().to_vec())
    }
    #[cfg(not(unix))]
    {
        path.to_str()
            .map(|value| value.as_bytes().to_vec())
            .ok_or(WorkspaceEditPublicationError::InvalidPath)
    }
}

fn path_from_bytes(bytes: &[u8]) -> Result<PathBuf, WorkspaceEditPublicationError> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        if bytes.is_empty() {
            Err(WorkspaceEditPublicationError::CorruptJournal)
        } else {
            Ok(PathBuf::from(OsString::from_vec(bytes.to_vec())))
        }
    }
    #[cfg(not(unix))]
    {
        if bytes.is_empty() {
            return Err(WorkspaceEditPublicationError::CorruptJournal);
        }
        String::from_utf8(bytes.to_vec())
            .map(PathBuf::from)
            .map_err(|_| WorkspaceEditPublicationError::CorruptJournal)
    }
}

fn sync_parent(path: &Path) -> Result<(), WorkspaceEditPublicationError> {
    let parent = path
        .parent()
        .ok_or(WorkspaceEditPublicationError::InvalidPath)?;
    #[cfg(unix)]
    {
        File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| io_error("sync-directory", &error))
    }
    #[cfg(not(unix))]
    {
        let _ = parent;
        Ok(())
    }
}

fn io_error(operation: &'static str, error: &io::Error) -> WorkspaceEditPublicationError {
    WorkspaceEditPublicationError::Io {
        operation,
        kind: error.kind(),
    }
}

fn rollback_error(operation: &'static str, error: &io::Error) -> WorkspaceEditPublicationError {
    WorkspaceEditPublicationError::RollbackIncomplete {
        operation,
        kind: error.kind(),
    }
}

const fn rollback_from_error(
    operation: &'static str,
    error: &WorkspaceEditPublicationError,
) -> WorkspaceEditPublicationError {
    match *error {
        WorkspaceEditPublicationError::Io { kind, .. }
        | WorkspaceEditPublicationError::RollbackIncomplete { kind, .. } => {
            WorkspaceEditPublicationError::RollbackIncomplete { operation, kind }
        }
        _ => WorkspaceEditPublicationError::RollbackIncomplete {
            operation,
            kind: io::ErrorKind::Other,
        },
    }
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = 0_u32.wrapping_sub(crc & 1);
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    #[test]
    fn empty_path_bytes_are_rejected_before_journal_ownership() {
        assert_eq!(
            super::path_bytes(std::path::Path::new("")),
            Err(super::WorkspaceEditPublicationError::InvalidPath)
        );
    }

    use std::{
        fs,
        path::Path,
        sync::atomic::{AtomicU64, Ordering},
    };

    use serde_json::value::RawValue;

    use super::{
        Journal, JournalOpenDisposition, JournalPhase, MAX_JOURNAL_BYTES, MAX_PATH_BYTES,
        PublicationStep, WorkspaceEditPublicationError, cleanup_committed, cleanup_preparing,
        crc32, decode_journal, encode_journal, entries_for, journal_open_disposition,
        journal_temporary_path, load_journal, next_journal, next_journal_with_sequence, path_bytes,
        path_from_bytes, preflight, publish_with_hook, recover_pending, remove_if_exists,
        rollback_from_error, stage_all, sync_parent, validate_prepared, validate_target_path,
        write_journal, write_journal_with_sequence,
    };
    use crate::{rust_navigation::local_file_uri, rust_workspace_edit::WorkspaceEditProposal};

    static FIXTURE: AtomicU64 = AtomicU64::new(1);

    fn fixture() -> Result<std::path::PathBuf, Box<dyn std::error::Error>> {
        let root = std::env::temp_dir().join(format!(
            "alpine-workspace-publish-{}-{}",
            std::process::id(),
            FIXTURE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root)?;
        Ok(fs::canonicalize(root)?)
    }

    fn prepared(
        root: &Path,
        files: &[(&Path, &str, &str)],
    ) -> Result<crate::rust_workspace_edit::PreparedWorkspaceEdit, Box<dyn std::error::Error>> {
        let mut changes = serde_json::Map::new();
        for (path, original, replacement) in files {
            fs::write(path, original)?;
            let uri = local_file_uri(path);
            changes.insert(
                uri,
                serde_json::json!([{
                    "range": {
                        "start": {"line": 0, "character": 0},
                        "end": {"line": 1, "character": 0}
                    },
                    "newText": replacement
                }]),
            );
        }
        let value = serde_json::json!({"changes": changes});
        let raw = RawValue::from_string(value.to_string())?;
        Ok(WorkspaceEditProposal::admit_rename(&raw, root)?.prepare()?)
    }

    #[test]
    fn two_file_publication_is_durable_and_cleans_every_artifact()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = fixture()?;
        let first = root.join("a.rs");
        let second = root.join("b.rs");
        let edit = prepared(
            &root,
            &[
                (&first, "old_a\n", "new_a\n"),
                (&second, "old_b\n", "new_b\n"),
            ],
        )?;
        let journal = root.join("journal.bin");
        let report = publish_with_hook(&journal, &edit, &mut |_| Ok(()))?;
        assert_eq!(report.files, 2);
        assert_eq!(report.edits, 2);
        assert_eq!(report.bytes_written, 12);
        assert!(!report.cleanup_deferred);
        assert_eq!(fs::read_to_string(&first)?, "new_a\n");
        assert_eq!(fs::read_to_string(&second)?, "new_b\n");
        assert!(!journal.exists());
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn prepared_and_path_validation_guards_are_independently_discriminating()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = fixture()?;
        let target = root.join("a.rs");
        let edit = prepared(&root, &[(&target, "old\n", "new\n")])?;
        assert_eq!(validate_prepared(&edit), Ok(()));
        assert_eq!(
            validate_prepared(&edit.publication_fixture_for_test(0, true)),
            Err(WorkspaceEditPublicationError::Empty)
        );
        assert_eq!(
            validate_prepared(&edit.publication_fixture_for_test(1, false)),
            Err(WorkspaceEditPublicationError::Empty)
        );
        assert_eq!(
            validate_prepared(&edit.publication_fixture_for_test(32, true)),
            Err(WorkspaceEditPublicationError::DuplicatePath)
        );
        assert_eq!(
            validate_prepared(&edit.publication_fixture_for_test(33, true)),
            Err(WorkspaceEditPublicationError::TooManyFiles)
        );

        assert_eq!(
            validate_target_path(Path::new("relative.rs")),
            Err(WorkspaceEditPublicationError::InvalidPath)
        );
        assert_eq!(
            validate_target_path(Path::new("/")),
            Err(WorkspaceEditPublicationError::InvalidPath)
        );
        assert_eq!(
            validate_target_path(Path::new("/..")),
            Err(WorkspaceEditPublicationError::InvalidPath)
        );
        let prefix_bytes = path_bytes(&root)?.len().saturating_add(1);
        let exact = root.join("x".repeat(MAX_PATH_BYTES.saturating_sub(prefix_bytes)));
        assert_eq!(path_bytes(&exact)?.len(), MAX_PATH_BYTES);
        assert_eq!(validate_target_path(&exact), Ok(()));
        let oversized = root.join(
            "x".repeat(
                MAX_PATH_BYTES
                    .saturating_sub(prefix_bytes)
                    .saturating_add(1),
            ),
        );
        assert_eq!(
            validate_target_path(&oversized),
            Err(WorkspaceEditPublicationError::PathTooLong)
        );
        fs::remove_dir_all(root)?;
        Ok(())
    }

    fn journal_with_targets(targets: Vec<std::path::PathBuf>) -> Journal {
        Journal {
            phase: JournalPhase::Preparing,
            process_id: 7,
            sequence: 11,
            targets: targets.into_boxed_slice(),
        }
    }

    #[test]
    fn journal_artifact_identity_counts_and_temporary_collisions_are_exact()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = fixture()?;
        let target = root.join("a.rs");
        let edit = prepared(&root, &[(&target, "old\n", "new\n")])?;

        let sequence = AtomicU64::new(700_000);
        let collided = Journal {
            phase: JournalPhase::Preparing,
            process_id: std::process::id(),
            sequence: 700_000,
            targets: vec![target.clone()].into_boxed_slice(),
        };
        let collided_entries = entries_for(&collided)?;
        fs::write(&collided_entries[0].stage, "collision")?;
        let selected = next_journal_with_sequence(&edit, &sequence)?;
        assert_eq!(selected.sequence, 700_001);
        fs::remove_file(&collided_entries[0].stage)?;

        assert_eq!(
            entries_for(&journal_with_targets(Vec::new())),
            Err(WorkspaceEditPublicationError::CorruptJournal)
        );
        let exact_targets = (0..32)
            .map(|index| root.join(format!("{index:02}.rs")))
            .collect::<Vec<_>>();
        assert_eq!(entries_for(&journal_with_targets(exact_targets))?.len(), 32);
        let excessive_targets = (0..33)
            .map(|index| root.join(format!("{index:02}.rs")))
            .collect::<Vec<_>>();
        assert_eq!(
            entries_for(&journal_with_targets(excessive_targets)),
            Err(WorkspaceEditPublicationError::CorruptJournal)
        );
        assert_eq!(
            entries_for(&journal_with_targets(vec![target.clone(), target.clone()])),
            Err(WorkspaceEditPublicationError::DuplicatePath)
        );

        let journal_path = root.join("journal.bin");
        let temporary_sequence = AtomicU64::new(800_000);
        let collision = journal_temporary_path(&journal_path, 800_000)?;
        fs::write(&collision, "occupied")?;
        write_journal_with_sequence(&journal_path, &selected, &temporary_sequence)?;
        assert_eq!(load_journal(&journal_path)?, selected);
        assert_eq!(fs::read_to_string(&collision)?, "occupied");
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn preflight_and_preparing_cleanup_reject_each_artifact_state()
    -> Result<(), Box<dyn std::error::Error>> {
        use std::os::unix::fs::symlink;

        let root = fixture()?;
        let target = root.join("a.rs");
        let edit = prepared(&root, &[(&target, "old\n", "new\n")])?;
        let journal = next_journal(&edit)?;
        let entries = entries_for(&journal)?;
        assert_eq!(preflight(&edit, &entries), Ok(()));

        fs::write(&entries[0].stage, "stage")?;
        assert_eq!(
            preflight(&edit, &entries),
            Err(WorkspaceEditPublicationError::ArtifactCollision)
        );
        fs::remove_file(&entries[0].stage)?;
        fs::write(&entries[0].backup, "backup")?;
        assert_eq!(
            preflight(&edit, &entries),
            Err(WorkspaceEditPublicationError::ArtifactCollision)
        );
        assert!(matches!(
            cleanup_preparing(&root.join("missing-journal"), &entries),
            Err(WorkspaceEditPublicationError::RollbackIncomplete {
                operation: "ambiguous-preparing-state",
                ..
            })
        ));
        fs::remove_file(&entries[0].backup)?;
        fs::remove_file(&target)?;
        assert!(matches!(
            cleanup_preparing(&root.join("missing-journal"), &entries),
            Err(WorkspaceEditPublicationError::RollbackIncomplete {
                operation: "ambiguous-preparing-state",
                ..
            })
        ));

        let real = root.join("real.rs");
        fs::write(&real, "old\n")?;
        symlink(&real, &target)?;
        assert_eq!(
            preflight(&edit, &entries),
            Err(WorkspaceEditPublicationError::InvalidPath)
        );
        fs::remove_file(&target)?;
        fs::create_dir(&target)?;
        assert_eq!(
            preflight(&edit, &entries),
            Err(WorkspaceEditPublicationError::InvalidPath)
        );
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn codec_boundaries_error_mapping_and_checksum_are_exact()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = fixture()?;
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
        assert_eq!(
            WorkspaceEditPublicationError::StaleFile.to_string(),
            "Rust workspace edit publication failed: StaleFile"
        );
        assert_eq!(
            journal_open_disposition(std::io::ErrorKind::AlreadyExists),
            JournalOpenDisposition::Retry
        );
        assert_eq!(
            journal_open_disposition(std::io::ErrorKind::PermissionDenied),
            JournalOpenDisposition::Fail(std::io::ErrorKind::PermissionDenied)
        );
        for length in 0..4 {
            assert_eq!(
                decode_journal(&vec![0_u8; length]),
                Err(WorkspaceEditPublicationError::CorruptJournal)
            );
        }
        assert_eq!(
            rollback_from_error(
                "mapped",
                &WorkspaceEditPublicationError::Io {
                    operation: "source",
                    kind: std::io::ErrorKind::StorageFull,
                },
            ),
            WorkspaceEditPublicationError::RollbackIncomplete {
                operation: "mapped",
                kind: std::io::ErrorKind::StorageFull,
            }
        );
        assert_eq!(
            rollback_from_error("mapped", &WorkspaceEditPublicationError::Empty),
            WorkspaceEditPublicationError::RollbackIncomplete {
                operation: "mapped",
                kind: std::io::ErrorKind::Other,
            }
        );

        let exact_targets = (0..32)
            .map(|index| root.join(format!("{index:02}")))
            .collect::<Vec<_>>();
        let exact = journal_with_targets(exact_targets);
        assert_eq!(decode_journal(&encode_journal(&exact)?)?, exact);
        let excessive_targets = (0..33)
            .map(|index| root.join(format!("{index:02}")))
            .collect::<Vec<_>>();
        assert_eq!(
            decode_journal(&encode_journal(&journal_with_targets(excessive_targets))?),
            Err(WorkspaceEditPublicationError::TooManyFiles)
        );

        let prefix_bytes = path_bytes(&root)?.len().saturating_add(1);
        let exact_path = root.join("x".repeat(MAX_PATH_BYTES.saturating_sub(prefix_bytes)));
        let exact_path_journal = journal_with_targets(vec![exact_path]);
        assert_eq!(
            decode_journal(&encode_journal(&exact_path_journal)?)?,
            exact_path_journal
        );
        let long_path = root.join(
            "x".repeat(
                MAX_PATH_BYTES
                    .saturating_sub(prefix_bytes)
                    .saturating_add(1),
            ),
        );
        assert_eq!(
            encode_journal(&journal_with_targets(vec![long_path])),
            Err(WorkspaceEditPublicationError::PathTooLong)
        );

        let round_trip = root.join("round-trip.rs");
        let encoded_path = path_bytes(&round_trip)?;
        assert_eq!(path_from_bytes(&encoded_path)?, round_trip);
        assert_eq!(
            path_bytes(Path::new("")),
            Err(WorkspaceEditPublicationError::InvalidPath)
        );
        assert_eq!(
            path_from_bytes(&[]),
            Err(WorkspaceEditPublicationError::CorruptJournal)
        );
        assert_eq!(
            sync_parent(Path::new("")),
            Err(WorkspaceEditPublicationError::InvalidPath)
        );
        assert!(remove_if_exists(&root).is_err());
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[cfg_attr(
        miri,
        ignore = "the 512 KiB journal ceiling is covered natively; Miri exercises bounded codec and checksum semantics"
    )]
    #[test]
    fn exact_journal_size_limit_is_rejected_as_corrupt() -> Result<(), Box<dyn std::error::Error>> {
        let root = fixture()?;
        let exact_size = root.join("exact-size.bin");
        fs::write(&exact_size, vec![0_u8; MAX_JOURNAL_BYTES])?;
        assert_eq!(
            load_journal(&exact_size),
            Err(WorkspaceEditPublicationError::CorruptJournal)
        );
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn loaded_editor_admits_committed_bytes_as_clean_and_undoable()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = fixture()?;
        let target = root.join("active.rs");
        let edit = prepared(&root, &[(&target, "old\n", "new\n")])?;
        let mut editor = alpine_text::Editor::open(&target)?;
        let transaction = edit.files()[0].transaction_for(&editor.buffer().snapshot())?;
        let journal = root.join("journal.bin");

        let _ = publish_with_hook(&journal, &edit, &mut |_| Ok(()))?;
        let (_, report) =
            editor.admit_persisted_transaction(transaction, edit.files()[0].replacement())?;
        assert_eq!(report.bytes_written(), 4);
        assert!(!editor.is_dirty());
        assert_eq!(editor.buffer().snapshot().text(), "new\n");
        assert!(editor.buffer_mut().undo()?);
        assert!(editor.is_dirty());
        assert_eq!(editor.buffer().snapshot().text(), "old\n");
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn stale_preflight_and_mid_install_failure_preserve_every_original_byte()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = fixture()?;
        let first = root.join("a.rs");
        let second = root.join("b.rs");
        let edit = prepared(
            &root,
            &[
                (&first, "old_a\n", "new_a\n"),
                (&second, "old_b\n", "new_b\n"),
            ],
        )?;
        let journal = root.join("journal.bin");
        let failure = publish_with_hook(&journal, &edit, &mut |step| {
            if step == PublicationStep::Install(0) {
                Err(std::io::Error::other("injected install failure"))
            } else {
                Ok(())
            }
        });
        assert!(failure.is_err());
        assert_eq!(fs::read_to_string(&first)?, "old_a\n");
        assert_eq!(fs::read_to_string(&second)?, "old_b\n");
        assert!(!journal.exists());

        fs::write(&first, "external\n")?;
        assert!(publish_with_hook(&journal, &edit, &mut |_| Ok(())).is_err());
        assert_eq!(fs::read_to_string(&first)?, "external\n");
        assert_eq!(fs::read_to_string(&second)?, "old_b\n");
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn prepared_and_committed_crash_states_recover_in_the_correct_direction()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = fixture()?;
        let first = root.join("a.rs");
        let edit = prepared(&root, &[(&first, "old\n", "new\n")])?;
        let journal_path = root.join("journal.bin");

        let preparing = next_journal(&edit)?;
        let entries = entries_for(&preparing)?;
        write_journal(&journal_path, &preparing)?;
        stage_all(&edit, &entries)?;
        assert!(recover_pending(&journal_path)?);
        assert_eq!(fs::read_to_string(&first)?, "old\n");
        assert!(!entries[0].stage.exists());

        let mut journal = next_journal(&edit)?;
        let entries = entries_for(&journal)?;
        write_journal(&journal_path, &journal)?;
        stage_all(&edit, &entries)?;
        journal.phase = JournalPhase::Prepared;
        write_journal(&journal_path, &journal)?;
        fs::rename(&entries[0].target, &entries[0].backup)?;
        fs::rename(&entries[0].stage, &entries[0].target)?;
        assert!(recover_pending(&journal_path)?);
        assert_eq!(fs::read_to_string(&first)?, "old\n");

        let mut journal = next_journal(&edit)?;
        let entries = entries_for(&journal)?;
        write_journal(&journal_path, &journal)?;
        stage_all(&edit, &entries)?;
        journal.phase = JournalPhase::Prepared;
        write_journal(&journal_path, &journal)?;
        fs::rename(&entries[0].target, &entries[0].backup)?;
        fs::rename(&entries[0].stage, &entries[0].target)?;
        journal.phase = JournalPhase::Committed;
        write_journal(&journal_path, &journal)?;
        assert_eq!(load_journal(&journal_path)?.phase, JournalPhase::Committed);
        assert!(recover_pending(&journal_path)?);
        assert_eq!(fs::read_to_string(&first)?, "new\n");
        assert!(!journal_path.exists());
        cleanup_committed(&journal_path, &entries)?;
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn interleaved_external_change_and_commit_failure_roll_back_without_overwrite()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = fixture()?;
        let first = root.join("a.rs");
        let second = root.join("b.rs");
        let edit = prepared(
            &root,
            &[
                (&first, "old_a\n", "new_a\n"),
                (&second, "old_b\n", "new_b\n"),
            ],
        )?;
        let journal = root.join("journal.bin");
        let failure = publish_with_hook(&journal, &edit, &mut |step| {
            if step == PublicationStep::Install(0) {
                fs::write(&second, "external\n")?;
            }
            Ok(())
        });
        assert_eq!(failure, Err(WorkspaceEditPublicationError::StaleFile));
        assert_eq!(fs::read_to_string(&first)?, "old_a\n");
        assert_eq!(fs::read_to_string(&second)?, "external\n");
        assert!(!journal.exists());

        fs::write(&second, "old_b\n")?;
        let failure = publish_with_hook(&journal, &edit, &mut |step| {
            if step == PublicationStep::CommitMarker {
                Err(std::io::Error::other("injected commit failure"))
            } else {
                Ok(())
            }
        });
        assert!(failure.is_err());
        assert_eq!(fs::read_to_string(&first)?, "old_a\n");
        assert_eq!(fs::read_to_string(&second)?, "old_b\n");
        assert!(!journal.exists());
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn durable_commit_defers_cleanup_and_startup_finishes_it()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = fixture()?;
        let target = root.join("a.rs");
        let edit = prepared(&root, &[(&target, "old\n", "new\n")])?;
        let journal = root.join("journal.bin");
        let report = publish_with_hook(&journal, &edit, &mut |step| {
            if step == PublicationStep::Cleanup {
                Err(std::io::Error::other("injected cleanup failure"))
            } else {
                Ok(())
            }
        })?;
        assert!(report.cleanup_deferred);
        assert_eq!(fs::read_to_string(&target)?, "new\n");
        assert_eq!(load_journal(&journal)?.phase, JournalPhase::Committed);
        assert!(recover_pending(&journal)?);
        assert_eq!(fs::read_to_string(&target)?, "new\n");
        assert!(!journal.exists());
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn ambiguous_prepared_state_fails_closed_with_journal_retained()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = fixture()?;
        let target = root.join("a.rs");
        let edit = prepared(&root, &[(&target, "old\n", "new\n")])?;
        let journal_path = root.join("journal.bin");
        let mut journal = next_journal(&edit)?;
        let entries = entries_for(&journal)?;
        write_journal(&journal_path, &journal)?;
        stage_all(&edit, &entries)?;
        journal.phase = JournalPhase::Prepared;
        write_journal(&journal_path, &journal)?;
        fs::rename(&entries[0].target, &entries[0].backup)?;
        fs::rename(&entries[0].stage, &entries[0].target)?;
        fs::remove_file(&entries[0].backup)?;

        assert!(matches!(
            recover_pending(&journal_path),
            Err(WorkspaceEditPublicationError::RollbackIncomplete {
                operation: "ambiguous-prepared-state",
                ..
            })
        ));
        assert_eq!(fs::read_to_string(&target)?, "new\n");
        assert!(journal_path.exists());
        fs::remove_dir_all(root)?;
        Ok(())
    }

    fn resign(bytes: &mut [u8]) {
        let payload_length = bytes.len() - 4;
        let checksum = crc32(&bytes[..payload_length]);
        bytes[payload_length..].copy_from_slice(&checksum.to_le_bytes());
    }

    #[test]
    fn journal_codec_rejects_each_identity_and_bound_axis() -> Result<(), Box<dyn std::error::Error>>
    {
        let root = fixture()?;
        let target = root.join("a.rs");
        let edit = prepared(&root, &[(&target, "old\n", "new\n")])?;
        let journal = next_journal(&edit)?;
        let encoded = encode_journal(&journal)?;
        assert_eq!(decode_journal(&encoded)?, journal);

        for (offset, value) in [(8_usize, 2_u8), (12, 99)] {
            let mut corrupt = encoded.clone();
            corrupt[offset] = value;
            resign(&mut corrupt);
            assert_eq!(
                decode_journal(&corrupt),
                Err(WorkspaceEditPublicationError::CorruptJournal)
            );
        }

        let mut zero_count = encoded.clone();
        zero_count[25..29].copy_from_slice(&0_u32.to_le_bytes());
        resign(&mut zero_count);
        assert_eq!(
            decode_journal(&zero_count),
            Err(WorkspaceEditPublicationError::CorruptJournal)
        );

        let mut long_path = encoded.clone();
        long_path[29..33].copy_from_slice(&u32::try_from(MAX_PATH_BYTES + 1)?.to_le_bytes());
        resign(&mut long_path);
        assert_eq!(
            decode_journal(&long_path),
            Err(WorkspaceEditPublicationError::PathTooLong)
        );

        let mut trailing = encoded.clone();
        trailing.insert(trailing.len() - 4, 0);
        resign(&mut trailing);
        assert_eq!(
            decode_journal(&trailing),
            Err(WorkspaceEditPublicationError::CorruptJournal)
        );

        let oversized = root.join("oversized.bin");
        fs::write(&oversized, vec![0_u8; MAX_JOURNAL_BYTES + 1])?;
        assert_eq!(
            load_journal(&oversized),
            Err(WorkspaceEditPublicationError::JournalTooLarge)
        );
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn corrupt_journal_is_rejected_without_touching_targets()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = fixture()?;
        let target = root.join("a.rs");
        fs::write(&target, "untouched\n")?;
        let journal = root.join("journal.bin");
        fs::write(&journal, b"corrupt")?;
        assert!(recover_pending(&journal).is_err());
        assert_eq!(fs::read_to_string(target)?, "untouched\n");
        fs::remove_dir_all(root)?;
        Ok(())
    }
}
