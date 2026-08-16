------------------------ MODULE ClipboardCloseResponse -----------------------
EXTENDS Naturals, TLC

CONSTANTS MaxClipboardBytes, FaultyCancelCloses,
          FaultyStaleCutMutates, FaultyDirtyCloseAllows

CloseStates == {"Idle", "Requested", "Cancelled", "Allowed"}

VARIABLES live, closeState, clipboardWriteBytes,
          documentRevision, selectionIdentity, documentBytes, dirty,
          cutPending, pendingCutRevision, pendingCutSelection,
          invalidCutMutation, closeWasDirty

vars == <<live, closeState, clipboardWriteBytes,
          documentRevision, selectionIdentity, documentBytes, dirty,
          cutPending, pendingCutRevision, pendingCutSelection,
          invalidCutMutation, closeWasDirty>>

documentState ==
    <<documentRevision, selectionIdentity, documentBytes, dirty,
      cutPending, pendingCutRevision, pendingCutSelection,
      invalidCutMutation>>

TypeOK ==
    /\ live \in BOOLEAN
    /\ closeState \in CloseStates
    /\ clipboardWriteBytes \in 0..MaxClipboardBytes
    /\ documentRevision \in BOOLEAN
    /\ selectionIdentity \in BOOLEAN
    /\ documentBytes \in 0..MaxClipboardBytes
    /\ dirty \in BOOLEAN
    /\ cutPending \in BOOLEAN
    /\ pendingCutRevision \in BOOLEAN
    /\ pendingCutSelection \in BOOLEAN
    /\ invalidCutMutation \in BOOLEAN
    /\ closeWasDirty \in BOOLEAN

Init ==
    /\ live = TRUE
    /\ closeState = "Idle"
    /\ clipboardWriteBytes = 0
    /\ documentRevision = FALSE
    /\ selectionIdentity = FALSE
    /\ documentBytes = MaxClipboardBytes
    /\ dirty = FALSE
    /\ cutPending = FALSE
    /\ pendingCutRevision = FALSE
    /\ pendingCutSelection = FALSE
    /\ invalidCutMutation = FALSE
    /\ closeWasDirty = FALSE

RequestClipboardWrite(bytes) ==
    /\ live
    /\ ~cutPending
    /\ clipboardWriteBytes = 0
    /\ bytes \in 1..MaxClipboardBytes
    /\ clipboardWriteBytes' = bytes
    /\ UNCHANGED <<live, closeState, documentState, closeWasDirty>>

CompleteClipboardWrite ==
    /\ clipboardWriteBytes > 0
    /\ ~cutPending
    /\ clipboardWriteBytes' = 0
    /\ UNCHANGED <<live, closeState, documentState, closeWasDirty>>

BeginCut ==
    /\ live
    /\ ~cutPending
    /\ documentBytes > 0
    /\ clipboardWriteBytes = 0
    /\ cutPending' = TRUE
    /\ pendingCutRevision' = documentRevision
    /\ pendingCutSelection' = selectionIdentity
    /\ clipboardWriteBytes' = 1
    /\ UNCHANGED <<live, closeState, documentRevision,
                    selectionIdentity, documentBytes, dirty,
                    invalidCutMutation, closeWasDirty>>

ChangeSelection ==
    /\ live
    /\ cutPending
    /\ selectionIdentity' = ~selectionIdentity
    /\ UNCHANGED <<live, closeState, clipboardWriteBytes,
                    documentRevision, documentBytes, dirty, cutPending,
                    pendingCutRevision, pendingCutSelection,
                    invalidCutMutation, closeWasDirty>>

EditDocument ==
    /\ live
    /\ documentRevision' = ~documentRevision
    /\ dirty' = TRUE
    /\ UNCHANGED <<live, closeState, clipboardWriteBytes,
                    selectionIdentity, documentBytes, cutPending,
                    pendingCutRevision, pendingCutSelection,
                    invalidCutMutation, closeWasDirty>>

SaveDocument ==
    /\ live
    /\ dirty
    /\ dirty' = FALSE
    /\ UNCHANGED <<live, closeState, clipboardWriteBytes,
                    documentRevision, selectionIdentity, documentBytes,
                    cutPending, pendingCutRevision, pendingCutSelection,
                    invalidCutMutation, closeWasDirty>>

CompleteCut(success) ==
    LET matches ==
            /\ documentRevision = pendingCutRevision
            /\ selectionIdentity = pendingCutSelection
        mutates ==
            /\ success
            /\ (matches \/ FaultyStaleCutMutates)
    IN
    /\ cutPending
    /\ clipboardWriteBytes > 0
    /\ cutPending' = FALSE
    /\ clipboardWriteBytes' = 0
    /\ documentBytes' = IF mutates THEN documentBytes - 1 ELSE documentBytes
    /\ documentRevision' = IF mutates THEN ~documentRevision ELSE documentRevision
    /\ dirty' = IF mutates THEN TRUE ELSE dirty
    /\ invalidCutMutation' =
        invalidCutMutation \/ (mutates /\ ~matches)
    /\ UNCHANGED <<live, closeState, selectionIdentity,
                    pendingCutRevision, pendingCutSelection, closeWasDirty>>

RequestClose ==
    /\ live
    /\ closeState \in {"Idle", "Cancelled"}
    /\ closeState' = "Requested"
    /\ closeWasDirty' = dirty
    /\ UNCHANGED <<live, clipboardWriteBytes, documentState>>

CancelClose ==
    /\ closeState = "Requested"
    /\ closeWasDirty
    /\ closeState' = "Cancelled"
    /\ live' = IF FaultyCancelCloses THEN FALSE ELSE TRUE
    /\ UNCHANGED <<clipboardWriteBytes, documentState, closeWasDirty>>

AllowClose ==
    /\ closeState = "Requested"
    /\ (~closeWasDirty \/ FaultyDirtyCloseAllows)
    /\ closeState' = "Allowed"
    /\ live' = FALSE
    /\ UNCHANGED <<clipboardWriteBytes, documentState, closeWasDirty>>

Next ==
    \/ \E bytes \in 1..MaxClipboardBytes: RequestClipboardWrite(bytes)
    \/ CompleteClipboardWrite
    \/ BeginCut
    \/ ChangeSelection
    \/ EditDocument
    \/ SaveDocument
    \/ CompleteCut(TRUE)
    \/ CompleteCut(FALSE)
    \/ RequestClose
    \/ CancelClose
    \/ AllowClose

Spec ==
    /\ Init
    /\ [][Next]_vars
    /\ WF_vars(CompleteClipboardWrite)
    /\ WF_vars(CancelClose \/ AllowClose)

ClipboardResponseIsBounded == clipboardWriteBytes <= MaxClipboardBytes

ResponseChannelsAreIndependent ==
    /\ clipboardWriteBytes \in 0..MaxClipboardBytes
    /\ closeState \in CloseStates

CancelledCloseStaysLive == closeState = "Cancelled" => live

AllowedCloseRevokesAdmission == closeState = "Allowed" => ~live

CutMutationRequiresMatchingCompletion == ~invalidCutMutation

DirtyCloseNeverAllows == ~(closeState = "Allowed" /\ closeWasDirty)

RequestedCloseEventuallyResolves ==
    [](closeState = "Requested" => <> (closeState # "Requested"))

=============================================================================
