use core::{error::Error, fmt};

use crate::FrameToken;

/// Exact number of reusable presentation frame slots.
pub const FRAME_SLOT_COUNT: usize = 3;

/// Monotonic identity of one native presentation owner generation.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct FrameOwnerGeneration(u64);

impl FrameOwnerGeneration {
    /// Creates a nonzero owner generation.
    #[must_use]
    pub const fn new(value: u64) -> Option<Self> {
        if value == 0 { None } else { Some(Self(value)) }
    }

    /// Returns the nonzero generation identity.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Stable index of one of the three frame slots.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct FrameSlotId(u8);

impl FrameSlotId {
    /// Returns the zero-based slot index.
    #[must_use]
    pub const fn get(self) -> u8 {
        self.0
    }

    fn index(self) -> usize {
        usize::from(self.0)
    }
}

/// Opaque identity that prevents stale completion from reusing a slot.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct FrameSlotLease {
    slot: FrameSlotId,
    sequence: u64,
    generation: FrameOwnerGeneration,
    token: FrameToken,
}

impl FrameSlotLease {
    /// Returns the selected frame slot.
    #[must_use]
    pub const fn slot(self) -> FrameSlotId {
        self.slot
    }

    /// Returns the monotonic slot-admission sequence.
    #[must_use]
    pub const fn sequence(self) -> u64 {
        self.sequence
    }

    /// Returns the owner generation captured at admission.
    #[must_use]
    pub const fn generation(self) -> FrameOwnerGeneration {
        self.generation
    }

    /// Returns the portable frame token captured at admission.
    #[must_use]
    pub const fn token(self) -> FrameToken {
        self.token
    }
}

/// Native ownership phase represented by one frame slot.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum FrameSlotPhase {
    /// The slot owns no frame resources.
    Free,
    /// The callback owns a drawable and may encode but has not committed.
    Encoding,
    /// One command buffer and its resources remain retained until completion.
    Submitted,
}

/// Terminal command status supplied by the native completion boundary.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum FrameCompletionStatus {
    /// GPU execution reached successful terminal completion.
    Completed,
    /// GPU execution reached a classified terminal failure.
    Failed,
    /// Shutdown classified committed work as cancelled after it drained.
    Cancelled,
}

/// Main-thread publication decision for one released slot.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum FrameCompletionDisposition {
    /// Completion matches the current generation, revision, and surface epoch.
    Current,
    /// Completion belongs to an invalidated native owner generation.
    StaleGeneration,
    /// Completion belongs to an older requested scene revision.
    SupersededRevision,
    /// Completion belongs to an older surface configuration epoch.
    SupersededEpoch,
    /// GPU execution reached a terminal failure.
    Failed,
    /// Shutdown cancelled publication after committed work drained.
    Cancelled,
}

/// Handle-free terminal result returned after a submitted slot is released.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameSlotCompletion {
    lease: FrameSlotLease,
    status: FrameCompletionStatus,
    disposition: FrameCompletionDisposition,
}

impl FrameSlotCompletion {
    /// Returns the exact released lease.
    #[must_use]
    pub const fn lease(self) -> FrameSlotLease {
        self.lease
    }

    /// Returns the native terminal command status.
    #[must_use]
    pub const fn status(self) -> FrameCompletionStatus {
        self.status
    }

    /// Returns whether main-thread publication is current or rejected.
    #[must_use]
    pub const fn disposition(self) -> FrameCompletionDisposition {
        self.disposition
    }
}

/// Result of attempting to acquire one bounded frame slot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FrameSlotAdmission {
    /// One free slot became exclusively owned by this lease.
    Acquired(FrameSlotLease),
    /// All three slots remain owned; the newest work must stay coalesced.
    Saturated,
}

/// Stable rejection category for frame-slot state transitions.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum FrameSlotErrorKind {
    /// An arithmetic identity or accounting counter cannot advance.
    SequenceExhausted,
    /// The supplied lease does not identify the slot's current owner.
    LeaseMismatch,
    /// The requested transition is disabled in the slot's current phase.
    ActionDisabled,
    /// The ring did not satisfy its executable ownership invariants.
    InvariantViolation,
}

/// Structured frame-slot transition failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameSlotError {
    kind: FrameSlotErrorKind,
    slot: Option<FrameSlotId>,
}

impl FrameSlotError {
    /// Returns the stable rejection category.
    #[must_use]
    pub const fn kind(self) -> FrameSlotErrorKind {
        self.kind
    }

    /// Returns the referenced slot when one was supplied.
    #[must_use]
    pub const fn slot(self) -> Option<FrameSlotId> {
        self.slot
    }
}

impl fmt::Display for FrameSlotError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "frame-slot transition rejected for {:?}: {:?}",
            self.slot, self.kind
        )
    }
}

impl Error for FrameSlotError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FrameSlot {
    phase: FrameSlotPhase,
    lease: Option<FrameSlotLease>,
}

impl FrameSlot {
    const FREE: Self = Self {
        phase: FrameSlotPhase::Free,
        lease: None,
    };
}

/// Handle-free accounting snapshot for the bounded frame-slot ring.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameSlotSnapshot {
    occupied_slots: u8,
    submitted_slots: u8,
    peak_occupied_slots: u8,
    admission_count: u64,
    release_count: u64,
    completion_count: u64,
    failure_count: u64,
    cancellation_count: u64,
    saturation_count: u64,
}

impl FrameSlotSnapshot {
    /// Returns the fixed slot capacity.
    #[must_use]
    pub const fn capacity(self) -> u8 {
        3
    }

    /// Returns slots owning encoding or submitted work.
    #[must_use]
    pub const fn occupied_slots(self) -> u8 {
        self.occupied_slots
    }

    /// Returns slots retaining committed work.
    #[must_use]
    pub const fn submitted_slots(self) -> u8 {
        self.submitted_slots
    }

    /// Returns the largest observed occupied-slot count.
    #[must_use]
    pub const fn peak_occupied_slots(self) -> u8 {
        self.peak_occupied_slots
    }

    /// Returns successful slot admissions.
    #[must_use]
    pub const fn admission_count(self) -> u64 {
        self.admission_count
    }

    /// Returns all terminal slot releases.
    #[must_use]
    pub const fn release_count(self) -> u64 {
        self.release_count
    }

    /// Returns successful GPU completion releases, including stale results.
    #[must_use]
    pub const fn completion_count(self) -> u64 {
        self.completion_count
    }

    /// Returns terminal GPU failure releases.
    #[must_use]
    pub const fn failure_count(self) -> u64 {
        self.failure_count
    }

    /// Returns precommit or drained-shutdown cancellations.
    #[must_use]
    pub const fn cancellation_count(self) -> u64 {
        self.cancellation_count
    }

    /// Returns valid admissions omitted because every slot was occupied.
    #[must_use]
    pub const fn saturation_count(self) -> u64 {
        self.saturation_count
    }
}

/// Allocation-free ownership and ABA guard for exactly three frame slots.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameSlotRing {
    slots: [FrameSlot; FRAME_SLOT_COUNT],
    next_sequence: u64,
    admissions: u64,
    releases: u64,
    completions: u64,
    failures: u64,
    cancellations: u64,
    saturations: u64,
    peak_occupied: u8,
}

impl Default for FrameSlotRing {
    fn default() -> Self {
        Self::new()
    }
}

impl FrameSlotRing {
    /// Creates three free frame slots with zero accounting.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            slots: [FrameSlot::FREE; FRAME_SLOT_COUNT],
            next_sequence: 0,
            admissions: 0,
            releases: 0,
            completions: 0,
            failures: 0,
            cancellations: 0,
            saturations: 0,
            peak_occupied: 0,
        }
    }

    /// Acquires one free slot or records bounded queue saturation.
    ///
    /// # Errors
    ///
    /// Returns a sequence or invariant error without partial ownership change.
    pub fn acquire(
        &mut self,
        token: FrameToken,
        generation: FrameOwnerGeneration,
    ) -> Result<FrameSlotAdmission, FrameSlotError> {
        let before = *self;
        match self.acquire_inner(token, generation) {
            Ok(admission) if self.invariants_hold() => Ok(admission),
            Ok(_) => {
                *self = before;
                Err(Self::error(FrameSlotErrorKind::InvariantViolation, None))
            }
            Err(error) => {
                *self = before;
                Err(error)
            }
        }
    }

    /// Marks an encoding lease as committed and retained by Metal.
    ///
    /// # Errors
    ///
    /// Returns a phase, lease, or invariant error atomically.
    pub fn mark_submitted(&mut self, lease: FrameSlotLease) -> Result<(), FrameSlotError> {
        self.apply_atomic(Some(lease.slot), |ring| {
            let slot = ring.validate_lease(lease)?;
            if !matches!(slot.phase, FrameSlotPhase::Encoding) {
                return Err(Self::error(
                    FrameSlotErrorKind::ActionDisabled,
                    Some(lease.slot),
                ));
            }
            slot.phase = FrameSlotPhase::Submitted;
            Ok(())
        })
    }

    /// Releases an encoding lease that never committed native work.
    ///
    /// # Errors
    ///
    /// Returns a phase, lease, sequence, or invariant error atomically.
    pub fn cancel_encoding(&mut self, lease: FrameSlotLease) -> Result<(), FrameSlotError> {
        self.apply_atomic(Some(lease.slot), |ring| {
            if !matches!(ring.validate_lease(lease)?.phase, FrameSlotPhase::Encoding) {
                return Err(Self::error(
                    FrameSlotErrorKind::ActionDisabled,
                    Some(lease.slot),
                ));
            }
            ring.release(lease, FrameCompletionStatus::Cancelled)
        })
    }

    /// Releases submitted ownership and classifies main-thread publication.
    ///
    /// The slot is released for every terminal status. A successful command is
    /// publishable only when generation, revision, and epoch remain current.
    ///
    /// # Errors
    ///
    /// Returns a phase, lease, sequence, or invariant error atomically.
    pub fn complete(
        &mut self,
        lease: FrameSlotLease,
        status: FrameCompletionStatus,
        current_generation: FrameOwnerGeneration,
        current_revision: crate::PresentationRevision,
        current_epoch: crate::SurfaceEpoch,
    ) -> Result<FrameSlotCompletion, FrameSlotError> {
        self.apply_atomic(Some(lease.slot), |ring| {
            if !matches!(ring.validate_lease(lease)?.phase, FrameSlotPhase::Submitted) {
                return Err(Self::error(
                    FrameSlotErrorKind::ActionDisabled,
                    Some(lease.slot),
                ));
            }
            let disposition = match status {
                FrameCompletionStatus::Failed => FrameCompletionDisposition::Failed,
                FrameCompletionStatus::Cancelled => FrameCompletionDisposition::Cancelled,
                FrameCompletionStatus::Completed if lease.generation != current_generation => {
                    FrameCompletionDisposition::StaleGeneration
                }
                FrameCompletionStatus::Completed if lease.token.revision() != current_revision => {
                    FrameCompletionDisposition::SupersededRevision
                }
                FrameCompletionStatus::Completed if lease.token.epoch() != current_epoch => {
                    FrameCompletionDisposition::SupersededEpoch
                }
                FrameCompletionStatus::Completed => FrameCompletionDisposition::Current,
            };
            ring.release(lease, status)?;
            Ok(FrameSlotCompletion {
                lease,
                status,
                disposition,
            })
        })
    }

    /// Returns bounded occupancy and terminal accounting.
    #[must_use]
    pub fn snapshot(self) -> FrameSlotSnapshot {
        FrameSlotSnapshot {
            occupied_slots: self.occupied_slots(),
            submitted_slots: self.submitted_slots(),
            peak_occupied_slots: self.peak_occupied,
            admission_count: self.admissions,
            release_count: self.releases,
            completion_count: self.completions,
            failure_count: self.failures,
            cancellation_count: self.cancellations,
            saturation_count: self.saturations,
        }
    }

    /// Checks bounded ownership, unique leases, and balanced accounting.
    #[must_use]
    pub fn invariants_hold(self) -> bool {
        let occupied = self.occupied_slots();
        if usize::from(occupied) > FRAME_SLOT_COUNT || self.peak_occupied < occupied {
            return false;
        }
        let Some(expected_admissions) = self.releases.checked_add(u64::from(occupied)) else {
            return false;
        };
        let Some(terminal_releases) = self
            .completions
            .checked_add(self.failures)
            .and_then(|value| value.checked_add(self.cancellations))
        else {
            return false;
        };
        if expected_admissions != self.admissions || terminal_releases != self.releases {
            return false;
        }
        let mut left = 0;
        while left < FRAME_SLOT_COUNT {
            let slot = self.slots[left];
            if matches!(slot.phase, FrameSlotPhase::Free) != slot.lease.is_none() {
                return false;
            }
            if let Some(lease) = slot.lease {
                if lease.slot.index() != left
                    || lease.sequence == 0
                    || lease.sequence > self.next_sequence
                {
                    return false;
                }
                let mut right = left + 1;
                while right < FRAME_SLOT_COUNT {
                    if let Some(other) = self.slots[right].lease
                        && (other.sequence == lease.sequence || other.token == lease.token)
                    {
                        return false;
                    }
                    right += 1;
                }
            }
            left += 1;
        }
        true
    }

    fn acquire_inner(
        &mut self,
        token: FrameToken,
        generation: FrameOwnerGeneration,
    ) -> Result<FrameSlotAdmission, FrameSlotError> {
        if self
            .slots
            .iter()
            .any(|slot| slot.lease.is_some_and(|lease| lease.token == token))
        {
            return Err(Self::error(FrameSlotErrorKind::LeaseMismatch, None));
        }
        let Some(index) = self
            .slots
            .iter()
            .position(|slot| matches!(slot.phase, FrameSlotPhase::Free))
        else {
            self.saturations = self
                .saturations
                .checked_add(1)
                .ok_or_else(|| Self::error(FrameSlotErrorKind::SequenceExhausted, None))?;
            return Ok(FrameSlotAdmission::Saturated);
        };
        let sequence = self
            .next_sequence
            .checked_add(1)
            .ok_or_else(|| Self::error(FrameSlotErrorKind::SequenceExhausted, None))?;
        let admissions = self
            .admissions
            .checked_add(1)
            .ok_or_else(|| Self::error(FrameSlotErrorKind::SequenceExhausted, None))?;
        let slot_index = u8::try_from(index)
            .map_err(|_| Self::error(FrameSlotErrorKind::InvariantViolation, None))?;
        let lease = FrameSlotLease {
            slot: FrameSlotId(slot_index),
            sequence,
            generation,
            token,
        };
        self.next_sequence = sequence;
        self.admissions = admissions;
        self.slots[index] = FrameSlot {
            phase: FrameSlotPhase::Encoding,
            lease: Some(lease),
        };
        self.peak_occupied = self.peak_occupied.max(self.occupied_slots());
        Ok(FrameSlotAdmission::Acquired(lease))
    }

    fn apply_atomic<T>(
        &mut self,
        slot: Option<FrameSlotId>,
        action: impl FnOnce(&mut Self) -> Result<T, FrameSlotError>,
    ) -> Result<T, FrameSlotError> {
        let before = *self;
        match action(self) {
            Ok(value) if self.invariants_hold() => Ok(value),
            Ok(_) => {
                *self = before;
                Err(Self::error(FrameSlotErrorKind::InvariantViolation, slot))
            }
            Err(error) => {
                *self = before;
                Err(error)
            }
        }
    }

    fn validate_lease(&mut self, lease: FrameSlotLease) -> Result<&mut FrameSlot, FrameSlotError> {
        let Some(slot) = self.slots.get_mut(lease.slot.index()) else {
            return Err(Self::error(
                FrameSlotErrorKind::LeaseMismatch,
                Some(lease.slot),
            ));
        };
        if slot.lease != Some(lease) {
            return Err(Self::error(
                FrameSlotErrorKind::LeaseMismatch,
                Some(lease.slot),
            ));
        }
        Ok(slot)
    }

    fn release(
        &mut self,
        lease: FrameSlotLease,
        status: FrameCompletionStatus,
    ) -> Result<(), FrameSlotError> {
        let releases = self
            .releases
            .checked_add(1)
            .ok_or_else(|| Self::error(FrameSlotErrorKind::SequenceExhausted, Some(lease.slot)))?;
        let terminal_counter = match status {
            FrameCompletionStatus::Completed => &mut self.completions,
            FrameCompletionStatus::Failed => &mut self.failures,
            FrameCompletionStatus::Cancelled => &mut self.cancellations,
        };
        *terminal_counter = terminal_counter
            .checked_add(1)
            .ok_or_else(|| Self::error(FrameSlotErrorKind::SequenceExhausted, Some(lease.slot)))?;
        self.releases = releases;
        self.slots[lease.slot.index()] = FrameSlot::FREE;
        Ok(())
    }

    fn occupied_slots(self) -> u8 {
        let mut count = 0_u8;
        let mut index = 0;
        while index < FRAME_SLOT_COUNT {
            if !matches!(self.slots[index].phase, FrameSlotPhase::Free) {
                count += 1;
            }
            index += 1;
        }
        count
    }

    fn submitted_slots(self) -> u8 {
        let mut count = 0_u8;
        let mut index = 0;
        while index < FRAME_SLOT_COUNT {
            if matches!(self.slots[index].phase, FrameSlotPhase::Submitted) {
                count += 1;
            }
            index += 1;
        }
        count
    }

    const fn error(kind: FrameSlotErrorKind, slot: Option<FrameSlotId>) -> FrameSlotError {
        FrameSlotError { kind, slot }
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::{
        FrameCompletionDisposition, FrameCompletionStatus, FrameOwnerGeneration,
        FrameSlotAdmission, FrameSlotErrorKind, FrameSlotLease, FrameSlotPhase, FrameSlotRing,
    };
    use crate::{FrameToken, PresentationRevision, SurfaceEpoch};

    fn generation(value: u64) -> FrameOwnerGeneration {
        match FrameOwnerGeneration::new(value) {
            Some(generation) => generation,
            None => FrameOwnerGeneration(1),
        }
    }

    const fn token(attempt: u64, revision: u64, epoch: u64) -> FrameToken {
        FrameToken {
            attempt,
            revision: PresentationRevision(revision),
            epoch: SurfaceEpoch(epoch),
        }
    }

    fn acquire(
        ring: &mut FrameSlotRing,
        token: FrameToken,
    ) -> Result<FrameSlotLease, &'static str> {
        match ring
            .acquire(token, generation(1))
            .map_err(|_| "admission")?
        {
            FrameSlotAdmission::Acquired(lease) => Ok(lease),
            FrameSlotAdmission::Saturated => Err("unexpected saturation"),
        }
    }

    #[test]
    fn admission_is_bounded_and_saturation_is_observable() -> Result<(), &'static str> {
        let mut ring = FrameSlotRing::new();
        for attempt in 1..=3 {
            let lease = acquire(&mut ring, token(attempt, attempt, 1))?;
            ring.mark_submitted(lease).map_err(|_| "submit")?;
        }
        assert_eq!(
            ring.acquire(token(4, 4, 1), generation(1))
                .map_err(|_| "saturation")?,
            FrameSlotAdmission::Saturated
        );
        let snapshot = ring.snapshot();
        assert_eq!(snapshot.capacity(), 3);
        assert_eq!(snapshot.occupied_slots(), 3);
        assert_eq!(snapshot.submitted_slots(), 3);
        assert_eq!(snapshot.peak_occupied_slots(), 3);
        assert_eq!(snapshot.saturation_count(), 1);
        assert!(ring.invariants_hold());
        Ok(())
    }

    #[test]
    fn completion_reordering_releases_exact_slots_and_rejects_aba() -> Result<(), &'static str> {
        let mut ring = FrameSlotRing::new();
        let first = acquire(&mut ring, token(1, 1, 1))?;
        let second = acquire(&mut ring, token(2, 2, 1))?;
        let third = acquire(&mut ring, token(3, 3, 1))?;
        for lease in [first, second, third] {
            ring.mark_submitted(lease).map_err(|_| "submit")?;
        }
        let reordered = ring
            .complete(
                second,
                FrameCompletionStatus::Completed,
                generation(1),
                PresentationRevision(3),
                SurfaceEpoch(1),
            )
            .map_err(|_| "complete second")?;
        assert_eq!(
            reordered.disposition(),
            FrameCompletionDisposition::SupersededRevision
        );
        let replacement = acquire(&mut ring, token(4, 4, 1))?;
        assert_eq!(replacement.slot(), second.slot());
        assert!(replacement.sequence() > second.sequence());
        let before = ring;
        assert_eq!(
            ring.complete(
                second,
                FrameCompletionStatus::Completed,
                generation(1),
                PresentationRevision(4),
                SurfaceEpoch(1),
            )
            .map_err(super::FrameSlotError::kind),
            Err(FrameSlotErrorKind::LeaseMismatch)
        );
        assert_eq!(ring, before);
        assert!(ring.invariants_hold());
        Ok(())
    }

    #[test]
    fn terminal_classification_checks_status_generation_revision_and_epoch()
    -> Result<(), &'static str> {
        let cases = [
            (
                FrameCompletionStatus::Completed,
                generation(1),
                PresentationRevision(1),
                SurfaceEpoch(1),
                FrameCompletionDisposition::Current,
            ),
            (
                FrameCompletionStatus::Completed,
                generation(2),
                PresentationRevision(1),
                SurfaceEpoch(1),
                FrameCompletionDisposition::StaleGeneration,
            ),
            (
                FrameCompletionStatus::Completed,
                generation(1),
                PresentationRevision(2),
                SurfaceEpoch(1),
                FrameCompletionDisposition::SupersededRevision,
            ),
            (
                FrameCompletionStatus::Completed,
                generation(1),
                PresentationRevision(1),
                SurfaceEpoch(2),
                FrameCompletionDisposition::SupersededEpoch,
            ),
            (
                FrameCompletionStatus::Failed,
                generation(1),
                PresentationRevision(1),
                SurfaceEpoch(1),
                FrameCompletionDisposition::Failed,
            ),
            (
                FrameCompletionStatus::Cancelled,
                generation(1),
                PresentationRevision(1),
                SurfaceEpoch(1),
                FrameCompletionDisposition::Cancelled,
            ),
        ];
        for (status, current_generation, revision, epoch, expected) in cases {
            let mut ring = FrameSlotRing::new();
            let lease = acquire(&mut ring, token(1, 1, 1))?;
            ring.mark_submitted(lease).map_err(|_| "submit")?;
            let completed = ring
                .complete(lease, status, current_generation, revision, epoch)
                .map_err(|_| "complete")?;
            assert_eq!(completed.lease(), lease);
            assert_eq!(completed.status(), status);
            assert_eq!(completed.disposition(), expected);
            assert_eq!(ring.snapshot().occupied_slots(), 0);
            assert!(ring.invariants_hold());
        }
        Ok(())
    }

    #[test]
    fn invalid_transitions_are_atomic_and_precommit_cancellation_balances()
    -> Result<(), &'static str> {
        let mut ring = FrameSlotRing::new();
        let lease = acquire(&mut ring, token(1, 1, 1))?;
        let before = ring;
        assert_eq!(
            ring.complete(
                lease,
                FrameCompletionStatus::Completed,
                generation(1),
                PresentationRevision(1),
                SurfaceEpoch(1),
            )
            .map_err(super::FrameSlotError::kind),
            Err(FrameSlotErrorKind::ActionDisabled)
        );
        assert_eq!(ring, before);
        ring.cancel_encoding(lease).map_err(|_| "cancel")?;
        let snapshot = ring.snapshot();
        assert_eq!(snapshot.release_count(), 1);
        assert_eq!(snapshot.cancellation_count(), 1);
        assert_eq!(snapshot.occupied_slots(), 0);
        assert!(ring.invariants_hold());
        Ok(())
    }

    #[test]
    fn bounded_reachable_sequences_preserve_invariants() {
        const CHOICES: u64 = 5;
        const STEPS: u32 = 7;
        for encoded in 0..CHOICES.pow(STEPS) {
            let mut choices = encoded;
            let mut ring = FrameSlotRing::new();
            for step in 0..STEPS {
                let choice = choices % CHOICES;
                choices /= CHOICES;
                let attempt = u64::from(step) + 1;
                match choice {
                    0 | 4 => {
                        let _ = ring.acquire(
                            token(attempt, attempt + choice, choice % 2),
                            generation(1 + choice % 2),
                        );
                    }
                    1 => {
                        if let Some(lease) = ring
                            .slots
                            .iter()
                            .find(|slot| matches!(slot.phase, FrameSlotPhase::Encoding))
                            .and_then(|slot| slot.lease)
                        {
                            let _ = ring.mark_submitted(lease);
                        }
                    }
                    2 => {
                        if let Some(lease) = ring
                            .slots
                            .iter()
                            .find(|slot| matches!(slot.phase, FrameSlotPhase::Submitted))
                            .and_then(|slot| slot.lease)
                        {
                            let _ = ring.complete(
                                lease,
                                FrameCompletionStatus::Completed,
                                generation(1),
                                PresentationRevision(attempt),
                                SurfaceEpoch(choice % 2),
                            );
                        }
                    }
                    _ => {
                        if let Some(lease) = ring
                            .slots
                            .iter()
                            .find(|slot| matches!(slot.phase, FrameSlotPhase::Encoding))
                            .and_then(|slot| slot.lease)
                        {
                            let _ = ring.cancel_encoding(lease);
                        }
                    }
                }
                assert!(ring.invariants_hold());
            }
        }
    }
}
