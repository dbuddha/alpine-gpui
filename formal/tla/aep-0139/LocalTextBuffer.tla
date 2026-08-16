--------------------------- MODULE LocalTextBuffer --------------------------
EXTENDS Naturals, Sequences, TLC

CONSTANTS Contents, MaxRevision, MaxAttempts, MaxHistory,
          FaultyReject, FaultyConflict

Dispositions == {"None", "Accepted", "Rejected", "Undo", "Redo"}
SaveDispositions == {"None", "Saved", "Conflict"}
DiskValues == Contents \cup {"Missing"}

VARIABLES content, revision, undo, redo, attempts,
          priorContent, priorRevision, disposition,
          disk, acceptedDisk, savedContent, saveDisposition,
          priorDisk, priorSavedContent

vars == <<content, revision, undo, redo, attempts,
          priorContent, priorRevision, disposition,
          disk, acceptedDisk, savedContent, saveDisposition,
          priorDisk, priorSavedContent>>

BoundedAppend(sequence, value) ==
    IF Len(sequence) < MaxHistory
    THEN Append(sequence, value)
    ELSE Append(Tail(sequence), value)

RemoveLast(sequence) ==
    IF Len(sequence) = 1
    THEN <<>>
    ELSE SubSeq(sequence, 1, Len(sequence) - 1)

TypeOK ==
    /\ content \in Contents
    /\ revision \in 0..MaxRevision
    /\ undo \in Seq(Contents)
    /\ redo \in Seq(Contents)
    /\ attempts \in 0..MaxAttempts
    /\ priorContent \in Contents
    /\ priorRevision \in 0..MaxRevision
    /\ disposition \in Dispositions
    /\ disk \in DiskValues
    /\ acceptedDisk \in Contents
    /\ savedContent \in Contents
    /\ saveDisposition \in SaveDispositions
    /\ priorDisk \in DiskValues
    /\ priorSavedContent \in Contents

Init ==
    /\ content \in Contents
    /\ revision = 0
    /\ undo = <<>>
    /\ redo = <<>>
    /\ attempts = 0
    /\ priorContent = content
    /\ priorRevision = revision
    /\ disposition = "None"
    /\ disk = content
    /\ acceptedDisk = content
    /\ savedContent = content
    /\ saveDisposition = "None"
    /\ priorDisk = disk
    /\ priorSavedContent = savedContent

Apply(nextContent) ==
    /\ attempts < MaxAttempts
    /\ revision < MaxRevision
    /\ nextContent \in Contents
    /\ nextContent # content
    /\ priorContent' = content
    /\ priorRevision' = revision
    /\ content' = nextContent
    /\ revision' = revision + 1
    /\ undo' = BoundedAppend(undo, content)
    /\ redo' = <<>>
    /\ attempts' = attempts + 1
    /\ disposition' = "Accepted"
    /\ UNCHANGED <<disk, acceptedDisk, savedContent, saveDisposition,
                    priorDisk, priorSavedContent>>

Reject(nextContent) ==
    /\ attempts < MaxAttempts
    /\ nextContent \in Contents
    /\ nextContent # content
    /\ priorContent' = content
    /\ priorRevision' = revision
    /\ content' = IF FaultyReject THEN nextContent ELSE content
    /\ revision' = revision
    /\ attempts' = attempts + 1
    /\ disposition' = "Rejected"
    /\ UNCHANGED <<undo, redo, disk, acceptedDisk, savedContent,
                    saveDisposition, priorDisk, priorSavedContent>>

Undo ==
    /\ attempts < MaxAttempts
    /\ revision < MaxRevision
    /\ Len(undo) > 0
    /\ priorContent' = content
    /\ priorRevision' = revision
    /\ content' = undo[Len(undo)]
    /\ revision' = revision + 1
    /\ undo' = RemoveLast(undo)
    /\ redo' = BoundedAppend(redo, content)
    /\ attempts' = attempts + 1
    /\ disposition' = "Undo"
    /\ UNCHANGED <<disk, acceptedDisk, savedContent, saveDisposition,
                    priorDisk, priorSavedContent>>

Redo ==
    /\ attempts < MaxAttempts
    /\ revision < MaxRevision
    /\ Len(redo) > 0
    /\ priorContent' = content
    /\ priorRevision' = revision
    /\ content' = redo[Len(redo)]
    /\ revision' = revision + 1
    /\ redo' = RemoveLast(redo)
    /\ undo' = BoundedAppend(undo, content)
    /\ attempts' = attempts + 1
    /\ disposition' = "Redo"
    /\ UNCHANGED <<disk, acceptedDisk, savedContent, saveDisposition,
                    priorDisk, priorSavedContent>>

ExternalChange(nextDisk) ==
    /\ attempts < MaxAttempts
    /\ nextDisk \in DiskValues
    /\ nextDisk # disk
    /\ priorDisk' = disk
    /\ priorSavedContent' = savedContent
    /\ disk' = nextDisk
    /\ saveDisposition' = "None"
    /\ attempts' = attempts + 1
    /\ UNCHANGED <<content, revision, undo, redo, priorContent,
                    priorRevision, disposition, acceptedDisk, savedContent>>

Save ==
    /\ attempts < MaxAttempts
    /\ disk = acceptedDisk
    /\ priorDisk' = disk
    /\ priorSavedContent' = savedContent
    /\ disk' = content
    /\ acceptedDisk' = content
    /\ savedContent' = content
    /\ saveDisposition' = "Saved"
    /\ attempts' = attempts + 1
    /\ UNCHANGED <<content, revision, undo, redo, priorContent,
                    priorRevision, disposition>>

RejectConflictingSave ==
    /\ attempts < MaxAttempts
    /\ disk # acceptedDisk
    /\ priorDisk' = disk
    /\ priorSavedContent' = savedContent
    /\ disk' = IF FaultyConflict THEN content ELSE disk
    /\ savedContent' = IF FaultyConflict THEN content ELSE savedContent
    /\ saveDisposition' = "Conflict"
    /\ attempts' = attempts + 1
    /\ UNCHANGED <<content, revision, undo, redo, priorContent,
                    priorRevision, disposition, acceptedDisk>>

Next ==
    \/ \E nextContent \in Contents: Apply(nextContent)
    \/ \E nextContent \in Contents: Reject(nextContent)
    \/ Undo
    \/ Redo
    \/ \E nextDisk \in DiskValues: ExternalChange(nextDisk)
    \/ Save
    \/ RejectConflictingSave

Spec == Init /\ [][Next]_vars

RejectedIsAtomic ==
    disposition = "Rejected" =>
        /\ content = priorContent
        /\ revision = priorRevision

RevisionNeverDecreases == revision >= priorRevision

HistoryIsBounded ==
    /\ Len(undo) <= MaxHistory
    /\ Len(redo) <= MaxHistory

ConflictPreservesAcceptedDisk ==
    saveDisposition = "Conflict" =>
        /\ disk = priorDisk
        /\ savedContent = priorSavedContent

=============================================================================
