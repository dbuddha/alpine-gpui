//! Workspace overlay ownership, independent of the active editor view.

use super::{
    AdmittedDiagnostics, Arc, BufferSnapshot, LanguageEffect, LanguageIdentity,
    LanguageProtocolError, LanguageWake, LspDocument, Path, ProcessWake, RustDiagnostics,
    RustDiagnosticsError, RustDocumentInput, RustSession, SessionState, Target, document_end,
    replace_status,
};
use crate::lsp_process::InputSequence;

pub(super) const MAX_OVERLAY_DOCUMENTS: usize = 32;
const MAX_OVERLAY_RETAINED_TEXT_BYTES: usize = 48 * 1_024 * 1_024;
const MAX_DOCUMENT_BYTES: usize = 8_388_608;

// Closing work owns the writer before ordinary overlay publication. An empty
// close queue permits fallthrough only when no previous write is outstanding.
enum ClosingFlush {
    Empty,
    Busy,
    Sent,
}

// One coalescing disk-change notification per owned document. The revision
// orders it behind overlay writes; it does not identify rustc's checked input.
pub(super) struct PendingSave {
    buffer_revision: u64,
    submitted: Option<InputSequence>,
}

fn acknowledge_save(save: &mut Option<PendingSave>, sequence: InputSequence) {
    if save
        .as_ref()
        .is_some_and(|save| save.submitted == Some(sequence))
    {
        *save = None;
    }
}

fn retry_save(save: &mut Option<PendingSave>) {
    if let Some(save) = save {
        save.submitted = None;
    }
}

pub(super) struct ParkedDocument {
    target: Target,
    identity: LanguageIdentity,
    document: LspDocument,
    snapshot: BufferSnapshot,
    synced_snapshot: BufferSnapshot,
    lsp_version: i32,
    pending_change: bool,
    pending_save: Option<PendingSave>,
    opened: bool,
    diagnostics: Option<AdmittedDiagnostics>,
    saved_compiler: Option<super::saved_compiler::SavedCompilerReport>,
}

// A removed tab owns only the transport work still required before didClose.
// Diagnostics and ordinary unsaved snapshots do not survive tab closure.
pub(super) struct ClosingDocument {
    document: LspDocument,
    opened: bool,
    pending_save: Option<PendingSave>,
    pending_text: Option<(BufferSnapshot, BufferSnapshot)>,
}

fn needs_closing_text(opened: bool, changed: bool, save: Option<&PendingSave>) -> bool {
    save.is_some_and(|save| save.submitted.is_none()) && (!opened || changed)
}

impl ClosingDocument {
    fn from_parked(document: ParkedDocument) -> Self {
        let pending_text = needs_closing_text(
            document.opened,
            document.pending_change,
            document.pending_save.as_ref(),
        )
        .then_some((document.snapshot, document.synced_snapshot));
        Self {
            document: document.document,
            opened: document.opened,
            pending_save: document.pending_save,
            pending_text,
        }
    }

    fn retained_text_bytes(&self) -> usize {
        self.pending_text.as_ref().map_or(0, |(current, synced)| {
            current.len_bytes().saturating_add(synced.len_bytes())
        })
    }

    fn reserved_text_bytes(&self) -> usize {
        self.pending_text.as_ref().map_or(0, |(current, synced)| {
            snapshot_reservation(current.len_bytes(), synced.len_bytes())
        })
    }
}

impl ParkedDocument {
    fn new(input: RustDocumentInput) -> Result<Self, RustDiagnosticsError> {
        validate_input(&input)?;
        Ok(Self {
            target: Target {
                path: input.path.clone(),
                workspace_root: input.workspace_root,
            },
            identity: input.identity,
            document: LspDocument::from_file_path(&input.path, "rust", 1)
                .map_err(RustDiagnosticsError::Language)?,
            synced_snapshot: input.snapshot.clone(),
            snapshot: input.snapshot,
            lsp_version: 1,
            pending_change: false,
            pending_save: None,
            opened: false,
            diagnostics: None,
            saved_compiler: None,
        })
    }

    fn update(&mut self, input: RustDocumentInput) -> Result<bool, RustDiagnosticsError> {
        validate_input(&input)?;
        let changed = self.identity.buffer_revision != input.identity.buffer_revision;
        if changed {
            self.lsp_version = self
                .lsp_version
                .checked_add(1)
                .ok_or(RustDiagnosticsError::VersionExhausted)?;
            self.document.set_version(self.lsp_version);
            self.snapshot = input.snapshot;
            self.pending_change = self.opened;
            self.diagnostics = None;
        }
        self.identity = input.identity;
        Ok(changed)
    }

    fn swap_active(&mut self, session: &mut RustSession) {
        use std::mem::swap;
        swap(&mut self.target, &mut session.target);
        swap(&mut self.identity, &mut session.identity);
        swap(&mut self.document, &mut session.document);
        swap(&mut self.snapshot, &mut session.snapshot);
        swap(&mut self.synced_snapshot, &mut session.synced_snapshot);
        swap(&mut self.lsp_version, &mut session.lsp_version);
        swap(&mut self.pending_change, &mut session.pending_change);
        swap(&mut self.pending_save, &mut session.pending_save);
        swap(&mut self.opened, &mut session.document_opened);
        swap(&mut self.diagnostics, &mut session.diagnostics);
        swap(&mut self.saved_compiler, &mut session.saved_compiler);
    }

    fn retained_text_bytes(&self) -> usize {
        self.snapshot
            .len_bytes()
            .saturating_add(self.synced_snapshot.len_bytes())
    }

    fn closing_text_reservation(&self) -> usize {
        if needs_closing_text(self.opened, self.pending_change, self.pending_save.as_ref()) {
            snapshot_reservation(self.snapshot.len_bytes(), self.synced_snapshot.len_bytes())
        } else {
            0
        }
    }
}

pub(super) fn validate_input(input: &RustDocumentInput) -> Result<(), RustDiagnosticsError> {
    if input.snapshot.len_bytes() > MAX_DOCUMENT_BYTES {
        return Err(RustDiagnosticsError::Language(
            LanguageProtocolError::DocumentTooLarge,
        ));
    }
    Ok(())
}

// Reserve the larger snapshot twice: a pending writer can replace the synced
// snapshot with the current one before another foreground admission. These
// logical bytes conservatively count shared COW text, not allocator residency.
fn snapshot_reservation(current: usize, synced: usize) -> usize {
    current.max(synced).saturating_mul(2)
}

pub(super) fn overlay_growth(current: usize, synced: usize, next: usize) -> usize {
    next.saturating_sub(current.max(synced))
}

// A document owner is an ID/path pair inside an already validated workspace.
// Edit and view revisions change within that owner; either key changing retires it.
fn matches_document_owner(document_id: u64, path: &Path, input: &RustDocumentInput) -> bool {
    document_id == input.identity.document_id && path == input.path.as_path()
}

fn collect_workspace_inputs<I>(inputs: I) -> Result<Vec<RustDocumentInput>, RustDiagnosticsError>
where
    I: IntoIterator<Item = RustDocumentInput>,
{
    let mut documents: Vec<RustDocumentInput> = Vec::new();
    for input in inputs {
        if documents.len() == MAX_OVERLAY_DOCUMENTS
            || documents.iter().any(|other| {
                other.identity.document_id == input.identity.document_id || other.path == input.path
            })
        {
            return Err(RustDiagnosticsError::OverlayBudget);
        }
        validate_input(&input)?;
        if let Some(first) = documents.first()
            && (first.workspace_root != input.workspace_root
                || first.identity.workspace_id != input.identity.workspace_id
                || first.identity.workspace_revision != input.identity.workspace_revision)
        {
            return Err(RustDiagnosticsError::InvalidIdentity);
        }
        documents
            .try_reserve(1)
            .map_err(|_| RustDiagnosticsError::OverlayBudget)?;
        documents.push(input);
    }
    Ok(documents)
}

impl RustSession {
    fn overlay_contents_match(&self, inputs: &[RustDocumentInput]) -> bool {
        self.parked.len() + 1 == inputs.len()
            && inputs.iter().all(|input| {
                self.matches_workspace(input)
                    && if matches_document_owner(
                        self.identity.document_id,
                        &self.target.path,
                        input,
                    ) {
                        self.identity.buffer_revision == input.identity.buffer_revision
                    } else {
                        self.parked.iter().any(|document| {
                            matches_document_owner(
                                document.identity.document_id,
                                &document.target.path,
                                input,
                            ) && document.identity.buffer_revision == input.identity.buffer_revision
                        })
                    }
            })
    }

    pub(super) fn clear_workspace_diagnostics(&mut self) -> bool {
        let mut changed = self.diagnostics.take().is_some();
        for document in &mut self.parked {
            changed |= document.diagnostics.take().is_some();
        }
        changed
    }

    fn matches_workspace(&self, input: &RustDocumentInput) -> bool {
        self.target.workspace_root == input.workspace_root
            && self.identity.workspace_id == input.identity.workspace_id
            && self.identity.workspace_revision == input.identity.workspace_revision
    }

    fn prospective_reservation(&self, input: &RustDocumentInput) -> usize {
        let synced = if matches_document_owner(self.identity.document_id, &self.target.path, input)
        {
            self.synced_snapshot.len_bytes()
        } else {
            self.parked
                .iter()
                .find(|document| {
                    matches_document_owner(
                        document.identity.document_id,
                        &document.target.path,
                        input,
                    )
                })
                .map_or(input.snapshot.len_bytes(), |document| {
                    document.synced_snapshot.len_bytes()
                })
        };
        snapshot_reservation(input.snapshot.len_bytes(), synced)
    }

    fn check_replacement_budget(
        &self,
        released: usize,
        replacement: usize,
    ) -> Result<(), RustDiagnosticsError> {
        if self
            .reserved_overlay_text_bytes()
            .saturating_sub(released)
            .saturating_add(replacement)
            > MAX_OVERLAY_RETAINED_TEXT_BYTES
        {
            return Err(RustDiagnosticsError::OverlayBudget);
        }
        Ok(())
    }

    pub(super) fn retained_overlay_text_bytes(&self) -> usize {
        self.parked
            .iter()
            .fold(
                self.snapshot
                    .len_bytes()
                    .saturating_add(self.synced_snapshot.len_bytes()),
                |total, document| total.saturating_add(document.retained_text_bytes()),
            )
            .saturating_add(self.overlay_closes.iter().fold(0_usize, |total, document| {
                total.saturating_add(document.retained_text_bytes())
            }))
    }

    pub(super) fn check_overlay_budget(
        &self,
        additional: usize,
    ) -> Result<(), RustDiagnosticsError> {
        if self
            .reserved_overlay_text_bytes()
            .saturating_add(additional.saturating_mul(2))
            > MAX_OVERLAY_RETAINED_TEXT_BYTES
        {
            return Err(RustDiagnosticsError::OverlayBudget);
        }
        Ok(())
    }

    pub(super) fn reserved_overlay_text_bytes(&self) -> usize {
        self.parked
            .iter()
            .fold(
                snapshot_reservation(self.snapshot.len_bytes(), self.synced_snapshot.len_bytes()),
                |total, document| {
                    total.saturating_add(snapshot_reservation(
                        document.snapshot.len_bytes(),
                        document.synced_snapshot.len_bytes(),
                    ))
                },
            )
            .saturating_add(self.closed_text_reservation())
    }

    fn closed_text_reservation(&self) -> usize {
        self.overlay_closes.iter().fold(0_usize, |total, document| {
            total.saturating_add(document.reserved_text_bytes())
        })
    }

    fn active_closing_text_reservation(&self) -> usize {
        if needs_closing_text(
            self.document_opened,
            self.pending_change,
            self.pending_save.as_ref(),
        ) {
            snapshot_reservation(self.snapshot.len_bytes(), self.synced_snapshot.len_bytes())
        } else {
            0
        }
    }

    fn prospective_closed_text_reservation(&self, inputs: &[RustDocumentInput]) -> usize {
        let retained = |identity: LanguageIdentity, target: &Target| {
            inputs
                .iter()
                .any(|input| matches_document_owner(identity.document_id, &target.path, input))
        };
        let mut reserved = self.closed_text_reservation();
        if !retained(self.identity, &self.target) {
            reserved = reserved.saturating_add(self.active_closing_text_reservation());
        }
        for document in &self.parked {
            if !retained(document.identity, &document.target) {
                reserved = reserved.saturating_add(document.closing_text_reservation());
            }
        }
        reserved
    }

    pub(super) fn workspace_ready(&self) -> bool {
        self.active_view
            && self.state == SessionState::Open
            && self.document_opened
            && !self.pending_change
            && self.overlay_write.is_none()
            && self.overlay_closes.is_empty()
            && self
                .parked
                .iter()
                .all(|document| document.opened && !document.pending_change)
    }

    pub(super) fn reset_overlay_transport(&mut self) {
        self.clear_saved_compiler();
        retry_save(&mut self.pending_save);
        self.diagnostic_pull = super::diagnostic_pull::PullState::default();
        self.overlay_write = None;
        // These close/save intents belong to the retired process. Do not
        // resurrect discarded tabs as overlays in the replacement process.
        self.overlay_closes.clear();
        self.document_opened = false;
        self.pending_change = false;
        for document in &mut self.parked {
            document.opened = false;
            document.pending_change = false;
            document.diagnostics = None;
            retry_save(&mut document.pending_save);
        }
    }

    pub(super) fn acknowledge_overlay(&mut self, sequence: InputSequence) -> bool {
        if self.overlay_write == Some(sequence) {
            self.overlay_write = None;
            acknowledge_save(&mut self.pending_save, sequence);
            for document in &mut self.parked {
                acknowledge_save(&mut document.pending_save, sequence);
            }
            for document in &mut self.overlay_closes {
                acknowledge_save(&mut document.pending_save, sequence);
            }
            return true;
        }
        false
    }

    pub(super) fn park_and_activate(
        &mut self,
        input: RustDocumentInput,
        retain_previous: bool,
    ) -> Result<(), RustDiagnosticsError> {
        validate_input(&input)?;
        let released = if retain_previous {
            0
        } else {
            snapshot_reservation(self.snapshot.len_bytes(), self.synced_snapshot.len_bytes())
                .saturating_sub(self.active_closing_text_reservation())
        };
        if !retain_previous && (self.document_opened || self.pending_save.is_some()) {
            if self.overlay_closes.len() == MAX_OVERLAY_DOCUMENTS {
                return Err(RustDiagnosticsError::OverlayBudget);
            }
            self.overlay_closes
                .try_reserve(1)
                .map_err(|_| RustDiagnosticsError::OverlayBudget)?;
        }
        let index = self.parked.iter().position(|document| {
            matches_document_owner(document.identity.document_id, &document.target.path, &input)
        });
        let mut next = if let Some(index) = index {
            let previous = &self.parked[index];
            self.check_replacement_budget(
                released.saturating_add(snapshot_reservation(
                    previous.snapshot.len_bytes(),
                    previous.synced_snapshot.len_bytes(),
                )),
                snapshot_reservation(
                    input.snapshot.len_bytes(),
                    previous.synced_snapshot.len_bytes(),
                ),
            )?;
            // Validate version arithmetic before moving the existing owner.
            if self.parked[index].identity.buffer_revision != input.identity.buffer_revision
                && self.parked[index].lsp_version == i32::MAX
            {
                return Err(RustDiagnosticsError::VersionExhausted);
            }
            let mut document = self.parked.remove(index);
            let _ = document.update(input)?;
            document
        } else {
            if self.parked.len() + usize::from(retain_previous) >= MAX_OVERLAY_DOCUMENTS {
                return Err(RustDiagnosticsError::OverlayBudget);
            }
            self.check_replacement_budget(
                released,
                snapshot_reservation(input.snapshot.len_bytes(), input.snapshot.len_bytes()),
            )?;
            if retain_previous {
                self.parked
                    .try_reserve(1)
                    .map_err(|_| RustDiagnosticsError::OverlayBudget)?;
            }
            ParkedDocument::new(input)?
        };
        next.swap_active(self);
        if retain_previous {
            self.parked.push(next);
        } else if next.opened || next.pending_save.is_some() {
            self.overlay_closes.push(ClosingDocument::from_parked(next));
        }
        if let Some(diagnostics) = self.diagnostics.as_mut() {
            diagnostics.identity = self.identity;
        }
        self.active_view = true;
        Ok(())
    }

    fn update_parked(&mut self, input: RustDocumentInput) -> Result<bool, RustDiagnosticsError> {
        if let Some(index) = self.parked.iter().position(|document| {
            matches_document_owner(document.identity.document_id, &document.target.path, &input)
        }) {
            self.check_overlay_budget(overlay_growth(
                self.parked[index].snapshot.len_bytes(),
                self.parked[index].synced_snapshot.len_bytes(),
                input.snapshot.len_bytes(),
            ))?;
            return self.parked[index].update(input);
        }
        if self.parked.len() + 1 >= MAX_OVERLAY_DOCUMENTS {
            return Err(RustDiagnosticsError::OverlayBudget);
        }
        self.check_overlay_budget(input.snapshot.len_bytes())?;
        self.parked
            .try_reserve(1)
            .map_err(|_| RustDiagnosticsError::OverlayBudget)?;
        self.parked.push(ParkedDocument::new(input)?);
        Ok(true)
    }

    fn retain_overlays(&mut self, retained: &[u64]) -> Result<(), RustDiagnosticsError> {
        self.retain_overlay_owners(|document| retained.contains(&document.identity.document_id))
    }

    fn retain_roster(&mut self, inputs: &[RustDocumentInput]) -> Result<(), RustDiagnosticsError> {
        self.retain_overlay_owners(|document| {
            inputs.iter().any(|input| {
                matches_document_owner(document.identity.document_id, &document.target.path, input)
            })
        })
    }

    fn retain_overlay_owners(
        &mut self,
        retained: impl Fn(&ParkedDocument) -> bool,
    ) -> Result<(), RustDiagnosticsError> {
        let closing = self
            .parked
            .iter()
            .filter(|document| {
                (document.opened || document.pending_save.is_some()) && !retained(document)
            })
            .count();
        if self.overlay_closes.len().saturating_add(closing) > MAX_OVERLAY_DOCUMENTS {
            return Err(RustDiagnosticsError::OverlayBudget);
        }
        self.overlay_closes
            .try_reserve(closing)
            .map_err(|_| RustDiagnosticsError::OverlayBudget)?;
        let mut index = 0;
        while index < self.parked.len() {
            if retained(&self.parked[index]) {
                index += 1;
            } else {
                let document = self.parked.remove(index);
                if document.opened || document.pending_save.is_some() {
                    self.overlay_closes
                        .push(ClosingDocument::from_parked(document));
                }
            }
        }
        Ok(())
    }

    // At most one overlay notification is awaiting the writer. A later update
    // coalesces into its COW snapshot; it cannot overtake an earlier wire write.
    pub(super) fn flush_overlay(&mut self) -> Result<bool, RustDiagnosticsError> {
        if self.state != SessionState::Open {
            return Ok(false);
        }
        match self.flush_closing_document()? {
            ClosingFlush::Empty => {}
            ClosingFlush::Busy => return Ok(false),
            ClosingFlush::Sent => return Ok(true),
        }
        if !self.document_opened || self.pending_change {
            let (method, params) = document_message(
                &self.document,
                &self.snapshot,
                &self.synced_snapshot,
                self.document_opened,
            )?;
            let sequence = self
                .client
                .notify(method, Some(&params))
                .map_err(RustDiagnosticsError::Client)?;
            self.overlay_write = Some(sequence);
            self.document_opened = true;
            self.synced_snapshot = self.snapshot.clone();
            self.pending_change = false;
            return Ok(true);
        }
        if let Some(document) = self
            .parked
            .iter_mut()
            .find(|document| !document.opened || document.pending_change)
        {
            let (method, params) = document_message(
                &document.document,
                &document.snapshot,
                &document.synced_snapshot,
                document.opened,
            )?;
            let sequence = self
                .client
                .notify(method, Some(&params))
                .map_err(RustDiagnosticsError::Client)?;
            self.overlay_write = Some(sequence);
            document.opened = true;
            document.synced_snapshot = document.snapshot.clone();
            document.pending_change = false;
            return Ok(true);
        }
        self.flush_saved_notification()
    }

    fn flush_closing_document(&mut self) -> Result<ClosingFlush, RustDiagnosticsError> {
        if self.overlay_write.is_some() {
            return Ok(ClosingFlush::Busy);
        }
        self.check_overlay_budget(0)?;
        let Some(document) = self.overlay_closes.first_mut() else {
            return Ok(ClosingFlush::Empty);
        };
        if let Some((current, synced)) = document.pending_text.as_ref() {
            let (method, params) =
                document_message(&document.document, current, synced, document.opened)?;
            let sequence = self
                .client
                .notify(method, Some(&params))
                .map_err(RustDiagnosticsError::Client)?;
            self.overlay_write = Some(sequence);
            document.opened = true;
            // The bounded process writer now owns the encoded payload. The
            // removed editor's snapshots are no longer needed by this owner.
            document.pending_text = None;
            return Ok(ClosingFlush::Sent);
        }
        if let Some(save) = document.pending_save.as_mut() {
            // A closing document cannot receive a later overlay revision.
            // This URI-only event reports its successful disk save, never the
            // authority or freshness of an in-memory text snapshot.
            let params = document
                .document
                .text_document_params()
                .map_err(RustDiagnosticsError::Language)?;
            let sequence = self
                .client
                .notify("textDocument/didSave", Some(&params))
                .map_err(RustDiagnosticsError::Client)?;
            save.submitted = Some(sequence);
            self.overlay_write = Some(sequence);
            return Ok(ClosingFlush::Sent);
        }
        let params = document
            .document
            .did_close_params()
            .map_err(RustDiagnosticsError::Language)?;
        let sequence = self
            .client
            .notify("textDocument/didClose", Some(&params))
            .map_err(RustDiagnosticsError::Client)?;
        self.overlay_write = Some(sequence);
        self.overlay_closes.remove(0);
        Ok(ClosingFlush::Sent)
    }

    fn flush_saved_notification(&mut self) -> Result<bool, RustDiagnosticsError> {
        let next = std::iter::once((
            &self.document,
            self.identity.buffer_revision,
            &mut self.pending_save,
        ))
        .chain(self.parked.iter_mut().map(|document| {
            (
                &document.document,
                document.identity.buffer_revision,
                &mut document.pending_save,
            )
        }))
        .find_map(|(document, revision, pending)| {
            pending
                .as_mut()
                .filter(|save| save.submitted.is_none() && save.buffer_revision <= revision)
                .map(|save| (document, save))
        });
        let Some((document, pending)) = next else {
            return Ok(false);
        };
        // URI-only didSave describes a successful disk save. A newer unsaved
        // overlay may already exist, so never attach its text or version here.
        let params = document
            .text_document_params()
            .map_err(RustDiagnosticsError::Language)?;
        let sequence = self
            .client
            .notify("textDocument/didSave", Some(&params))
            .map_err(RustDiagnosticsError::Client)?;
        pending.submitted = Some(sequence);
        self.overlay_write = Some(sequence);
        Ok(true)
    }
}

fn document_message(
    document: &LspDocument,
    snapshot: &BufferSnapshot,
    previous: &BufferSnapshot,
    opened: bool,
) -> Result<(&'static str, Box<serde_json::value::RawValue>), RustDiagnosticsError> {
    if snapshot.len_bytes() > MAX_DOCUMENT_BYTES {
        return Err(RustDiagnosticsError::Language(
            LanguageProtocolError::DocumentTooLarge,
        ));
    }
    let text = snapshot.text();
    if opened {
        let end = document_end(previous).map_err(RustDiagnosticsError::Language)?;
        Ok((
            "textDocument/didChange",
            document
                .did_change_params(&text, end)
                .map_err(RustDiagnosticsError::Language)?,
        ))
    } else {
        Ok((
            "textDocument/didOpen",
            document
                .did_open_params(&text)
                .map_err(RustDiagnosticsError::Language)?,
        ))
    }
}

impl RustDiagnostics {
    #[cfg(test)]
    pub(crate) fn initialize_installed_transport_for_test(
        &mut self,
    ) -> Result<(), RustDiagnosticsError> {
        let session = self
            .session
            .as_mut()
            .ok_or(RustDiagnosticsError::InvalidIdentity)?;
        session.client.initialize_inert_for_test();
        Ok(())
    }

    pub(crate) fn record_saved_document(&mut self, identity: LanguageIdentity) -> LanguageEffect {
        let Some(session) = self.session.as_mut() else {
            return LanguageEffect::default();
        };
        if session.identity.workspace_id != identity.workspace_id
            || session.identity.workspace_revision != identity.workspace_revision
        {
            return LanguageEffect::default();
        }
        let owned = if session.identity.document_id == identity.document_id {
            Some((session.identity.buffer_revision, &mut session.pending_save))
        } else {
            session
                .parked
                .iter_mut()
                .find(|document| document.identity.document_id == identity.document_id)
                .map(|document| {
                    (
                        document.identity.buffer_revision,
                        &mut document.pending_save,
                    )
                })
        };
        let Some((revision, pending)) = owned else {
            return LanguageEffect::default();
        };
        if identity.buffer_revision < revision
            || pending
                .as_ref()
                .is_some_and(|save| save.buffer_revision > identity.buffer_revision)
        {
            return LanguageEffect::default();
        }
        *pending = Some(PendingSave {
            buffer_revision: identity.buffer_revision,
            submitted: None,
        });
        LanguageEffect {
            visual_changed: self.flush_change(),
            continuation: None,
        }
    }

    pub(crate) fn workspace_root(&self) -> Option<&Path> {
        self.target
            .as_ref()
            .map(|target| target.workspace_root.as_path())
    }

    pub(crate) fn reject_workspace(&mut self, error: RustDiagnosticsError) -> LanguageEffect {
        // Preserve an existing rejection status when there is no owned server
        // to stop. Clearing it on each retry turns a stable limit into redraws.
        let stopped = self.session.is_some() && self.stop();
        self.target = None;
        // A later admissible workspace may retry. Do not turn a temporary
        // overlay limit into the sticky missing-executable failure policy.
        let mut effect = self.fail(error);
        effect.visual_changed |= stopped;
        effect
    }

    fn preflight_workspace(
        &self,
        inputs: &[RustDocumentInput],
    ) -> Result<(), RustDiagnosticsError> {
        let mut reserved = self
            .session
            .as_ref()
            .filter(|session| {
                inputs
                    .first()
                    .is_some_and(|input| session.matches_workspace(input))
            })
            .map_or(0, |session| {
                session.prospective_closed_text_reservation(inputs)
            });
        for input in inputs {
            let bytes = self
                .session
                .as_ref()
                .filter(|session| session.matches_workspace(input))
                .map_or_else(
                    || snapshot_reservation(input.snapshot.len_bytes(), input.snapshot.len_bytes()),
                    |session| session.prospective_reservation(input),
                );
            reserved = reserved.saturating_add(bytes);
            if reserved > MAX_OVERLAY_RETAINED_TEXT_BYTES {
                return Err(RustDiagnosticsError::OverlayBudget);
            }
        }
        Ok(())
    }

    pub(super) fn cancel_view_requests(&mut self) -> bool {
        let mut changed = self.workspace_edit_preparation.take().is_some();
        let Some(session) = self.session.as_mut() else {
            return changed;
        };
        changed |= session.completion.take().is_some();
        changed |= session.navigation.take().is_some();
        changed |= session.symbols.take().is_some();
        // These counters describe local invalidation, not successful wire
        // cancellation. A failed cancellation must still revoke publication.
        let requests = [
            (
                session
                    .pending_completion
                    .take()
                    .map(|pending| pending.request_id),
                &mut self.completion_cancellations,
            ),
            (
                session
                    .pending_navigation
                    .take()
                    .map(|pending| pending.request_id),
                &mut self.navigation_cancellations,
            ),
            (
                session
                    .pending_symbols
                    .take()
                    .map(|pending| pending.request_id),
                &mut self.symbol_cancellations,
            ),
            (
                session
                    .pending_workspace_edit
                    .take()
                    .map(|pending| pending.request_id),
                &mut self.workspace_edit_cancellations,
            ),
        ];
        for (request, counter) in requests {
            if let Some(request) = request {
                *counter = counter.saturating_add(1);
                changed = true;
                if let Err(error) = session.client.cancel(request) {
                    let _ = replace_status(
                        &mut self.status,
                        Some(Arc::from(RustDiagnosticsError::Client(error).to_string())),
                    );
                }
            }
        }
        changed
    }

    pub(super) fn deactivate_view(&mut self) -> LanguageEffect {
        let mut changed = self.cancel_view_requests();
        if let Some(session) = self.session.as_mut() {
            changed |= session.active_view;
            session.active_view = false;
        }
        LanguageEffect {
            visual_changed: changed,
            continuation: None,
        }
    }

    pub(crate) fn sync_workspace<I, F>(
        &mut self,
        inputs: I,
        active_document: Option<u64>,
        wake_factory: F,
    ) -> LanguageEffect
    where
        I: IntoIterator<Item = RustDocumentInput>,
        F: FnOnce(LanguageWake) -> ProcessWake,
    {
        let mut documents = match collect_workspace_inputs(inputs) {
            Ok(documents) => documents,
            Err(error) => return self.reject_workspace(error),
        };
        if let Err(error) = self.preflight_workspace(&documents) {
            return self.reject_workspace(error);
        }
        if documents.is_empty() {
            return LanguageEffect {
                visual_changed: self.stop(),
                continuation: None,
            };
        }
        let mut diagnostics_invalidated = false;
        if let Some(session) = self.session.as_mut()
            && !session.overlay_contents_match(&documents)
        {
            match session.invalidate_workspace_diagnostics() {
                Ok(changed) => diagnostics_invalidated = changed,
                Err(error) => return self.reject_workspace(error),
            }
        }
        let requested =
            active_document.or_else(|| self.session.as_ref().map(|s| s.identity.document_id));
        let index = documents
            .iter()
            .position(|input| Some(input.identity.document_id) == requested);
        if active_document.is_some() && index.is_none() {
            return self.reject_workspace(RustDiagnosticsError::InvalidIdentity);
        }
        let mut retained = [0_u64; MAX_OVERLAY_DOCUMENTS];
        for (index, input) in documents.iter().enumerate() {
            retained[index] = input.identity.document_id;
        }
        let retained_count = documents.len();
        let anchor_index = index.unwrap_or(0);
        let retain_previous = self.session.as_ref().is_none_or(|session| {
            documents.iter().any(|input| {
                matches_document_owner(session.identity.document_id, &session.target.path, input)
            })
        });
        if let Some(session) = self.session.as_mut()
            && session.matches_workspace(&documents[anchor_index])
            && let Err(error) = session.retain_roster(&documents)
        {
            return self.reject_workspace(error);
        }
        let anchor = documents.remove(anchor_index);
        // Reconcile the entire roster before a new didOpen can overtake a
        // didClose for the same URI. An inactive view must not be reactivated
        // on every otherwise unchanged foreground reconciliation.
        let mut effect = self.sync_document(
            Some(anchor),
            wake_factory,
            active_document.is_some(),
            true,
            retain_previous,
        );
        effect.visual_changed |= diagnostics_invalidated;
        let mut changed = false;
        let result = if let Some(session) = self.session.as_mut() {
            (|| {
                session.retain_overlays(&retained[..retained_count])?;
                for input in documents {
                    changed |= session.update_parked(input)?;
                }
                Ok(())
            })()
        } else {
            return effect;
        };
        if let Err(error) = result {
            return self.reject_workspace(error);
        }
        if changed {
            effect.visual_changed |= self.cancel_view_requests();
        }
        if active_document.is_none() {
            effect.visual_changed |= self.deactivate_view().visual_changed;
        }
        effect.visual_changed |= self.flush_change();
        effect.visual_changed |= self.pump_diagnostics();
        effect
    }
}

#[cfg(test)]
#[path = "rust_workspace_tests.rs"]
mod tests;

pub(super) fn saved_compiler_counts(
    current: Option<&super::saved_compiler::SavedCompilerReport>,
    parked: &[ParkedDocument],
) -> (usize, usize) {
    current
        .into_iter()
        .chain(
            parked
                .iter()
                .filter_map(|document| document.saved_compiler.as_ref()),
        )
        .fold((0, 0), |(items, bytes), report| {
            (items + report.items(), bytes + report.retained_bytes())
        })
}

pub(super) fn route_saved_compiler(
    document: &LspDocument,
    active_view: bool,
    current: &mut Option<super::saved_compiler::SavedCompilerReport>,
    parked: &mut [ParkedDocument],
    report: super::saved_compiler::SavedCompilerReport,
) -> Result<bool, LanguageProtocolError> {
    let (items, bytes) = saved_compiler_counts(current.as_ref(), parked);
    let active = document.uri() == report.uri();
    let slot = if active {
        current
    } else {
        &mut parked
            .iter_mut()
            .find(|document| document.document.uri() == report.uri())
            .ok_or(LanguageProtocolError::DocumentMismatch)?
            .saved_compiler
    };
    let old_items = slot
        .as_ref()
        .map_or(0, super::saved_compiler::SavedCompilerReport::items);
    let old_bytes = slot.as_ref().map_or(
        0,
        super::saved_compiler::SavedCompilerReport::retained_bytes,
    );
    let added_bytes = if report.items() == 0 {
        0
    } else {
        report.retained_bytes()
    };
    if items - old_items + report.items() > super::saved_compiler::MAX_WORKSPACE_ITEMS
        || bytes - old_bytes + added_bytes > super::saved_compiler::MAX_WORKSPACE_BYTES
    {
        return Err(LanguageProtocolError::DiagnosticRetentionExceeded);
    }
    let replacement = (report.items() != 0).then_some(report);
    let changed = *slot != replacement;
    *slot = replacement;
    Ok(changed && active && active_view)
}

impl RustSession {
    pub(super) fn clear_saved_compiler(&mut self) {
        self.saved_compiler = None;
        for document in &mut self.parked {
            document.saved_compiler = None;
        }
    }
}
