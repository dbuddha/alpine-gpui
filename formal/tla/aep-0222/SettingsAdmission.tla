---- MODULE SettingsAdmission ----
EXTENDS Naturals

CONSTANT MaxGeneration

VARIABLES requested, submitted, inFlight, pending, accepted, config

vars == <<requested, submitted, inFlight, pending, accepted, config>>

Init ==
    /\ requested = 1
    /\ submitted = 0
    /\ inFlight = FALSE
    /\ pending = TRUE
    /\ accepted = 0
    /\ config = 0

Request ==
    /\ requested < MaxGeneration
    /\ requested' = requested + 1
    /\ pending' = TRUE
    /\ UNCHANGED <<submitted, inFlight, accepted, config>>

Submit ==
    /\ pending
    /\ ~inFlight
    /\ submitted' = requested
    /\ inFlight' = TRUE
    /\ pending' = FALSE
    /\ UNCHANGED <<requested, accepted, config>>

CompleteCurrent ==
    /\ inFlight
    /\ submitted = requested
    /\ accepted' = submitted
    /\ config' = submitted
    /\ inFlight' = FALSE
    /\ UNCHANGED <<requested, submitted, pending>>

CompleteSuperseded ==
    /\ inFlight
    /\ submitted # requested
    /\ inFlight' = FALSE
    /\ UNCHANGED <<requested, submitted, pending, accepted, config>>

FailCurrent ==
    /\ inFlight
    /\ submitted = requested
    /\ inFlight' = FALSE
    /\ UNCHANGED <<requested, submitted, pending, accepted, config>>

Retry ==
    /\ ~inFlight
    /\ ~pending
    /\ pending' = TRUE
    /\ UNCHANGED <<requested, submitted, inFlight, accepted, config>>

Next == Request \/ Submit \/ CompleteCurrent \/ CompleteSuperseded \/ FailCurrent \/ Retry

Spec == Init /\ [][Next]_vars

PublishedIsCurrent == accepted = 0 \/ accepted = requested

PublicationIsAtomic == accepted = config

OneInFlightGeneration == ~inFlight \/ submitted > 0

FaultyPublishStale ==
    /\ inFlight
    /\ submitted < requested
    /\ accepted' = submitted
    /\ config' = submitted
    /\ inFlight' = FALSE
    /\ UNCHANGED <<requested, submitted, pending>>

FaultySpec == Init /\ [][Next \/ FaultyPublishStale]_vars

====
