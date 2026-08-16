------------------------ MODULE ClipboardCloseResponse -----------------------
EXTENDS Naturals, TLC

CONSTANTS MaxClipboardBytes, FaultyCancelCloses

CloseStates == {"Idle", "Requested", "Cancelled", "Allowed"}

VARIABLES live, closeState, clipboardWriteBytes

vars == <<live, closeState, clipboardWriteBytes>>

TypeOK ==
    /\ live \in BOOLEAN
    /\ closeState \in CloseStates
    /\ clipboardWriteBytes \in 0..MaxClipboardBytes

Init ==
    /\ live = TRUE
    /\ closeState = "Idle"
    /\ clipboardWriteBytes = 0

RequestClipboardWrite(bytes) ==
    /\ live
    /\ clipboardWriteBytes = 0
    /\ bytes \in 1..MaxClipboardBytes
    /\ clipboardWriteBytes' = bytes
    /\ UNCHANGED <<live, closeState>>

CompleteClipboardWrite ==
    /\ clipboardWriteBytes > 0
    /\ clipboardWriteBytes' = 0
    /\ UNCHANGED <<live, closeState>>

RequestClose ==
    /\ live
    /\ closeState \in {"Idle", "Cancelled"}
    /\ closeState' = "Requested"
    /\ UNCHANGED <<live, clipboardWriteBytes>>

CancelClose ==
    /\ closeState = "Requested"
    /\ closeState' = "Cancelled"
    /\ live' = IF FaultyCancelCloses THEN FALSE ELSE TRUE
    /\ UNCHANGED clipboardWriteBytes

AllowClose ==
    /\ closeState = "Requested"
    /\ closeState' = "Allowed"
    /\ live' = FALSE
    /\ UNCHANGED clipboardWriteBytes

Next ==
    \/ \E bytes \in 1..MaxClipboardBytes: RequestClipboardWrite(bytes)
    \/ CompleteClipboardWrite
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

RequestedCloseEventuallyResolves ==
    [](closeState = "Requested" => <> (closeState # "Requested"))

=============================================================================
