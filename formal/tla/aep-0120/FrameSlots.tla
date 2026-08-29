------------------------------ MODULE FrameSlots ------------------------------
EXTENDS Naturals, FiniteSets, TLC

CONSTANTS MaxSequence, MaxRevision, MaxEpoch, MaxGeneration, FaultyStalePublish

Slots == 0..2
SlotStates == {"Free", "Encoding", "Submitted"}
TerminalStatuses == {"Completed", "Failed", "Cancelled"}
Publications == {"None", "Current", "Rejected"}

VARIABLES slotState, slotSequence, slotGeneration, slotRevision, slotEpoch,
          nextSequence, currentGeneration, currentRevision, currentEpoch,
          admissions, releases, saturations, lastPublication,
          lastGeneration, lastRevision, lastEpoch, shuttingDown

vars == <<slotState, slotSequence, slotGeneration, slotRevision, slotEpoch,
          nextSequence, currentGeneration, currentRevision, currentEpoch,
          admissions, releases, saturations, lastPublication,
          lastGeneration, lastRevision, lastEpoch, shuttingDown>>

Occupied == {slot \in Slots: slotState[slot] # "Free"}

TypeOK ==
    /\ slotState \in [Slots -> SlotStates]
    /\ slotSequence \in [Slots -> 0..MaxSequence]
    /\ slotGeneration \in [Slots -> 0..MaxGeneration]
    /\ slotRevision \in [Slots -> 0..MaxRevision]
    /\ slotEpoch \in [Slots -> 0..MaxEpoch]
    /\ nextSequence \in 0..MaxSequence
    /\ currentGeneration \in 1..MaxGeneration
    /\ currentRevision \in 0..MaxRevision
    /\ currentEpoch \in 0..MaxEpoch
    /\ admissions \in 0..MaxSequence
    /\ releases \in 0..MaxSequence
    /\ saturations \in 0..1
    /\ lastPublication \in Publications
    /\ lastGeneration \in 0..MaxGeneration
    /\ lastRevision \in 0..MaxRevision
    /\ lastEpoch \in 0..MaxEpoch
    /\ shuttingDown \in BOOLEAN

Init ==
    /\ slotState = [slot \in Slots |-> "Free"]
    /\ slotSequence = [slot \in Slots |-> 0]
    /\ slotGeneration = [slot \in Slots |-> 0]
    /\ slotRevision = [slot \in Slots |-> 0]
    /\ slotEpoch = [slot \in Slots |-> 0]
    /\ nextSequence = 0
    /\ currentGeneration = 1
    /\ currentRevision = 0
    /\ currentEpoch = 0
    /\ admissions = 0
    /\ releases = 0
    /\ saturations = 0
    /\ lastPublication = "None"
    /\ lastGeneration = 0
    /\ lastRevision = 0
    /\ lastEpoch = 0
    /\ shuttingDown = FALSE

Acquire(slot) ==
    /\ ~shuttingDown
    /\ slotState[slot] = "Free"
    /\ nextSequence < MaxSequence
    /\ slotState' = [slotState EXCEPT ![slot] = "Encoding"]
    /\ slotSequence' = [slotSequence EXCEPT ![slot] = nextSequence + 1]
    /\ slotGeneration' = [slotGeneration EXCEPT ![slot] = currentGeneration]
    /\ slotRevision' = [slotRevision EXCEPT ![slot] = currentRevision]
    /\ slotEpoch' = [slotEpoch EXCEPT ![slot] = currentEpoch]
    /\ nextSequence' = nextSequence + 1
    /\ admissions' = admissions + 1
    /\ UNCHANGED <<currentGeneration, currentRevision, currentEpoch, releases,
                    saturations, lastPublication, lastGeneration, lastRevision,
                    lastEpoch, shuttingDown>>

RecordSaturation ==
    /\ ~shuttingDown
    /\ Cardinality(Occupied) = 3
    /\ saturations = 0
    /\ saturations' = saturations + 1
    /\ UNCHANGED <<slotState, slotSequence, slotGeneration, slotRevision,
                    slotEpoch, nextSequence, currentGeneration,
                    currentRevision, currentEpoch, admissions, releases,
                    lastPublication, lastGeneration, lastRevision, lastEpoch,
                    shuttingDown>>

Submit(slot) ==
    /\ slotState[slot] = "Encoding"
    /\ slotState' = [slotState EXCEPT ![slot] = "Submitted"]
    /\ UNCHANGED <<slotSequence, slotGeneration, slotRevision, slotEpoch,
                    nextSequence, currentGeneration, currentRevision,
                    currentEpoch, admissions, releases, saturations,
                    lastPublication, lastGeneration, lastRevision, lastEpoch,
                    shuttingDown>>

PublishesCurrent(slot, status) ==
    /\ status = "Completed"
    /\ slotGeneration[slot] = currentGeneration
    /\ slotRevision[slot] = currentRevision
    /\ slotEpoch[slot] = currentEpoch

Release(slot, status) ==
    /\ slotState[slot] = "Submitted"
    /\ status \in TerminalStatuses
    /\ slotState' = [slotState EXCEPT ![slot] = "Free"]
    /\ slotSequence' = [slotSequence EXCEPT ![slot] = 0]
    /\ slotGeneration' = [slotGeneration EXCEPT ![slot] = 0]
    /\ slotRevision' = [slotRevision EXCEPT ![slot] = 0]
    /\ slotEpoch' = [slotEpoch EXCEPT ![slot] = 0]
    /\ releases' = releases + 1
    /\ lastPublication' =
        IF PublishesCurrent(slot, status) THEN "Current" ELSE "Rejected"
    /\ lastGeneration' =
        IF PublishesCurrent(slot, status) THEN slotGeneration[slot] ELSE 0
    /\ lastRevision' =
        IF PublishesCurrent(slot, status) THEN slotRevision[slot] ELSE 0
    /\ lastEpoch' =
        IF PublishesCurrent(slot, status) THEN slotEpoch[slot] ELSE 0
    /\ UNCHANGED <<nextSequence, currentGeneration, currentRevision,
                    currentEpoch, admissions, saturations, shuttingDown>>

CancelEncoding(slot) ==
    /\ slotState[slot] = "Encoding"
    /\ slotState' = [slotState EXCEPT ![slot] = "Free"]
    /\ slotSequence' = [slotSequence EXCEPT ![slot] = 0]
    /\ slotGeneration' = [slotGeneration EXCEPT ![slot] = 0]
    /\ slotRevision' = [slotRevision EXCEPT ![slot] = 0]
    /\ slotEpoch' = [slotEpoch EXCEPT ![slot] = 0]
    /\ releases' = releases + 1
    /\ lastPublication' = "Rejected"
    /\ lastGeneration' = 0
    /\ lastRevision' = 0
    /\ lastEpoch' = 0
    /\ UNCHANGED <<nextSequence, currentGeneration, currentRevision,
                    currentEpoch, admissions, saturations, shuttingDown>>

AdvanceRevision ==
    /\ currentRevision < MaxRevision
    /\ currentRevision' = currentRevision + 1
    /\ lastPublication' = "None"
    /\ lastGeneration' = 0
    /\ lastRevision' = 0
    /\ lastEpoch' = 0
    /\ UNCHANGED <<slotState, slotSequence, slotGeneration, slotRevision,
                    slotEpoch, nextSequence, currentGeneration, currentEpoch,
                    admissions, releases, saturations, shuttingDown>>

AdvanceEpoch ==
    /\ currentEpoch < MaxEpoch
    /\ currentEpoch' = currentEpoch + 1
    /\ lastPublication' = "None"
    /\ lastGeneration' = 0
    /\ lastRevision' = 0
    /\ lastEpoch' = 0
    /\ UNCHANGED <<slotState, slotSequence, slotGeneration, slotRevision,
                    slotEpoch, nextSequence, currentGeneration,
                    currentRevision, admissions, releases, saturations,
                    shuttingDown>>

AdvanceGeneration ==
    /\ currentGeneration < MaxGeneration
    /\ currentGeneration' = currentGeneration + 1
    /\ lastPublication' = "None"
    /\ lastGeneration' = 0
    /\ lastRevision' = 0
    /\ lastEpoch' = 0
    /\ UNCHANGED <<slotState, slotSequence, slotGeneration, slotRevision,
                    slotEpoch, nextSequence, currentRevision, currentEpoch,
                    admissions, releases, saturations, shuttingDown>>

BeginShutdown ==
    /\ ~shuttingDown
    /\ shuttingDown' = TRUE
    /\ UNCHANGED <<slotState, slotSequence, slotGeneration, slotRevision,
                    slotEpoch, nextSequence, currentGeneration,
                    currentRevision, currentEpoch, admissions, releases,
                    saturations, lastPublication, lastGeneration,
                    lastRevision, lastEpoch>>

FaultyPublishStale(slot) ==
    /\ FaultyStalePublish
    /\ slotState[slot] = "Submitted"
    /\ \/ slotGeneration[slot] # currentGeneration
       \/ slotRevision[slot] # currentRevision
       \/ slotEpoch[slot] # currentEpoch
    /\ slotState' = [slotState EXCEPT ![slot] = "Free"]
    /\ slotSequence' = [slotSequence EXCEPT ![slot] = 0]
    /\ slotGeneration' = [slotGeneration EXCEPT ![slot] = 0]
    /\ slotRevision' = [slotRevision EXCEPT ![slot] = 0]
    /\ slotEpoch' = [slotEpoch EXCEPT ![slot] = 0]
    /\ releases' = releases + 1
    /\ lastPublication' = "Current"
    /\ lastGeneration' = slotGeneration[slot]
    /\ lastRevision' = slotRevision[slot]
    /\ lastEpoch' = slotEpoch[slot]
    /\ UNCHANGED <<nextSequence, currentGeneration, currentRevision,
                    currentEpoch, admissions, saturations, shuttingDown>>

Next ==
    \/ \E slot \in Slots: Acquire(slot)
    \/ RecordSaturation
    \/ \E slot \in Slots: Submit(slot)
    \/ \E slot \in Slots, status \in TerminalStatuses: Release(slot, status)
    \/ \E slot \in Slots: CancelEncoding(slot)
    \/ AdvanceRevision
    \/ AdvanceEpoch
    \/ AdvanceGeneration
    \/ BeginShutdown
    \/ \E slot \in Slots: FaultyPublishStale(slot)

Spec ==
    /\ Init
    /\ [][Next]_vars
    /\ \A slot \in Slots: WF_vars(Submit(slot))
    /\ \A slot \in Slots: WF_vars(Release(slot, "Completed"))
    /\ \A slot \in Slots: WF_vars(CancelEncoding(slot))

BoundedFrameSlots == Cardinality(Occupied) <= 3

FreeSlotsHaveNoIdentity ==
    \A slot \in Slots:
        (slotState[slot] = "Free") <=>
        (slotSequence[slot] = 0 /\ slotGeneration[slot] = 0)

OwnedSequencesUnique ==
    \A left, right \in Slots:
        left # right /\ left \in Occupied /\ right \in Occupied =>
            slotSequence[left] # slotSequence[right]

BalancedOwnership == admissions = releases + Cardinality(Occupied)

CurrentPublicationIsCurrent ==
    lastPublication = "Current" =>
        /\ lastGeneration = currentGeneration
        /\ lastRevision = currentRevision
        /\ lastEpoch = currentEpoch

InactivePublicationHasNoIdentity ==
    lastPublication # "Current" =>
        /\ lastGeneration = 0
        /\ lastRevision = 0
        /\ lastEpoch = 0

SubmittedEventuallyReleases ==
    \A slot \in Slots: [](slotState[slot] = "Submitted" => <> (slotState[slot] = "Free"))

ShutdownEventuallyDrains ==
    [](shuttingDown => <> (Cardinality(Occupied) = 0))

=============================================================================
