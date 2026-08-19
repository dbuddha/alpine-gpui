---- MODULE CompletionAdmission ----
EXTENDS Naturals

CONSTANT MaxIdentity, MaxRequest, MaxItems

VARIABLES open, identity, nextRequest, pendingRequest, pendingIdentity,
          cancelledRequest, cancelledIdentity, admittedRequest,
          admittedIdentity, items, appliedRequest, appliedIdentity

vars == <<open, identity, nextRequest, pendingRequest, pendingIdentity,
          cancelledRequest, cancelledIdentity, admittedRequest,
          admittedIdentity, items, appliedRequest, appliedIdentity>>

Init ==
    /\ open = FALSE
    /\ identity = 0
    /\ nextRequest = 0
    /\ pendingRequest = 0
    /\ pendingIdentity = 0
    /\ cancelledRequest = 0
    /\ cancelledIdentity = 0
    /\ admittedRequest = 0
    /\ admittedIdentity = 0
    /\ items = 0
    /\ appliedRequest = 0
    /\ appliedIdentity = 0

Open ==
    /\ ~open
    /\ open' = TRUE
    /\ UNCHANGED <<identity, nextRequest, pendingRequest, pendingIdentity,
                    cancelledRequest, cancelledIdentity, admittedRequest,
                    admittedIdentity, items, appliedRequest, appliedIdentity>>

ChangeIdentity ==
    /\ open
    /\ identity < MaxIdentity
    /\ identity' = identity + 1
    /\ cancelledRequest' = pendingRequest
    /\ cancelledIdentity' = pendingIdentity
    /\ pendingRequest' = 0
    /\ pendingIdentity' = 0
    /\ admittedRequest' = 0
    /\ admittedIdentity' = 0
    /\ items' = 0
    /\ appliedRequest' = 0
    /\ appliedIdentity' = 0
    /\ UNCHANGED <<open, nextRequest>>

Trigger ==
    /\ open
    /\ nextRequest < MaxRequest
    /\ nextRequest' = nextRequest + 1
    /\ pendingRequest' = nextRequest + 1
    /\ pendingIdentity' = identity
    /\ cancelledRequest' = pendingRequest
    /\ cancelledIdentity' = pendingIdentity
    /\ admittedRequest' = 0
    /\ admittedIdentity' = 0
    /\ items' = 0
    /\ appliedRequest' = 0
    /\ appliedIdentity' = 0
    /\ UNCHANGED <<open, identity>>

CompleteCurrent ==
    /\ open
    /\ pendingRequest # 0
    /\ pendingRequest = nextRequest
    /\ pendingIdentity = identity
    /\ \E count \in 0..MaxItems: items' = count
    /\ admittedRequest' = pendingRequest
    /\ admittedIdentity' = pendingIdentity
    /\ pendingRequest' = 0
    /\ pendingIdentity' = 0
    /\ UNCHANGED <<open, identity, nextRequest, cancelledRequest,
                    cancelledIdentity, appliedRequest, appliedIdentity>>

DropCancelled ==
    /\ cancelledRequest # 0
    /\ cancelledRequest' = 0
    /\ cancelledIdentity' = 0
    /\ UNCHANGED <<open, identity, nextRequest, pendingRequest,
                    pendingIdentity, admittedRequest, admittedIdentity,
                    items, appliedRequest, appliedIdentity>>

ApplyCurrent ==
    /\ open
    /\ admittedRequest # 0
    /\ admittedRequest = nextRequest
    /\ admittedIdentity = identity
    /\ items > 0
    /\ appliedRequest' = admittedRequest
    /\ appliedIdentity' = admittedIdentity
    /\ admittedRequest' = 0
    /\ admittedIdentity' = 0
    /\ items' = 0
    /\ UNCHANGED <<open, identity, nextRequest, pendingRequest,
                    pendingIdentity, cancelledRequest, cancelledIdentity>>

FocusLoss ==
    /\ open
    /\ cancelledRequest' = pendingRequest
    /\ cancelledIdentity' = pendingIdentity
    /\ pendingRequest' = 0
    /\ pendingIdentity' = 0
    /\ admittedRequest' = 0
    /\ admittedIdentity' = 0
    /\ items' = 0
    /\ UNCHANGED <<open, identity, nextRequest, appliedRequest, appliedIdentity>>

Close ==
    /\ open
    /\ open' = FALSE
    /\ pendingRequest' = 0
    /\ pendingIdentity' = 0
    /\ cancelledRequest' = 0
    /\ cancelledIdentity' = 0
    /\ admittedRequest' = 0
    /\ admittedIdentity' = 0
    /\ items' = 0
    /\ appliedRequest' = 0
    /\ appliedIdentity' = 0
    /\ UNCHANGED <<identity, nextRequest>>

Next == Open \/ ChangeIdentity \/ Trigger \/ CompleteCurrent
        \/ DropCancelled \/ ApplyCurrent \/ FocusLoss \/ Close

Spec == Init /\ [][Next]_vars

PublishedIsCurrent ==
    admittedRequest = 0 \/
    (open /\ admittedRequest = nextRequest /\ admittedIdentity = identity)

ResultsAreBounded == items <= MaxItems

ClosedOwnsNoCompletion ==
    ~open => (pendingRequest = 0 /\ cancelledRequest = 0 /\
              admittedRequest = 0 /\ items = 0)

ApplyRequiresCurrent ==
    appliedRequest = 0 \/
    (open /\ appliedRequest = nextRequest /\ appliedIdentity = identity)

FaultyPublishCancelled ==
    /\ open
    /\ cancelledRequest # 0
    /\ admittedRequest' = cancelledRequest
    /\ admittedIdentity' = cancelledIdentity
    /\ items' = 1
    /\ UNCHANGED <<open, identity, nextRequest, pendingRequest,
                    pendingIdentity, cancelledRequest, cancelledIdentity,
                    appliedRequest, appliedIdentity>>

FaultyApplyStale ==
    /\ open
    /\ identity > 0
    /\ nextRequest > 0
    /\ appliedRequest' = nextRequest
    /\ appliedIdentity' = identity - 1
    /\ UNCHANGED <<open, identity, nextRequest, pendingRequest,
                    pendingIdentity, cancelledRequest, cancelledIdentity,
                    admittedRequest, admittedIdentity, items>>

FaultyLateSpec == Init /\ [][Next \/ FaultyPublishCancelled]_vars
FaultyApplySpec == Init /\ [][Next \/ FaultyApplyStale]_vars

====
