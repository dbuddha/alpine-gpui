---- MODULE AccessibilityTransport ----
EXTENDS Naturals

CONSTANT MaxRevision, MaxRequest

VARIABLES open, revision, nextRequest, activeRequest, requestedRevision,
          responseRequest, observedRevision, responses, mutationRevision

vars == <<open, revision, nextRequest, activeRequest, requestedRevision,
          responseRequest, observedRevision, responses, mutationRevision>>

Init ==
    /\ open = TRUE
    /\ revision = 1
    /\ nextRequest = 0
    /\ activeRequest = 0
    /\ requestedRevision = 0
    /\ responseRequest = 0
    /\ observedRevision = 0
    /\ responses = 0
    /\ mutationRevision = 0

Issue ==
    /\ open
    /\ nextRequest < MaxRequest
    /\ nextRequest' = nextRequest + 1
    /\ activeRequest' = nextRequest + 1
    /\ requestedRevision' = revision
    /\ responseRequest' = 0
    /\ observedRevision' = 0
    /\ responses' = 0
    /\ mutationRevision' = 0
    /\ UNCHANGED <<open, revision>>

ChangeRevision ==
    /\ open
    /\ revision < MaxRevision
    /\ revision' = revision + 1
    /\ responseRequest' = 0
    /\ observedRevision' = 0
    /\ responses' = 0
    /\ mutationRevision' = 0
    /\ UNCHANGED <<open, nextRequest, activeRequest, requestedRevision>>

Respond ==
    /\ open
    /\ activeRequest # 0
    /\ responses = 0
    /\ responseRequest' = activeRequest
    /\ observedRevision' = revision
    /\ responses' = 1
    /\ UNCHANGED <<open, revision, nextRequest, activeRequest,
                    requestedRevision, mutationRevision>>

ApplyCurrent ==
    /\ open
    /\ activeRequest # 0
    /\ requestedRevision = revision
    /\ responses = 1
    /\ responseRequest = activeRequest
    /\ observedRevision = revision
    /\ mutationRevision' = revision
    /\ UNCHANGED <<open, revision, nextRequest, activeRequest,
                    requestedRevision, responseRequest, observedRevision,
                    responses>>

Close ==
    /\ open
    /\ open' = FALSE
    /\ activeRequest' = 0
    /\ requestedRevision' = 0
    /\ responseRequest' = 0
    /\ observedRevision' = 0
    /\ responses' = 0
    /\ mutationRevision' = 0
    /\ UNCHANGED <<revision, nextRequest>>

Reopen ==
    /\ ~open
    /\ open' = TRUE
    /\ activeRequest' = 0
    /\ requestedRevision' = 0
    /\ responseRequest' = 0
    /\ observedRevision' = 0
    /\ responses' = 0
    /\ mutationRevision' = 0
    /\ UNCHANGED <<revision, nextRequest>>

Next == Issue \/ ChangeRevision \/ Respond \/ ApplyCurrent \/ Close \/ Reopen

Spec == Init /\ [][Next]_vars

ResponseMatchesRequest ==
    responseRequest = 0 \/
    (open /\ responseRequest = activeRequest /\
     observedRevision = revision /\ responses = 1)

MutationIsCurrent ==
    mutationRevision = 0 \/ (open /\ mutationRevision = revision)

AtMostOneResponse == responses <= 1

ClosedOwnsNoResponse ==
    ~open => (activeRequest = 0 /\ responseRequest = 0 /\
              responses = 0 /\ mutationRevision = 0)

FaultyStaleMutation ==
    /\ open
    /\ activeRequest # 0
    /\ requestedRevision # revision
    /\ mutationRevision' = requestedRevision
    /\ UNCHANGED <<open, revision, nextRequest, activeRequest,
                    requestedRevision, responseRequest, observedRevision,
                    responses>>

FaultyDuplicateResponse ==
    /\ open
    /\ activeRequest # 0
    /\ responses = 1
    /\ responses' = 2
    /\ UNCHANGED <<open, revision, nextRequest, activeRequest,
                    requestedRevision, responseRequest, observedRevision,
                    mutationRevision>>

FaultyStaleSpec == Init /\ [][Next \/ FaultyStaleMutation]_vars
FaultyDuplicateSpec == Init /\ [][Next \/ FaultyDuplicateResponse]_vars

====
