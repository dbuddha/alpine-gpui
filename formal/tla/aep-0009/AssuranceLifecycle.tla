------------------------- MODULE AssuranceLifecycle -------------------------
EXTENDS Integers, TLC

CONSTANTS Requirements, EvidenceKinds, RequiredKinds, FaultyClosure

VARIABLES approved, implemented, recordedEvidence, closed, capabilityClosed

vars == <<approved, implemented, recordedEvidence, closed, capabilityClosed>>

EvidenceFor(requirement) ==
    {kind \in EvidenceKinds : <<requirement, kind>> \in recordedEvidence}

TypeOK ==
    /\ approved \subseteq Requirements
    /\ implemented \subseteq Requirements
    /\ recordedEvidence \subseteq (Requirements \X EvidenceKinds)
    /\ closed \subseteq Requirements
    /\ capabilityClosed \in BOOLEAN

Init ==
    /\ approved = {}
    /\ implemented = {}
    /\ recordedEvidence = {}
    /\ closed = {}
    /\ capabilityClosed = FALSE

Approve(requirement) ==
    /\ requirement \in Requirements \ approved
    /\ approved' = approved \cup {requirement}
    /\ UNCHANGED <<implemented, recordedEvidence, closed, capabilityClosed>>

Implement(requirement) ==
    /\ requirement \in approved \ implemented
    /\ implemented' = implemented \cup {requirement}
    /\ UNCHANGED <<approved, recordedEvidence, closed, capabilityClosed>>

RecordEvidence(requirement, kind) ==
    /\ requirement \in implemented
    /\ kind \in EvidenceKinds
    /\ <<requirement, kind>> \notin recordedEvidence
    /\ recordedEvidence' = recordedEvidence \cup {<<requirement, kind>>}
    /\ UNCHANGED <<approved, implemented, closed, capabilityClosed>>

CloseRequirement(requirement) ==
    /\ requirement \in Requirements \ closed
    /\ FaultyClosure
       \/ /\ requirement \in implemented
          /\ RequiredKinds \subseteq EvidenceFor(requirement)
    /\ closed' = closed \cup {requirement}
    /\ UNCHANGED <<approved, implemented, recordedEvidence, capabilityClosed>>

CloseCapability ==
    /\ ~capabilityClosed
    /\ closed = Requirements
    /\ capabilityClosed' = TRUE
    /\ UNCHANGED <<approved, implemented, recordedEvidence, closed>>

Next ==
    \/ \E requirement \in Requirements : Approve(requirement)
    \/ \E requirement \in Requirements : Implement(requirement)
    \/ \E requirement \in Requirements, kind \in EvidenceKinds :
           RecordEvidence(requirement, kind)
    \/ \E requirement \in Requirements : CloseRequirement(requirement)
    \/ CloseCapability

Spec == Init /\ [][Next]_vars /\ WF_vars(Next)

QualifiedClosure ==
    \A requirement \in closed :
        /\ requirement \in approved
        /\ requirement \in implemented
        /\ RequiredKinds \subseteq EvidenceFor(requirement)

CapabilityClosure == capabilityClosed => closed = Requirements

CanComplete == <>capabilityClosed

=============================================================================
