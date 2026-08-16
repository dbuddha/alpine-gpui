-------------------------- MODULE WorkspaceSelection ------------------------
EXTENDS Naturals, TLC

CONSTANTS MaxDocumentIdentity, MaxDocumentBytes,
          FaultyFailedSelectionMutates,
          FaultyReplacementDoesNotAdvance

VARIABLES documentIdentity, previousIdentity, documentBytes, dirty,
          activeFile, failedSelectionMutated, replacementWithoutAdvance

vars == <<documentIdentity, previousIdentity, documentBytes, dirty,
          activeFile, failedSelectionMutated, replacementWithoutAdvance>>

TypeOK ==
    /\ MaxDocumentIdentity > 0
    /\ MaxDocumentBytes > 0
    /\ documentIdentity \in 0..MaxDocumentIdentity
    /\ previousIdentity \in 0..MaxDocumentIdentity
    /\ documentBytes \in 0..MaxDocumentBytes
    /\ dirty \in BOOLEAN
    /\ activeFile \in 0..1
    /\ failedSelectionMutated \in BOOLEAN
    /\ replacementWithoutAdvance \in BOOLEAN

Init ==
    /\ documentIdentity = 0
    /\ previousIdentity = 0
    /\ documentBytes = 0
    /\ dirty = FALSE
    /\ activeFile = 0
    /\ failedSelectionMutated = FALSE
    /\ replacementWithoutAdvance = FALSE

EditDocument ==
    /\ documentIdentity < MaxDocumentIdentity
    /\ previousIdentity' = documentIdentity
    /\ documentIdentity' = documentIdentity + 1
    /\ documentBytes' = (documentBytes + 1) % (MaxDocumentBytes + 1)
    /\ dirty' = TRUE
    /\ UNCHANGED <<activeFile, failedSelectionMutated,
                    replacementWithoutAdvance>>

SaveDocument ==
    /\ dirty
    /\ dirty' = FALSE
    /\ UNCHANGED <<documentIdentity, previousIdentity, documentBytes,
                    activeFile, failedSelectionMutated,
                    replacementWithoutAdvance>>

OpenValidFile ==
    /\ ~dirty
    /\ documentIdentity < MaxDocumentIdentity
    /\ previousIdentity' = documentIdentity
    /\ documentIdentity' =
        IF FaultyReplacementDoesNotAdvance
        THEN documentIdentity
        ELSE documentIdentity + 1
    /\ documentBytes' = (documentBytes + 1) % (MaxDocumentBytes + 1)
    /\ activeFile' = 1 - activeFile
    /\ replacementWithoutAdvance' =
        (replacementWithoutAdvance \/ FaultyReplacementDoesNotAdvance)
    /\ UNCHANGED <<dirty, failedSelectionMutated>>

RejectSelection ==
    /\ previousIdentity' = documentIdentity
    /\ documentIdentity' =
        IF FaultyFailedSelectionMutates /\ documentIdentity < MaxDocumentIdentity
        THEN documentIdentity + 1
        ELSE documentIdentity
    /\ documentBytes' =
        IF FaultyFailedSelectionMutates
        THEN (documentBytes + 1) % (MaxDocumentBytes + 1)
        ELSE documentBytes
    /\ failedSelectionMutated' =
        (failedSelectionMutated \/ FaultyFailedSelectionMutates)
    /\ UNCHANGED <<dirty, activeFile, replacementWithoutAdvance>>

Next ==
    \/ EditDocument
    \/ SaveDocument
    \/ OpenValidFile
    \/ RejectSelection

Spec ==
    /\ Init
    /\ [][Next]_vars
    /\ WF_vars(SaveDocument)
    /\ WF_vars(OpenValidFile)

FailedSelectionPreservesDocument == ~failedSelectionMutated

SuccessfulReplacementAdvancesIdentity == ~replacementWithoutAdvance

DocumentIdentityNeverDecreases == documentIdentity >= previousIdentity

=============================================================================
